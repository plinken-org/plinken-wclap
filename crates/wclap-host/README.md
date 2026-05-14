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

## Why this exists

WCLAP unifies plugin distribution: a single bundle that runs in the
browser AND in a native DAW. The browser side is solved (see the live
host at [wclap.plinken.org](https://wclap.plinken.org)). The native
side isn't, and that's the gap this crate closes.

The goal is a **community-owned Rust host** for the WebCLAP ecosystem.
Any Rust audio engine — open-source or commercial — can `cargo add`
this crate and run the same plugins the community publishes via the
open catalog at [plinken.org/shelf.json](https://plinken.org/shelf.json).
We don't want native DAWs forking the work; we want one common path.

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
2. **Instantiate** it under `wasmtime` with the right WASI shim and
   host-side imports defined by [`wclap-bridge`](https://github.com/WebCLAP/wclap-bridge).
3. **Expose** each plugin in the wasm's `clap_plugin_factory` as a
   `clack_host::Plugin` so the rest of the engine doesn't know it's
   wasm.

Audio processing calls the wasm `clap_plugin->process()` from the
host's audio callback. Memory is shared via a `wasmtime::Memory` view
to avoid per-block copies.

## Scope (v0.1)

In:
- Stereo audio effect plugins
- CLAP `audio-ports`, `params`, `state`, `note-ports`, `event-input/output`
- Single-threaded process loop
- WCLAP bundle sniffing (bare wasm OR `tar.gz` with `module.wasm`)
- Per-bundle plugin enumeration via the CLAP factory

Out (initially):
- Plugin GUIs (webview). The browser host renders them in iframes;
  native is harder — either skip and expose params programmatically,
  or embed a webview crate (`wry`, Tauri's `webview`) later.
- Threaded plugins (SharedArrayBuffer path). Needs wasmtime's
  `threads` feature and careful thread-local state.
- SIMD / WASM exception handling. Useful but not on the critical path
  for the first plugin to make sound.

## Design constraints

- **No fork of the bundle format.** Whatever the browser host loads,
  this crate must load. If the format needs to change, both sides
  change together (and the change is upstreamed to WebCLAP).
- **No DAW lock-in.** The crate doesn't depend on any specific audio
  engine. Anyone can `cargo add wclap-host`. A small example host
  binary lives in this repo as a smoke test and integration
  reference.
- **Audio-thread safety.** No allocations on the process path after
  activation. The wasm memory is grown ahead of time; `wasmtime`'s
  `with_caller` callbacks must be lock-free.
- **Same registry.** The first plugins it loads are the ones already
  in [plinken.org/shelf.json](https://plinken.org/shelf.json). No
  parallel catalog.

## Open questions

- **WASI scope.** WCLAP plugins built with `wclap-cpp` link against
  WASI for fs / time / random. We need to decide which WASI calls are
  safe to expose at audio-thread time vs. only at init. Likely uses
  `wasmtime-wasi` with a curated import list.
- **Plugin authoring parity.** A plugin built today with `as-clap` or
  `wclap-cpp` should run unchanged. If we discover incompatibilities,
  we fix them upstream rather than vendoring patches here.
- **Multi-instance lifetime.** Browser host loads each plugin in its
  own `AudioWorkletNode`. Native host can pool wasm instances per
  bundle, or instantiate fresh per slot — TBD based on memory profile.
- **GUI story.** Some plugins are useless without their UI (the
  `example-keyboard` from `signalsmith-clap-cpp`). Native hosts will
  need a webview-or-substitute story before such plugins are useful
  outside the browser.

## Milestones

| | |
|---|---|
| **M0** | Crate skeleton + `Cargo.toml` published in workspace. _(this README is the only artifact so far)_ |
| **M1** | Load a bare-wasm plugin (`clack-gain`), instantiate, expose audio I/O. Smoke test produces audible output through a tiny example host binary. |
| **M2** | `.wclap.tar.gz` bundle unpacking. Loading multi-plugin bundles, plugin enumeration. |
| **M3** | Param events + state save/load round-trip. Match the browser host's behavior on the same plugin/state. |
| **M4** | Stable v0.1 published to crates.io. Hosts can `cargo add wclap-host`. |
| **M5** | Upstream the crate to `github.com/WebCLAP` as `wclap-rs` (if the WebCLAP maintainers want it there). |

## Repo layout (when code lands)

```
crates/wclap-host/
├── Cargo.toml
├── src/
│   ├── lib.rs              # public API
│   ├── bundle.rs           # tar.gz unpack + sniff
│   ├── instance.rs         # wasmtime wiring
│   ├── imports.rs          # host-side fn imports (CLAP host iface)
│   ├── wasi.rs             # WASI surface (curated)
│   └── factory.rs          # CLAP plugin factory → clack-host adapter
├── tests/
│   └── load_clack_gain.rs  # smoke test against the live shelf
└── examples/
    └── tiny_host.rs        # CLI: load a .wclap, render N seconds of silence to wav
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

## License

MIT — inherited from the workspace. See [`LICENSE`](../../LICENSE).
