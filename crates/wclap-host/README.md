# `wclap-host` (Rust)

**Status:** _design phase. The existing `src/` is a `wasmtime`-based sketch from before the scope was pinned to web-only — it will be replaced in M1 and should be treated as a dead branch until then._

A Rust port of the upstream [`wclap-cpp`](https://github.com/WebCLAP/wclap-cpp) + [`wclap-js-instance`](https://github.com/WebCLAP/wclap-host-js) host. Compiles to `wasm32-unknown-unknown` as a `cdylib` and produces a drop-in replacement for the C++ `host.wasm` currently used by [`apps/wclap-host`](../../apps/wclap-host/) — and embeddable inside other Rust + `wasm-bindgen` audio engines (Plinken's mixer being the primary consumer).

> **WCLAP is the web's CLAP.** Native DAWs already host CLAP through the C ABI; compiling CLAP plugins to wasm only matters in sandboxed contexts that can't link native code — browsers first, anything wasm-based after. This crate does in Rust what the upstream C++ host already does in those environments, and nothing more.

## Why a Rust port?

`plinken-app-wasm/audio` is a Rust + `wasm-bindgen` `cdylib`. It needs to host WCLAP plugins on each mixer channel. Bridging a foreign-language host (`host.wasm` built from C++) into a Rust audio engine adds an extra JS hop on every cross-module call and duplicates allocator + WASI surfaces. A Rust crate compiled *into* the same wasm module removes the hop and lets the host live in the same address space the engine already manages.

For the standalone case ([`apps/wclap-host`](../../apps/wclap-host/)), the crate also builds as its own `cdylib` that drops into `apps/wclap-host/src/wclap-runtime/host.wasm`, replacing the C++ artifact without touching any of the surrounding JS glue ([`wclap-host-js`](../../vendor/wclap-host-js/), `clap-audionode.mjs`, the worklet).

## Two consumption models

```
                Rust crate: wclap-host
                        │
            ┌───────────┴───────────┐
            ▼                       ▼
   built as a cdylib →      compiled inside a larger
   standalone host.wasm     wasm-bindgen cdylib
   (drops into              (plinken-app-wasm/audio
   apps/wclap-host/)         per-channel hosting)
            │                       │
            ▼                       ▼
    loaded by wclap-host-js   linked into the audio
    in the example app         engine; no extra JS hop
```

Both paths build with the same `cargo` invocation. The crate declares the wasm imports the upstream C++ host declares (`_wclapInstance.*`, `env.*`, WASI); whatever wraps it on the JS side — `wclap-host-js` for the example, Plinken's own glue for plinken-app — provides them.

## Wasm boundary (what the JS glue sees)

These are not arbitrary choices — they're the upstream `wclap-cpp` / `wclap-js-instance` contract. We mirror them so the existing JS host (`wclap-host-js`) drives our `host.wasm` unchanged.

### Imports we declare

- `_wclapInstance.{init32, malloc32, memcpyToOther32, memcpyFromOther32, call32, registerHost32, countUntil32, runThread, release}` (+ `*64` variants). JS implements these — we never touch a plugin's `Memory` or `Table` directly; we ask JS to copy bytes or invoke a function index, identified by an opaque `handle`.
- `env.{log, paramsRescan, stateMarkDirty, webviewSend, eventsOutTryPush}` — host-side callbacks the embedder wires up.
- `wasi_snapshot_preview1.*` + `wasi.thread-spawn` — supplied by `wclap-host-js`'s WASI shim, or, when embedded in a larger `wasm-bindgen` module, by that module's existing surface.

The authoritative source for the import shapes is [`vendor/wclap-host-js/cpp/wclap-js-instance.h`](../../vendor/wclap-host-js/cpp/wclap-js-instance.h).

### Exports we provide (the JS-facing API)

- **Bytes channel**: `createBytes`, `resizeBytes`, `getBytesData`, `getBytesLength`. CBOR-encoded payloads cross the JS/wasm boundary through this channel.
- **Lifecycle**: `makeHosted`, `createPlugin`, `pluginStart`, `pluginMainThread`.
- **Audio**: `pluginProcess`.
- **Info**: `pluginGetInfo`, `getInfo`.
- **Params**: `pluginSetParam`, `pluginGetParam`, `pluginGetParams`, `pluginParamsFlush`.
- **State**: `pluginSaveState`, `pluginLoadState`.
- **Events / UI**: `pluginAcceptEvent`, `pluginMessage`, `pluginGetResource`.

Confirm against [`apps/wclap-host/src/wclap-runtime/clap-audioworkletprocessor.mjs`](../../apps/wclap-host/src/wclap-runtime/clap-audioworkletprocessor.mjs) — that file is the unfiltered list of `hostApi.*` calls our `host.wasm` must answer.

## What's different from the C++ host

- **Smaller wasm.** The C++ build is ~3.1 MB (wasi-libc + libc++ + templates). A `no_std + alloc` Rust port should land in a few hundred KB after `wasm-opt -Oz`.
- **Typed CLAP structs.** Field offsets the C++ side computes through templated `Pointer<T>` / `Function<R, Args…>` become `#[repr(C)]` Rust structs — cuts a class of bugs that only surfaces when a plugin happens to dereference a field nobody walked yet.
- **Linkable as a Rust crate** inside another Rust wasm module. The C++ host is only consumable as a separate `WebAssembly.Instance`.

## Architecture

```
+---------------------------------------------+
|  apps/wclap-host page  /  plinken-app       |
+----------------------┬----------------------+
                       │  JS glue (wclap-host-js):
                       │   compile + instantiate
                       │   host.wasm and plugin.wasm,
                       │   wire _wclapInstance.* and WASI
+----------------------▼----------------------+
|  host.wasm  ← built from this crate         |
|   - declares _wclapInstance.* imports       |
|   - exports the JS API above                |
|   - walks clap_entry / factory / plugin     |
+----------------------┬----------------------+
                       │  via _wclapInstance.call32 /
                       │      memcpyToOther32 /
                       │      memcpyFromOther32 / …
+----------------------▼----------------------+
|  plugin.wasm  (clack-gain, signalsmith, …)  |
|   - separate WebAssembly.Instance           |
|   - JS routes cross-instance ops on its     |
|     Memory + Table via the bridge above     |
+---------------------------------------------+
```

The host wasm never holds a direct reference to a plugin's `Memory` or `Table` — both `WebAssembly.Instance`s sit in JS, and all cross-module operations are mediated by the `_wclapInstance` bridge.

## Roadmap

| | Title | Delivers |
|---|---|---|
| **M0** | Skeleton | `Cargo.toml` set to `wasm32-unknown-unknown` cdylib; import declarations match the C++ host; empty Rust stubs for every export. Standalone `host.wasm` loads in `apps/wclap-host` (and does nothing). |
| **M1** | First sound | Replace the C++ `host.wasm` in the example app with our Rust build and route the page's 440 Hz tone through `clack-gain`. Implements: bytes channel, `makeHosted`, `createPlugin`, `pluginStart`, `pluginProcess`, `pluginGetInfo`. |
| **M2** | Bundles | `.wclap.tar.gz` decode + multi-plugin factory enumeration. Match what `apps/wclap-host` already shows when loading Signalsmith Basics through the C++ host. |
| **M3** | Params + state | `pluginSetParam`, `pluginGetParam(s)`, `pluginParamsFlush`, `pluginSaveState`, `pluginLoadState`. State round-trip must match the C++ host bit-for-bit. |
| **M4** | Events | `pluginAcceptEvent`, `eventsOutTryPush`. Drive a wasm synth (`clack-polysynth`) from MIDI. |
| **M5** | Plinken embedding | Link the crate into `plinken-app-wasm/audio` and host plugins per mixer channel. |
| **M6** | GUI | `pluginMessage`, `webviewSend`, `pluginGetResource` — the CLAP webview path. |
| **M7** | Threading | `runThread` / `wasi.thread-spawn`; matches the C++ host's threaded fast path. |
| **M8** | Upstream | Propose under `github.com/WebCLAP/wclap-rs` so it lives alongside `wclap-host-js`. |

The C++ host stays the fallback in `apps/wclap-host` until parity is reached.

## Build (planned)

```sh
cargo build --target wasm32-unknown-unknown --release
wasm-opt -Oz -o host.wasm \
  target/wasm32-unknown-unknown/release/wclap_host.wasm
cp host.wasm ../../apps/wclap-host/src/wclap-runtime/host.wasm
pnpm --filter @plinken/wclap-host dev
```

`wasm-opt` is part of [`binaryen`](https://github.com/WebAssembly/binaryen).

## Open questions

- **`no_std` vs. `std`.** `no_std + alloc` keeps the wasm small but loses `println!` and friends. A slim `std` build is fine on `wasm32-unknown-unknown` for most things but compiles in unused panic infra. Decide in M0.
- **CBOR vs. simpler wire format.** The C++ host uses CBOR over the bytes channel; the JS side decodes via [`cbor.mjs`](../../apps/wclap-host/src/wclap-runtime/cbor.mjs). Keeping CBOR preserves protocol compatibility with `wclap-host-js`. Adds a Rust dep (`minicbor` is small and `no_std`).
- **Allocator.** Default `dlmalloc` is probably fine; revisit if profiling shows fragmentation in long sessions.

## Related

- [`apps/wclap-host`](../../apps/wclap-host) — the example app, consumes `host.wasm`.
- [`vendor/wclap-host-js/cpp/`](../../vendor/wclap-host-js/cpp/) — the C++ source we're porting from. `wclap-cpp/include/wclap/_impl/wclap-generic.hpp` is the generated CLAP-as-WCLAP header; [`wclap-js-instance.h`](../../vendor/wclap-host-js/cpp/wclap-js-instance.h) defines the `_wclapInstance.*` imports we mirror.
- [`vendor/wclap-host-js/es6/wclap.mjs`](../../vendor/wclap-host-js/es6/wclap.mjs) — the JS glue that wraps our `host.wasm`.
- Upstream: [`github.com/WebCLAP`](https://github.com/WebCLAP).

## License

MIT — inherited from the workspace. See [`LICENSE`](../../LICENSE).
