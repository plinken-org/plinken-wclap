# `wclap-host` (Rust)

**Status:** _planned — design phase. No code yet._

A Rust crate that loads **WCLAP** bundles (CLAP audio plugins compiled
to `wasm32`) inside any Rust audio engine and exposes them through the
[`clack-host`](https://github.com/prokopyl/clack) traits — so a host
treats a wasm plugin exactly like a native CLAP.

This is the native counterpart to the browser host that already ships
in [`apps/wclap-host`](../../apps/wclap-host/) (TypeScript + C++ host
wasm). One artifact, two runtimes.

```
                          .wclap.wasm  /  .wclap.tar.gz
                                 ▲
                                 │ exactly the same artifact
                  ┌──────────────┴──────────────┐
                  │                             │
                  ▼                             ▼
   ┌──────────────────────────┐   ┌──────────────────────────┐
   │  apps/wclap-host         │   │  crates/wclap-host       │
   │  ─────────────────────   │   │  ─────────────────────   │
   │  TS driver + AudioWorklet│   │  Rust + wasmtime + WASI  │
   │  C++ host.wasm (vendored)│   │  clack-host bindings     │
   │  runs at wclap.plinken.org│  │  embeds in any DAW       │
   └──────────────────────────┘   └──────────────────────────┘
```

## Goal

A **full-fidelity WCLAP host in Rust** — community-owned, embeddable,
no DAW lock-in.

End state: anything a CLAP plugin can express, this crate can host.
Audio effects, instruments and samplers, plugin GUIs, MIDI/note event
routing, parameter automation, transport, threading, state save/load —
all of it. A WCLAP bundle authored once should behave identically in
the browser host and in a native engine that pulls in this crate. The
browser host's plumbing (audio chain, plugin GUIs, event routing)
becomes table-stakes; native does everything the browser does plus
things only native can do (lower-latency I/O, tighter realtime
guarantees, OS-integrated file dialogs, optional native windowing for
plugin GUIs).

The destination is upstream-able — `github.com/WebCLAP/wclap-rs` —
not a Plinken-internal tool. Any Rust DAW can `cargo add wclap-host`
and immediately host the same plugins the community publishes through
[plinken.org/shelf.json](https://plinken.org/shelf.json).

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  host process — your Rust DAW / engine                       │
│                                                              │
│   clack_host::*  ── trait impls ──▶  wclap_host::Plugin      │
│                                            │                 │
│                                            ▼                 │
│                                     wasmtime::Instance       │
│                                            │                 │
│                                            ▼                 │
│                                     plugin.wasm  (CLAP)      │
│                                            │                 │
│                                     wasi_snapshot_preview1   │
│                                     + host imports           │
└──────────────────────────────────────────────────────────────┘
```

The crate's job is the middle slab:

1. **Load** a `.wclap.wasm` or `.wclap.tar.gz` bundle (sniff & unpack).
2. **Instantiate** under `wasmtime` with the WASI shim and host-side
   imports defined by [`wclap-bridge`](https://github.com/WebCLAP/wclap-bridge).
3. **Expose** each plugin in the wasm's `clap_plugin_factory` as a
   `clack_host::Plugin` so the engine doesn't know it's wasm.
4. **Forward** the rest of the CLAP API surface (events, params, state,
   GUI) bidirectionally between native host and wasm guest.

Audio processing calls the wasm `clap_plugin->process()` from the
host's audio callback. Memory is shared via a `wasmtime::Memory` view
to avoid per-block copies.

## Design constraints

These apply at every milestone, not just v0.1:

- **No fork of the bundle format.** Whatever the browser host loads,
  this crate must load. Format changes are upstreamed to WebCLAP and
  shipped to both runtimes together.
- **No DAW lock-in.** No dependency on any specific audio engine.
  Anyone can `cargo add wclap-host`. A small example host binary in
  this repo serves as a smoke test and integration reference.
- **Audio-thread safety.** No allocations on the process path after
  activation. The wasm memory is sized at activation time;
  `wasmtime`'s `with_caller` callbacks must be lock-free.
- **Same registry.** Plugins come from
  [plinken.org/shelf.json](https://plinken.org/shelf.json) (or any
  registry that follows the same spec — see
  [`REGISTRY.md`](../../REGISTRY.md)). No parallel catalog.

## Roadmap

The road to the full host, milestone by milestone. Each one ships
something usable on its own; v1.0 is the destination, not a precondition.

| | Title | Delivers |
|---|---|---|
| **M0** | Skeleton | Crate skeleton + `Cargo.toml` in the workspace. _(this README is the only artifact so far.)_ |
| **M1** | First sound | Load a bare `.wclap.wasm` (e.g. `clack-gain`). Instantiate, expose stereo audio I/O. Produces audible output through a tiny CLI example. |
| **M2** | Bundles + factory | `.wclap.tar.gz` unpacking. Multi-plugin enumeration through the CLAP factory. Pick any plugin by id. |
| **M3** | Params + state | Parameter events, value-to-text, state save/load. Round-trip a plugin's state to a buffer and back; result matches the browser host bit-for-bit. |
| **M4** | Instruments + events | CLAP `note-ports`, `note_on`/`note_off`, polyphonic note expressions. Drive a wasm synth (e.g. `clack-polysynth`) from a host-side MIDI stream. Voice management on the host side; the wasm guest receives the same event shape the browser host does. |
| **M5** | Plugin GUIs | Webview integration for the `clap.gui` (webview) path. Likely `wry` so the host gets a window without depending on a specific GUI toolkit. The browser host's iframe semantics map to a native window owned by the DAW. Param sync and visibility events go both directions. |
| **M6** | Threading | wasmtime `threads` feature + SharedArrayBuffer-style memory for plugins that report threaded process support. Matches the browser host's threaded fast path. |
| **M7** | Realtime + transport | Tight audio-thread guarantees, transport events (`clap.transport`), tempo/time-signature changes, sample-accurate automation. Benchmarks vs. native CLAP hosts to prove the wasm overhead is acceptable. |
| **M8** | v1.0 | All of CLAP, on wasm. Publish to crates.io. Propose upstreaming to [`github.com/WebCLAP/wclap-rs`](https://github.com/WebCLAP) so it lives next to `wclap-host-js`. |

Milestones aren't dated — they fall in this order because each one
needs the previous, not because we're committing to a timeline.

## Open questions

These are the parts where the design isn't obvious yet:

- **WASI scope.** WCLAP plugins built with `wclap-cpp` link against
  WASI for fs / time / random. We need to decide which WASI calls are
  safe at audio-thread time vs. only at init. Likely `wasmtime-wasi`
  with a curated import list, plus an audit pass per milestone.
- **Plugin authoring parity.** A plugin that works in the browser
  host today must work here without rebuilding. Any incompat is fixed
  upstream rather than patched in this crate.
- **Multi-instance lifetime.** Browser host loads each plugin in its
  own `AudioWorkletNode`. Native host can pool wasm instances per
  bundle, or instantiate fresh per slot — TBD by memory profile.
- **GUI host policy.** Native hosts vary wildly in window management.
  The crate should default to "embed in a window the host provides"
  rather than spawning top-level windows itself, so a DAW can place
  the plugin GUI inside its own layout.
- **Realtime guarantees.** wasmtime is fast but JIT-compiled.
  Pre-compilation + cwasm caching to get cold-start latency under
  control for live workflows.

## Repo layout (when code lands)

```
crates/wclap-host/
├── Cargo.toml
├── src/
│   ├── lib.rs              # public API
│   ├── bundle.rs           # tar.gz unpack + sniff
│   ├── instance.rs         # wasmtime wiring
│   ├── imports.rs          # host-side fn imports (CLAP host iface)
│   ├── wasi.rs             # curated WASI surface
│   ├── factory.rs          # CLAP plugin factory → clack-host adapter
│   ├── events.rs           # note + param event marshaling (M3+)
│   ├── gui.rs              # webview integration (M5+)
│   └── threads.rs          # threaded process path (M6+)
├── tests/
│   └── load_clack_gain.rs  # smoke test against the live shelf
└── examples/
    └── tiny_host.rs        # CLI: load a .wclap, render N seconds to wav
```

## Related

- [`apps/wclap-host`](../../apps/wclap-host) — the browser counterpart.
- [`plugins/`](../../plugins) + [`REGISTRY.md`](../../REGISTRY.md) —
  the manifest + plugin catalog this crate loads from.
- Upstream: [`WebCLAP`](https://github.com/WebCLAP) (`wclap-cpp`,
  `wclap-host-js`, `wclap-bridge`).
- [`clack`](https://github.com/prokopyl/clack) — Rust CLAP host
  bindings we'll build on top of.
- [`wasmtime`](https://wasmtime.dev) — wasm runtime.
- [`wry`](https://github.com/tauri-apps/wry) — likely webview crate
  for the M5 GUI work.

## License

MIT — inherited from the workspace. See [`LICENSE`](../../LICENSE).
