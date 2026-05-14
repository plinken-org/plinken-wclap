# `wclap-host` implementation plan

Living spine doc for the crate. Decisions here are binding until something measurable contradicts them. Per-milestone plans live alongside (`m1-first-sound.md`, etc.) and are written when that milestone is the next thing to ship.

**Scope:** Rust port of [`wclap-cpp`](https://github.com/WebCLAP/wclap-cpp) + [`wclap-js-instance`](https://github.com/WebCLAP/wclap-host-js). Web-only (`wasm32-unknown-unknown` cdylib). For the reasoning, see [`../README.md`](../README.md).

## 1. Tech stack

| Dependency | Use | Why |
|---|---|---|
| `wasm-bindgen` *(optional)* | JS-facing exports | If the export surface needs JS-flavoured marshaling (`String`, `JsValue`, etc.). For the raw `extern "C"` API the C++ host exposes, plain `#[no_mangle]` is enough — we may skip `wasm-bindgen` entirely. |
| `minicbor` | bytes-channel encoding | `no_std`-friendly, ~10 KB, matches the format `cbor.mjs` already decodes on the JS side. |
| `thiserror` | typed errors | Standard. |
| `tracing` | logging | Routed to the `env.log` host import. Compile-time disabled in release if it bloats wasm. |

**Not** going to use:

- `wasmtime` / `wasmer` / `wasmi` — we are not embedding a runtime. JS owns plugin instantiation and cross-instance ops.
- `clack-host` — its `Plugin` trait is shaped around a native host. We expose our own surface that matches what `wclap-host-js` expects.
- `anyhow` in library code — only in tests.

Build target is `wasm32-unknown-unknown` with `crate-type = ["cdylib"]`. Default Rust allocator (`dlmalloc`) for now. `no_std + alloc` is preferred for size; allow `std` if it makes development hurt (decide in M0).

Releases pass through `wasm-opt -Oz` to shave dead code and tighten the artifact.

## 2. Module layout

```
src/
├── lib.rs           # re-exports, panic hook, #[no_mangle] surface
├── bytes.rs         # createBytes / resizeBytes / getBytesData / getBytesLength
├── cbor.rs          # CBOR encode/decode helpers over the bytes channel
├── instance.rs      # _wclapInstance.* extern declarations + safe wrappers
├── host.rs          # in-memory clap_host struct + host stub indices
├── factory.rs       # walk clap_entry → clap_plugin_factory
├── plugin.rs        # Plugin: tracks clap_plugin_t + process scaffolding
├── process.rs       # build clap_process_t, call plugin.process indirectly
├── events.rs        # CLAP event marshaling                       (M4+)
├── state.rs         # save / load over the bytes channel          (M3+)
├── gui.rs           # webview bridge                              (M6+)
├── threads.rs       # runThread / wasi.thread-spawn               (M7+)
└── error.rs         # Error enum (thiserror)
```

`gui.rs` and `threads.rs` exist as one-line placeholders at M0 with a `// MILESTONE: M6 / M7` comment. Their absence isn't a gap, it's a deferral.

## 3. Wasm boundary

This is the part the C++ host defines and we have to mirror exactly. The JS glue ([`wclap-host-js`](../../../vendor/wclap-host-js/)) knows the names and types; deviating breaks the contract.

### 3.1 Imports

From **`_wclapInstance`** (defined in [`wclap-js-instance.h`](../../../vendor/wclap-host-js/cpp/wclap-js-instance.h)):

| Name | Signature (wasm32) | Purpose |
|---|---|---|
| `init32` | `(handle: i32) -> i32` | One-time init for a plugin instance; returns the wasm-side handle pointer. |
| `malloc32` | `(handle: i32, size: i32) -> i32` | Alloc bytes inside the plugin's memory. |
| `memcpyToOther32` | `(handle: i32, destP: i32, src: i32, count: i32) -> i32 (bool)` | Copy `src` from *our* memory → `destP` in *plugin* memory. |
| `memcpyFromOther32` | `(handle: i32, dest: i32, srcP: i32, count: i32) -> i32 (bool)` | Copy `srcP` in plugin memory → `dest` in our memory. |
| `call32` | `(handle: i32, wasmFn: i32, isPtrToFn: i32, resultPtr: i32, argsPtr: i32, argsCount: i32) -> i32 (bool)` | Invoke a function index in the plugin's table. Args + return marshaled via `TaggedValue`. |
| `registerHost32` | `(handle: i32, ctx: i32, fn: i32, sig: i32, sigLen: i32) -> i32` | Register a *our*-side function as a callable in the plugin's table (host-stub installation). |
| `countUntil32` | `(handle: i32, startP: i32, until: i32, itemSize: i32, maxCount: i32) -> i32` | Linear scan in plugin memory looking for a sentinel — used to find C-string ends, etc. |
| `runThread` | `(handle: i32, threadId: i32, startArg: i64)` | Re-enter the plugin on a worker thread (M7). |
| `release` | `(handle: i32)` | Plugin instance is being dropped; tear down host-side state. |

`*64` variants exist for wasm64; we declare them but won't exercise them at v1 (the plugins on the shelf are all wasm32).

From **`env`**:

| Name | Signature | Purpose |
|---|---|---|
| `log` | `(pluginPtr: i32, severity: i32, msgPtr: i32, len: i32)` | Plugin-side `clap.log` callback. Embedder routes to console / file. |
| `paramsRescan` | `(pluginPtr: i32, flags: i32)` | Plugin reports parameter list changed. |
| `stateMarkDirty` | `(pluginPtr: i32)` | Plugin's state is dirty (host should save). |
| `webviewSend` | `(pluginPtr: i32, ptr: i32, len: i32)` | Plugin → webview message. |
| `eventsOutTryPush` | `(pluginPtr: i32, ptr: i32, len: i32) -> i32 (bool)` | Plugin emits an output event (CBOR-encoded). |

From **`wasi_snapshot_preview1`**: the subset Rust pulls in once a build uses `std`. See [`wasi-surface.md`](wasi-surface.md). We do not relay WASI to plugins — JS supplies WASI to each plugin instance directly.

### 3.2 Exports

Names and shapes come from what [`apps/wclap-host/src/wclap-runtime/clap-audioworkletprocessor.mjs`](../../../apps/wclap-host/src/wclap-runtime/clap-audioworkletprocessor.mjs) calls. M1 needs the starred ones; the rest land in later milestones.

| Export | Signature | Milestone |
|---|---|---|
| `createBytes` ★ | `() -> i32` | M1 |
| `resizeBytes` ★ | `(handle: i32, len: i32) -> i32 (data ptr)` | M1 |
| `getBytesData` ★ | `(handle: i32) -> i32` | M1 |
| `getBytesLength` ★ | `(handle: i32) -> i32` | M1 |
| `makeHosted` ★ | `(wclapInstancePtr: i32) -> i32 (hostedPtr)` | M1 |
| `getInfo` | `(hostedPtr: i32, bytes: i32) -> i32` | M2 |
| `createPlugin` ★ | `(hostedPtr: i32, pluginIdBytes: i32) -> i32 (pluginPtr)` | M1 |
| `pluginGetInfo` ★ | `(pluginPtr: i32, bytes: i32) -> i32` | M1 |
| `pluginStart` ★ | `(pluginPtr: i32, sampleRate: f64, ?, maxFrames: i32, bytes: i32) -> i32` | M1 |
| `pluginProcess` ★ | `(pluginPtr: i32, blockLen: i32) -> i32 (status)` | M1 |
| `pluginMainThread` | `(pluginPtr: i32)` | M1 (no-op stub) |
| `pluginSetParam` | `(pluginPtr: i32, paramId: i32, value: f64)` | M3 |
| `pluginGetParam` | `(pluginPtr: i32, paramId: i32, bytes: i32) -> i32` | M3 |
| `pluginGetParams` | `(pluginPtr: i32, bytes: i32) -> i32` | M3 |
| `pluginParamsFlush` | `(pluginPtr: i32)` | M3 |
| `pluginSaveState` | `(pluginPtr: i32, bytes: i32) -> i32 (bool)` | M3 |
| `pluginLoadState` | `(pluginPtr: i32, bytes: i32) -> i32 (bool)` | M3 |
| `pluginAcceptEvent` | `(pluginPtr: i32, bytes: i32)` | M4 |
| `pluginMessage` | `(pluginPtr: i32, bytes: i32)` | M6 |
| `pluginGetResource` | `(pluginPtr: i32, path: i32) -> i32` | M6 |

`pluginStart`'s third arg is `min_frames_count` (`0` per the existing JS call site). Confirm the exact name in C++ when porting.

### 3.3 Marshaling

- **Strings, structs**: through the *bytes channel*. The host owns a small set of `Bytes` records, JS reads/writes them. CBOR is the encoding for structured payloads (descriptors, params, audio-port maps).
- **Function calls into plugin memory**: through `_wclapInstance.call32`. Args are written into a `TaggedValue[]` block in *our* memory, JS unpacks them, marshals into the plugin's `call_indirect`.
- **Audio**: per-channel float pointers are returned by `pluginStart` (CBOR map `{inputs: [[ptr,...], …], outputs: [[ptr,...], …]}`). JS writes input samples directly into the plugin's `Memory` at those pointers before each `pluginProcess`, reads outputs back after.

## 4. Threading model

- **Main thread**: `makeHosted`, `createPlugin`, `pluginStart`, `pluginSaveState/LoadState`, GUI ops. Allowed to allocate.
- **Audio worklet**: `pluginProcess` and `pluginMainThread` only. No allocation, no syscalls. The audio loop reads/writes pre-allocated buffers via JS `Float32Array` views into the plugin's memory.
- **Plugin worker threads**: M7. `runThread` re-enters on a worker spawned by `wasi.thread-spawn` and routed through JS's `Worker` infrastructure (mirrors what the C++ host does).

The crate itself is single-threaded by design — there is no shared mutable state across the boundary. Per-plugin state lives behind the `pluginPtr` returned to JS, which JS holds and passes back; the wasm side treats it as a typed handle.

## 5. Error handling

```rust
// src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid bundle: {0}")]
    Bundle(String),

    #[error("CLAP ABI: {0}")]
    ClapAbi(String),

    #[error("instance bridge: {0}")]
    Instance(String),

    #[error("bytes channel: {0}")]
    Bytes(String),

    #[error("CBOR: {0}")]
    Cbor(String),

    #[error("WASI denied: {0}")]
    WasiDenied(String),
}

pub type Result<T> = core::result::Result<T, Error>;
```

Errors surface to JS as: `false` from boolean exports, `0` from pointer-returning exports, or a CBOR-encoded `{error: "…"}` payload in the bytes channel for richer messages. Panics are routed through `std::panic::set_hook` to `env.log` with severity `error` — the C++ host just `abort()`s, we can do better.

## 6. Testing

The corpus lives in [`apps/wclap-host/public/samples/`](../../../apps/wclap-host/public/samples/) and spans three toolchains plus two bundle formats:

| Plugin | Toolchain | Format | First exercised at |
|---|---|---|---|
| `clack-gain.wasm` | Rust ([`clack`](https://github.com/prokopyl/clack)) | bare `.wasm` | M1 (effect, unity default) |
| `as-clap-example.wclap.wasm` | AssemblyScript ([`as-clap`](https://github.com/WebCLAP/as-clap)) | bare `.wasm` | M1 (different toolchain than clack) |
| `com.plinken.auto-pan.wclap.wasm` | Plinken's own | bare `.wasm` | M1 (audible motion, useful smoke test) |
| `clack-polysynth.wasm` | Rust (clack) | bare `.wasm` | M4 (note events) |
| `signalsmith-basics.wclap.tar.gz` | C++ (`wclap-cpp`) | `.wclap.tar.gz`, multi-plugin | M2 (factory enumeration + tar.gz) |
| `signalsmith-clap-cpp.wclap.tar.gz` | C++ (`wclap-cpp`) | `.wclap.tar.gz`, multi-plugin | M2 |

The example app is the integration harness. M1 verification is "open the page, drop a plugin onto it, hear the 440 Hz test tone through it" — the same workflow used to validate the C++ host. We test manually until the page can emit a result the worker can check; longer term, headless Chrome + `wasm-bindgen-test` runs the same exports the worklet would.

Unit tests live under `#[cfg(test)]` in each module and run on the *native* host (cargo's default target) where possible — they cover pure logic (CBOR layouts, struct offsets, error paths) without crossing the wasm boundary.

## 7. Out of scope

The crate explicitly does **not** do:

- **Plugin instantiation.** That's `WebAssembly.instantiate` in JS. We receive a `handle` and operate on it through `_wclapInstance.*`.
- **WASI for plugins.** JS supplies WASI to each plugin instance. Our own WASI usage (if any) is unrelated.
- **Voice management.** Polyphony policy is the audio engine's job (Plinken side).
- **Native target.** See README. If a sandboxed native plugin distribution scheme appears later, a separate crate can target it.

## 8. Open questions (M0)

- **`no_std` or slim `std`.** Size vs. dev ergonomics. Default plan: `no_std + alloc`, lift to `std` only if a specific dep forces it.
- **`wasm-bindgen` or raw `extern "C"`.** The JS side calls our exports as bare wasm functions, not through `wasm-bindgen`'s descriptor protocol. Raw `extern "C"` is simpler and smaller. Pick raw unless something needs JS marshaling.
- **CLAP ABI version.** Plugins on the shelf report 1.2.2. We declare host as 1.2.2 to match. If a plugin reports newer minor, accept and pray — it's wasm, we can iterate fast.
- **Cross-origin isolation.** `apps/wclap-host` already sends COOP+COEP. M7 will lean on that for `SharedArrayBuffer`. No action at M0.
