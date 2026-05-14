# M1 — First sound

**Goal:** the example app at [`apps/wclap-host`](../../../apps/wclap-host/) renders its 440 Hz test tone through a WCLAP plugin **using our Rust-built `host.wasm`** instead of the upstream C++ build. End deliverable: drop a plugin onto the page, press Play, hear it.

This is M1 because nothing else in the crate matters if the audio loop doesn't run end-to-end. Params, state, events, GUIs, threads, multi-plugin bundles all build on the working `pluginProcess`.

## Done when

- `cargo build --target wasm32-unknown-unknown --release` produces `target/wasm32-unknown-unknown/release/wclap_host.wasm`.
- Replacing `apps/wclap-host/src/wclap-runtime/host.wasm` with that file (after `wasm-opt -Oz`) lets `pnpm --filter @plinken/wclap-host dev` run unchanged.
- The page plays audibly through each of these:
  - `clack-gain.wasm` (Rust clack, the simplest case — unity gain by default)
  - `as-clap-example.wclap.wasm` (AssemblyScript path — proves toolchain-independence)
  - `com.plinken.auto-pan.wclap.wasm` (Plinken's own plugin — audible motion, easy to ear-check)
- No console errors. The C++ host's binary stays in the tree as a fallback.

## Out of scope at M1

- `.wclap.tar.gz` bundles — M2.
- `clack-polysynth` and any synth that needs note events — M4.
- Params, state, GUI, multi-plugin — see roadmap.
- The standalone Rust API (`wclap_host::Plugin`-style). M1 only proves the wasm export surface; idiomatic Rust embedding gets designed once the export plumbing is solid.

## Pre-requisites

Before step 1: **delete the existing `src/` and `examples/`.** They were written against the old wasmtime-native scope and don't compile under the new target. Keep the docs in place. A `git rm` is fine — git history is the audit trail.

```
crates/wclap-host/
├── Cargo.toml         # rewritten in step 0
├── docs/              # unchanged (already rewritten)
├── README.md          # unchanged (already rewritten)
└── src/
    └── lib.rs         # only file at step 0
```

## Steps

Each step ends in a runnable verification. Don't move to step N+1 until step N's verification passes.

### 0. Crate skeleton

`Cargo.toml`:

```toml
[package]
name = "wclap-host"
version = "0.0.1"
edition = "2021"
license = "MIT"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
thiserror = { version = "1", default-features = false }
minicbor = { version = "0.25", default-features = false, features = ["alloc"] }

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
strip = true
```

`src/lib.rs` exports one no-op so we can verify the toolchain works:

```rust
#![no_std]
extern crate alloc;

#[no_mangle]
pub extern "C" fn createBytes() -> u32 {
    0
}
```

**Verify:**

```sh
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
ls -l target/wasm32-unknown-unknown/release/wclap_host.wasm
```

The artifact exists and `wasm-objdump -x` lists `createBytes` as an export. Size should be a few KB.

### 1. Declare the wasm imports

`src/instance.rs`:

```rust
#[link(wasm_import_module = "_wclapInstance")]
extern "C" {
    pub fn init32(handle: u32) -> u32;
    pub fn malloc32(handle: u32, size: u32) -> u32;
    pub fn memcpyToOther32(handle: u32, dest_p: u32, src: *const u8, count: u32) -> u32;
    pub fn memcpyFromOther32(handle: u32, dest: *mut u8, src_p: u32, count: u32) -> u32;
    pub fn call32(
        handle: u32, wasm_fn: u32, is_ptr_to_fn: u32,
        result_ptr: *mut u8, args_ptr: *const u8, args_count: u32,
    ) -> u32;
    pub fn registerHost32(
        handle: u32, ctx: u32, fn_idx: u32,
        sig: *const u8, sig_len: u32,
    ) -> u32;
    pub fn countUntil32(
        handle: u32, start_p: u32, until: *const u8,
        item_size: u32, max_count: u32,
    ) -> u32;
    pub fn runThread(handle: u32, thread_id: u32, start_arg: u64);
    pub fn release(handle: u32);
}

#[link(wasm_import_module = "env")]
extern "C" {
    pub fn log(plugin_ptr: u32, severity: u32, msg_ptr: *const u8, len: u32);
    pub fn paramsRescan(plugin_ptr: u32, flags: u32);
    pub fn stateMarkDirty(plugin_ptr: u32);
    pub fn webviewSend(plugin_ptr: u32, ptr: *const u8, len: u32);
    pub fn eventsOutTryPush(plugin_ptr: u32, ptr: *const u8, len: u32) -> u32;
}
```

**Verify:** `cargo build …` still succeeds. The imports section of the resulting wasm matches what the C++ `host.wasm` declared (cross-check with `wasm-objdump -x ../apps/wclap-host/src/wclap-runtime/host.wasm`).

### 2. Bytes channel

`src/bytes.rs`. The C++ host owns a pool of `std::vector<uint8_t>`-equivalent buffers JS can resize and read. We do the same.

```rust
use alloc::vec::Vec;
use core::cell::RefCell;
use alloc::collections::BTreeMap;

struct BytesPool {
    next_id: u32,
    map: BTreeMap<u32, Vec<u8>>,
}

thread_local! {
    static POOL: RefCell<BytesPool> =
        RefCell::new(BytesPool { next_id: 1, map: BTreeMap::new() });
}

#[no_mangle]
pub extern "C" fn createBytes() -> u32 {
    POOL.with(|p| {
        let mut p = p.borrow_mut();
        let id = p.next_id;
        p.next_id += 1;
        p.map.insert(id, Vec::new());
        id
    })
}

#[no_mangle]
pub extern "C" fn resizeBytes(handle: u32, len: u32) -> u32 {
    POOL.with(|p| {
        let mut p = p.borrow_mut();
        let buf = p.map.get_mut(&handle).expect("bad bytes handle");
        buf.resize(len as usize, 0);
        buf.as_mut_ptr() as u32
    })
}

#[no_mangle]
pub extern "C" fn getBytesData(handle: u32) -> u32 { … }

#[no_mangle]
pub extern "C" fn getBytesLength(handle: u32) -> u32 { … }
```

`no_std` note: `thread_local!` needs `std`. For `no_std`, swap for `static mut` behind a single-threaded assumption — the wasm module runs in one JS context. Reconsider when threads land (M7).

**Verify:** unit test on the native target round-trips bytes (`create → resize → write via raw ptr → getBytesData/Length`).

### 3. `makeHosted` + the hosted-plugin map

`src/host.rs`. `wclap-host-js` calls `makeHosted(wclapInstancePtr)` where `wclapInstancePtr` is the `Instance *` JS allocated for a plugin module. We get the opaque handle from the bridge and store host-side bookkeeping (the `clap_host_t` we'll hand the plugin in `createPlugin`, a registry of host-stub indices, etc.).

For M1 the bookkeeping is minimal: just the handle and a `Vec` of `Plugin`s once they exist.

**Verify:** `makeHosted` returns a non-zero pointer; calling it twice returns distinct pointers.

### 4. `createPlugin`

`src/factory.rs` + `src/plugin.rs`. Walk the plugin's CLAP entry exactly as `tiny_host.rs` did, but via `_wclapInstance.call32` instead of `wasmtime::TypedFunc`.

Reuse from the wasmtime sketch:

- struct offsets for `clap_entry`, `clap_plugin_factory_t`, `clap_plugin_descriptor`, `clap_plugin_t`, `clap_host_t`
- the host-stub set (`get_extension`, `request_restart`, `request_process`, `request_callback`, plus the event-list stubs `size`, `get`, `try_push`)

The stubs go through `_wclapInstance.registerHost32`, which inserts a host-side wasm function into the plugin's table and returns the resulting table index. Those indices become the function-pointer fields in our `clap_host_t` written into plugin memory.

Sequence:

1. `_wclapInstance.init32(handle)` once — returns the `Instance *` we already have.
2. Read `clap_entry` from the plugin's exported global (it's an i32 offset into plugin memory; fetch via `memcpyFromOther32`).
3. `_wclapInstance.call32` on `clap_entry.init` with a NULL path argument.
4. `_wclapInstance.call32` on `clap_entry.get_factory` with the C-string `"clap.plugin-factory"` (alloc via `malloc32`, write via `memcpyToOther32`).
5. From the returned factory pointer, read `get_plugin_count` / `get_plugin_descriptor` / `create_plugin` function indices, then call them via `call32`.
6. Build our `clap_host_t` in plugin memory; register the 4 host stubs via `registerHost32`; write the indices into the struct.
7. `create_plugin(factory, host, plugin_id)` — store the returned `clap_plugin_t *` against our `pluginPtr` handle.

`pluginPtr` returned to JS is a Rust-side handle (`u32`), not the plugin's own pointer — JS treats it opaquely.

**Verify:** Loading `clack-gain.wasm`, then `as-clap-example.wclap.wasm`, then `com.plinken.auto-pan.wclap.wasm` all return non-zero `pluginPtr`s. `pluginGetInfo` (next step) is needed before we can prove the right plugin came out.

### 5. `pluginGetInfo`

CBOR-encode the descriptor `{ id, name, vendor, version, features, audio_ports: [...], note_ports: [...], params_count }` into a bytes-channel buffer. JS decodes via `cbor.mjs` and renders it in the page sidebar (the existing worklet code already does this for the C++ host).

Audio-ports come from the `clap.audio-ports` extension (`get` and `count`). M1 only reads them — no events through the extension yet.

**Verify:** the page sidebar shows the same metadata for each test plugin as the C++ host shows. Spot-check `clack-gain` (1 stereo input, 1 stereo output, 0 params at default) and `com.plinken.auto-pan` (whatever it actually declares).

### 6. `pluginStart`

Allocate audio buffers, write the `clap_process_t` skeleton, call `clap_plugin.activate` + `start_processing`, return a CBOR map of per-channel `Float32Array`-pointers:

```json
{ "inputs":  [[<ptrL>, <ptrR>]],
  "outputs": [[<ptrL>, <ptrR>]] }
```

JS uses these to write/read sample data directly into the plugin's memory each block — no wasm↔JS copy per sample.

`maxFramesCount` comes from the worklet's `maxFramesCount` (128 today). Sample rate is passed in (`globalThis.sampleRate` in the worklet).

**Verify:** call from JS, check the returned CBOR has 1 input port + 1 output port for `clack-gain`; both have 2 channels (stereo).

### 7. `pluginProcess`

Per block:

1. Write the current `frames_count` into the `clap_process_t` (other fields stay put).
2. `_wclapInstance.call32` on `clap_plugin.process` with `(plugin_ptr, &clap_process)`.
3. Return the process-status integer as-is.

The JS side has already copied input samples into the plugin's `Memory` at the `inputs` pointers from step 6; on return it reads outputs from the `outputs` pointers. None of that copy logic lives in the wasm — the JS worklet (`clap-audioworkletprocessor.mjs`) already implements it for the C++ host.

`pluginMainThread` is a no-op stub at M1.

**Verify:** the loaded plugin is audible. For `clack-gain`, output should equal input within `f32` rounding (it's a unity gain at default).

### 8. Drop-in + manual smoke test

```sh
cargo build --target wasm32-unknown-unknown --release
wasm-opt -Oz \
  -o apps/wclap-host/src/wclap-runtime/host.wasm \
  target/wasm32-unknown-unknown/release/wclap_host.wasm
pnpm --filter @plinken/wclap-host dev
```

Manual checks (in order):

1. Open the page. No console errors during host wasm compile.
2. Drop `clack-gain.wasm`. Sidebar shows correct metadata. Press Play — hear the 440 Hz tone unchanged.
3. Drop `as-clap-example.wclap.wasm`. Same.
4. Drop `com.plinken.auto-pan.wclap.wasm`. Hear the panning movement on the tone.
5. Confirm `host.wasm` size is a fraction of the C++ build (~3.1 MB → expect well under 1 MB).

### 9. Save the artifact comparison

Once steps 1–8 pass, record the **size delta** and any **export-list deltas** between our `host.wasm` and the upstream C++ build in this doc (or a follow-up file). Future regressions are easier to catch when there's a baseline written down.

## Risks at M1

- **`registerHost32` semantics.** The C++ side uses a templated signature string (`"i32(i32,i32) -> i32"` etc.); JS picks a matching pre-built host shim. Confirm the exact string format in [`wclap-host-js/es6/wclap.mjs`](../../../vendor/wclap-host-js/es6/wclap.mjs) (the `registerHost32` import handler) before writing host stubs. If our signature strings don't match what JS expects, the stub indices we get back will trap when the plugin tries to call them.
- **`clap_entry` lookup.** Different toolchains export the entry slightly differently: clack uses a global named `clap_entry` holding an i32 offset; `wclap-cpp` exports `wclap_entry` (also a global, possibly a direct struct). Confirm against each test plugin via `wasm-objdump -x` before assuming one format.
- **Audio buffer pointer stability.** The plugin's `Memory` can grow between `pluginStart` and the first `pluginProcess` — if it does, the JS-side `Float32Array` views we handed out become detached. Either size the plugin memory at activation and never grow, or have JS refresh views before each block. The C++ host handles this somewhere in `wclap-cpp` — find that code path and mirror it.

If `pluginProcess` produces zero output: compare against the C++ host running the same plugin in the same page. Same wasm bytes; if the C++ path works and the Rust one doesn't, our `clap_process_t` layout, our host stubs, or our event-list pointers are wrong.

## After M1

M2: `.wclap.tar.gz` unpacking + multi-plugin enumeration (`getInfo`). Unblocks the two Signalsmith bundles in the test corpus.
