# Plinken vocal-host — V2 low-latency host design

A second host app, alongside `apps/wclap-host`, dedicated to Plinken's
vocal-tool chain (limiter → compressor → EQ). The existing host stays as
the generic test/dev sandbox; this one is the **production vocal app**,
opinionated and optimised for the lowest plugin-chain latency Web Audio
can deliver.

Saved 2026-05-15 at end of a long conversation that produced the V1 host
under `apps/wclap-host`. Read that path's `CLAUDE.md` ("Rust WCLAP plugin
gotchas" + "How a plugin UI iframe is routed") before working here — V2
inherits every plugin-side mechanism intact.

## Goal

Run a fixed-shape vocal plugin chain in the browser with the **smallest
possible end-to-end latency**. Trained musicians hear single-digit ms;
we can't beat the ~30–50 ms macOS+Chrome system audio baseline from inside
a browser, but we can stop *adding to it* per plugin.

| | Baseline (system) | Per plugin (chain) | Total, 3-plugin chain |
|---|---|---|---|
| V1 host (`apps/wclap-host`) | ~30–50 ms | +3 ms each | ~40–60 ms |
| V2 host (`apps/vocal-host`) | ~30–50 ms | **+0 ms after first** | ~32–52 ms |

That's parity with Logic's low-latency mode for the plugin-chain
portion. We give up ~5 ms of macOS+browser system audio overhead to
native hosts; can't fix that without leaving the browser.

## Why V2 — what's wrong with V1

V1 follows the upstream `wclap-host-js` convention: **one
`AudioWorkletNode` per plugin slot**. Every node in a Web Audio graph
adds one render quantum (~2.67 ms @ 48 kHz). For a 3-plugin vocal chain
that's ~8 ms of *additional* latency on top of the system baseline —
audible to a trained ear, and 100% unnecessary because the DSP itself
(peak limiter, envelope-follower compressor, biquad EQ) has zero inherent
latency.

The fix is well-trodden in native hosts (Logic, Reaper, Bitwig): one
audio callback runs every plugin in the chain in sequence within the
same buffer. We can do the equivalent in a single `AudioWorkletNode`
that owns all plugins internally — that's V2.

## Architecture

### V1 (current)

```
oscillator/mic → inGain
    → AudioWorkletNode #1 (limiter)
        ├─ wclap-host.wasm
        └─ vocal-limiter.wclap.wasm
    → AudioWorkletNode #2 (compressor)
        ├─ wclap-host.wasm     (separate copy)
        └─ vocal-compressor.wclap.wasm
    → AudioWorkletNode #3 (EQ)
        ├─ wclap-host.wasm     (separate copy)
        └─ vocal-eq.wclap.wasm
    → splitter+meters
    → outGain → destination
```

Per-AWN cost: 1 render quantum × N nodes.

### V2 (target)

```
oscillator/mic → inGain
    → AudioWorkletNode (the chain)
        ├─ chain-worklet.mjs
        ├─ wclap-host.wasm   (ONE instance, owns N plugin handles)
        ├─ plugin handle 0 → vocal-limiter.wclap.wasm
        ├─ plugin handle 1 → vocal-compressor.wclap.wasm
        └─ plugin handle 2 → vocal-eq.wclap.wasm
    → splitter+meters
    → outGain → destination
```

Per-AWN cost: 1 render quantum **total**, independent of N.

Audio flow inside `chain-worklet.process()`:

```js
process(inputs, outputs) {
    let buf = inputs[0];                              // mic / source
    for (const handle of this.chainHandles) {
        // pluginProcess uses the host's pre-allocated buffer slots
        // hostApi.audioInputs[0]  = current buf (filled in step above)
        // hostApi.audioOutputs[0] = scratch
        this.hostApi.pluginProcess(handle, blockLength);
        buf = this.hostApi.audioOutputs[0];           // ping-pong
        [this.hostApi.audioInputs[0], this.hostApi.audioOutputs[0]] =
            [buf, this.hostApi.audioInputs[0]];
    }
    outputs[0].set(buf);                              // final write to JS
}
```

Two JS↔wasm copies for the whole block — once into the wasm host's input
buffer, once out of its final output — vs. V1's two copies per plugin.

## What changes & what doesn't

### Unchanged (everything plugin-side)

- All plugin tarballs (`vocal-limiter`, future compressor, EQ, plus the
  existing `auto-pan` and `synome`) work as-is. The wasm doesn't know
  whether the host wraps it in one AWN per plugin or one AWN for the
  whole chain.
- `crates/wclap-plugin` shared Rust scaffold — unchanged. The Plugin
  trait, params extension, webview extension, latency extension all keep
  working.
- All widgets (`widgets/fader.mjs`, `pot.mjs`, `meter.mjs`, etc.) —
  unchanged. Each plugin's iframe UI runs the same way.
- `crates/wclap-host` Rust host — **unchanged**. It already supports
  multi-plugin operation per host instance: `createPlugin(...)` returns
  a handle, `pluginProcess(handle, ...)` is per-handle, internal slot
  map tracks them. We just stop spinning up a fresh host wasm per AWN.
- CLAP extensions (`clap.params`, `clap.webview/3`, `clap.latency`) work
  unchanged across the architecture change — they're CLAP-level, the
  audio routing change is below them.

### Changed (host-side JS)

- New top-level worklet: `apps/vocal-host/src/chain-worklet.mjs`. Owns
  the Rust host wasm, manages a chain array of plugin handles, drives
  audio in one `process()` call.
- New main-thread orchestrator: `apps/vocal-host/src/main.ts`. Page wiring,
  shelf, mic/tone source, panel/iframe lifecycle, message bridging
  between iframe UIs and the chain worklet.
- New shelf: filtered to Plinken vocal plugins only. Drag-drop into
  fixed slots (Limiter / Compressor / EQ in known signal-flow order).
- New UI shell: not a generic "drop any plugin here" rack, but a tidy
  vocal-chain panel. The plugin iframe UIs still load as today.

## Message protocol (main thread ↔ chain worklet)

Worklet exposes a single `MessagePort` for both control and audio-thread
messages. Main thread → worklet:

| Message | Payload | Effect |
|---|---|---|
| `load` | `{ slot, module, files, manifest }` | Worklet instantiates the wasm into the Rust host, registers it at the slot index. Audio thread sees the new chain on the next `process()` (atomic pointer flip). |
| `unload` | `{ slot }` | Removes the plugin from the chain, frees its handle. |
| `set` | ArrayBuffer (CBOR `{set:[id,value]}`) | Forwarded to plugin's `webview.receive`. |
| `iframe-mounted` | `{ slot }` | UI iframe has loaded; worklet sends a current-state snapshot back. |

Worklet → main thread:

| Message | Payload | When |
|---|---|---|
| ArrayBuffer | CBOR `{params:{...}}` | Plugin called `host_webview.send` — route to the right iframe based on slot tracking. |
| `loaded` | `{ slot, success, error? }` | Reply to a `load`. |
| `crashed` | `{ slot, error }` | Plugin or host wasm trapped. |

The main thread is the bridge between iframes and the worklet. Iframes
still use the `widgets/transport.mjs` postMessage convention; main
thread routes those to the appropriate `set` worklet messages by slot
ID, and forwards worklet→iframe param snapshots by reading the slot
from a tracking map (each iframe is associated with a slot at load
time).

## Plugin lifecycle in V2

Loading a plugin:

1. **Main thread**: fetch tarball → expand → `WebAssembly.compile(module.wasm)` → collect the files map (the same `{path: ArrayBuffer}` shape `wclap-host-js` uses).
2. **Main thread**: `worklet.port.postMessage({ kind: 'load', slot, module, files, manifest })`.
3. **Worklet**: receive message → instantiate plugin wasm in the same audio context → call `_initialize` → walk CLAP entry → register with Rust host via `hostApi.createPlugin(plugin_id, host_struct)` → host returns a handle → push handle into `chainHandles` at the requested slot.
4. **Worklet**: respond with `loaded` so main thread can create the iframe for this slot.
5. **Audio thread**: next `process()` reads the updated `chainHandles` array (single-writer = the worklet's JS thread, single-reader = its own audio function; atomic via standard JS memory model in workers).

Unloading:

1. **Main thread**: `worklet.port.postMessage({ kind: 'unload', slot })`.
2. **Worklet**: splice the handle out of `chainHandles` → call `hostApi.destroyPlugin(handle)` → drop wasm instance.
3. **Audio thread**: next quantum sees the smaller chain.

Hot-swap (replace plugin at slot N): unload then load. Tiny gap of one
quantum where slot N is skipped, no glitch.

## Implementation outline (file-by-file)

```
apps/vocal-host/
├── DESIGN.md                       (this file)
├── package.json                    @plinken/vocal-host, builds with Vite
├── index.html                      Page shell — source picker, vocal chain UI, panel iframe slots
├── vite.config.ts                  Mirror of apps/wclap-host's config, port 5175
├── public/
│   └── plugin-proxy-sw.js          Copy of host-side service worker (iframes load via /plugin-proxy/)
└── src/
    ├── main.ts                     Page wiring, source switching, shelf filtered to vocal plugins
    ├── chain-worklet.mjs           THE worklet — one AudioWorkletNode for the whole chain
    ├── plugin-loader.ts            Tarball fetch + expand + compile, hands to worklet via postMessage
    ├── iframe-bridge.ts            Routes iframe ↔ worklet via main thread, per slot
    ├── ui/                         Page-level UI (source picker, slot panels) — not the plugin UIs
    └── styles.css
```

The plugin iframes themselves live unchanged inside `.wclap.tar.gz` files
and load via the proxy SW exactly as in V1.

## Effort estimate

- **Design + scaffolding** (this doc, app skeleton, vite config, page shell): half a session.
- **chain-worklet.mjs core** (audio loop, multi-plugin host wiring, message handlers): one focused session.
- **plugin-loader + dynamic load/unload**: half a session.
- **iframe-bridge + main thread UI**: half a session.
- **First audible end-to-end** (load 1 plugin, hear audio through it, see UI work): cumulative ~2 sessions.
- **Polish + 3-plugin chain working with current vocal-limiter**: one more session.

Call it **3–4 focused sessions** to first usable product. Most of the
plugin-side work is already done — the wasm side doesn't move.

## Open decisions (settle before/during V1 → V2 cutover)

1. **App naming.** `apps/vocal-host`? `apps/plinken-vocals`? `apps/vocal-suite`? User preference.
2. **Fixed slots vs flexible.** ~~Should the chain be hard-coded
   "Limiter → Compressor → EQ", or user-orderable with constraints?~~
   **Decided 2026-05-15: 5 slots, user-fillable, empty = pass-through.**
   Mirrors V1's rack UX exactly. Slot index 0..4 maps to chain order
   top→bottom. The worklet's `chainHandles` array is length 5 with
   `null` for empty slots; `process()` skips nulls.
3. **Source pre-chain.** V2 inherits the mic + tone + channel-router work
   from V1 essentially unchanged — copy `apps/wclap-host/src/main.ts`'s
   source section and the channel router, plus the `Tone/Mic` toggle.
4. **Master gain & meters.** V1's outGain + analyser meters work fine;
   reuse the pattern.
5. **Bypass per plugin.** Real vocal hosts have per-plugin bypass. Worth
   adding to the chain worklet's slot model from the start (a `bypass`
   flag per slot, worklet skips that plugin in the loop).
6. **Plugin order discovery.** When a user drops vocal-eq into a slot,
   does the host enforce the canonical signal flow (HPF→drive→shelves→
   limiter etc.)? Probably yes — re-order automatically based on the
   plugin's manifest `category` or a Plinken-specific field.
7. **Replay-while-load.** Loading a plugin shouldn't drop audio. Verify
   the worklet's chain mutation is glitch-free across the quantum
   boundary; should be, given JS's single-threaded execution model
   inside the worklet.

## V1 retirement

**Decided 2026-05-15:** once V2 works end-to-end (audio + chain + UI
through ≥1 plugin), V1 (`apps/wclap-host`) is retired. V2 becomes the
sole host — there is no parallel maintenance plan. The "5-slot,
pass-through if empty" model means V2 covers V1's generic-rack use
case too; we don't lose the ability to A/B random WCLAPs.

Both share:
- All plugin tarballs (under `plugins/com.plinken/`)
- The widget library (`widgets/`)
- The Rust scaffold (`crates/wclap-plugin`)
- The Rust host wasm (`crates/wclap-host`)
- The bundle pipeline (`scripts/bundle-wclap.mjs`, `scripts/build-shelf.mjs`)

## Suggested sequencing for next session

Before any V2 code:

1. **Verify the V1 work that's currently uncertain.** The most recent
   change wired the limiter's plugin→UI push (peak + GR meters) via
   `clap_host_webview.send`. User had not yet confirmed it works at
   conversation-end. **First action next session: reload V1 host, drop
   limiter, run a hot signal, verify peak/GR meters move.** If they
   don't, debug that path first — it's the foundation for any
   meter-bearing plugin in V2 too.
2. **Decide app name** (see open decisions).
3. **Scaffold `apps/vocal-host/`** following the file layout above.
4. **Write `chain-worklet.mjs`** — start with single-plugin, prove
   audio flows + a plugin loads + UI works through the new message
   protocol. Then extend to N plugins in the chain.

## Reference points in the existing codebase

- `apps/wclap-host/src/wclap-runtime/clap-audioworkletprocessor.mjs` —
  the per-plugin AWP we're replacing. Read `process()` (~line 322), the
  port message handlers (~line 173), and `setParam` (~line 264).
- `apps/wclap-host/src/wclap-runtime/clap-audionode.mjs` — main-thread
  iframe bridge. The `messageHandler` (~line 201) routes iframe → AWP
  ArrayBuffer messages; line 174-180 routes AWP → iframe.
- `crates/wclap-host/src/plugin.rs:1124` (`pluginMessage`) — the path
  from JS bytes to plugin's `webview.receive`. Reusable as-is.
- `crates/wclap-host/src/host_stubs.rs:242` (`_wclap_host_webview_send`)
  — plugin → iframe push side. Also reusable.

## Closing note

V2 is the right architecture *for Plinken*. The V1 host correctly
inherits an architecture that's right for "load any WCLAP plugin and
hear it work" — both should exist. Don't delete V1.
