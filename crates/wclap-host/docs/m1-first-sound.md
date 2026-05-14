# M1 — First sound

**Goal:** load `clack-gain.wasm` (the bare WCLAP plugin already on our
shelf), instantiate it, push a known stereo block through `process()`,
and confirm the output equals input × default-gain. End deliverable: a
`tiny_host` binary that renders 1 second of a 440 Hz sine through the
plugin and writes it to a WAV file.

This is M1 because nothing in CLAP matters if `process()` doesn't
work. Everything later (params, state, GUIs, threads) builds on the
audio loop.

## Done when

- `cargo test --release` passes the `tests/load_clack_gain.rs`
  integration test.
- `cargo run --release --example tiny_host -- clack-gain.wasm out.wav`
  produces a 1-second stereo 48 kHz 32-bit float WAV with audible
  output.
- Loading the same `clack-gain.wasm` byte-for-byte (from
  `apps/wclap-host/public/samples/clack-gain.wasm`) yields the same
  audio behavior as the browser host at https://wclap.plinken.org with
  that plugin on slot 1.

## Out of scope at M1

- `.tar.gz` bundles — M2.
- Params via events — at M1 the plugin runs with its default param
  value. The shape of `paramsGetValue` calls might exist for plumbing
  but param events do not.
- State save/load — M3.
- Note events — M4.
- Plugin GUIs — M5.
- Threads — M6.

## Steps

Each step ends in a runnable verification. Don't move to step N+1
until step N's verification passes.

### 0. Crate skeleton

Create `crates/wclap-host/Cargo.toml`:

```toml
[package]
name = "wclap-host"
version = "0.0.1"
edition = "2021"
license = "MIT"
description = "Load WCLAP (CLAP-as-wasm) plugins natively, expose them via clack-host"

[dependencies]
wasmtime = "25"
wasmtime-wasi = "25"
clack-host = "0.4"     # check current version; pin to the one we test against
thiserror = "1"
tracing = "0.1"

[dev-dependencies]
anyhow = "1"
hound = "3"
tracing-subscriber = "0.3"
```

Register the crate in the root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/wclap-host"]
```

Add empty `src/lib.rs` with `//! WCLAP host — see README.md`.

**Verify:** `cargo build -p wclap-host` succeeds with an empty crate.

### 1. `Engine`

`src/engine.rs`. Wraps `wasmtime::Engine` with a `Config` that:

- enables `wasm_simd`, `wasm_relaxed_simd` (clack-gain may use simd)
- enables `wasm_bulk_memory`
- disables `wasm_threads` for now (M6)
- sets `cranelift_opt_level(Speed)` for release builds

```rust
pub struct Engine {
    inner: wasmtime::Engine,
}

impl Engine {
    pub fn new() -> Result<Self, Error> { /* ... */ }
}
```

**Verify:** `Engine::new()` succeeds in a unit test.

### 2. `Bundle` (wasm-only path)

`src/bundle.rs`. M1 only handles bare `.wasm`:

```rust
pub struct Bundle {
    module: wasmtime::Module,
}

impl Bundle {
    pub fn load(engine: &Engine, bytes: &[u8]) -> Result<Bundle, Error> {
        // Sniff: must start with the wasm magic 00 61 73 6d
        if bytes.len() < 4 || &bytes[..4] != b"\0asm" {
            return Err(Error::Bundle("not a wasm module (M1 doesn't unpack tar.gz)".into()));
        }
        let module = wasmtime::Module::from_binary(&engine.inner, bytes)?;
        Ok(Bundle { module })
    }
}
```

Tar.gz path comes in M2; for now refuse it with a clear error.

**Verify:** unit test loads
`../../apps/wclap-host/public/samples/clack-gain.wasm` into a
`Bundle`. Loading a `.tar.gz` returns `Error::Bundle`.

### 3. WASI minimal surface

`src/wasi.rs`. The wasm built by `wclap-cpp` links against WASI for
`fd_write` (stdout/stderr only), `clock_time_get`, `random_get`. For
M1 we wire `wasmtime-wasi::WasiCtxBuilder::new()` with stdio inherited
to the host's stdout (so `console.log` from the plugin shows up in
the terminal during development), no filesystem access.

```rust
pub(crate) fn build_wasi() -> wasmtime_wasi::WasiCtx {
    wasmtime_wasi::WasiCtxBuilder::new()
        .inherit_stdout()
        .inherit_stderr()
        .build_p1()
}
```

Curating the WASI surface (denying calls dangerous to audio-thread
behavior) is `wasi-surface.md` work, not M1. For now we accept
whatever `wasmtime-wasi`'s preview1 gives us.

**Verify:** A plugin compiled with WASI imports instantiates without
linker errors (covered by step 5).

### 4. CLAP host imports

`src/imports.rs`. CLAP's plugin instance imports a small set of host
functions (the host interface). For M1 we provide stubs that:

- `host.get_extension()` → always returns null (no extensions supported yet)
- `host.request_callback()` → no-op
- `host.request_process()` → no-op
- `host.request_restart()` → no-op
- `host.log()` (if requested) → forward to `tracing::info!`

These are wired into a `wasmtime::Linker` and exported under the
import name space the wasm expects (typically `"env"` or matching
`wclap-bridge`'s convention).

Look at `vendor/wclap-host-js/cpp/wclap-cpp/include/wclap/_impl/`
to get the exact import names — they're shared with the browser host.

**Verify:** A test that calls `linker.instantiate(&store, &module)`
succeeds for `clack-gain.wasm` and `linker.get(&store, "clap_entry")`
returns a callable function.

### 5. `Plugin` instantiation + process loop

`src/plugin.rs`. The audio thread happy-path:

1. Call `clap_entry.init()` (CLAP entry init).
2. Get the plugin factory via `clap_entry.get_factory()`.
3. Get plugin descriptor 0.
4. Create a plugin instance via `factory.create_plugin(host, plugin_id)`.
5. `plugin.init()` then `plugin.activate(sample_rate, min_frames, max_frames)`.
6. `plugin.start_processing()`.
7. `plugin.process(process_struct)` per block — input → output via
   wasm memory views.
8. `plugin.stop_processing()` and `plugin.deactivate()` on teardown.

The `process_struct` is a `clap_process` written into wasm memory.
Audio buffers (`f32` arrays) are pointers into wasm memory; the host
writes input there before calling `process`, reads output from there
after.

```rust
pub struct Plugin {
    store: wasmtime::Store<HostState>,
    instance: wasmtime::Instance,
    plugin_ptr: u32,       // wasm pointer to the clap_plugin_t
    // pre-allocated pointers into wasm memory
    process_ptr: u32,
    audio_in: [u32; 2],    // L, R input buffer pointers
    audio_out: [u32; 2],   // L, R output buffer pointers
    block_size: u32,
}

impl Plugin {
    pub fn process(&mut self, input: [&[f32]; 2], output: [&mut [f32]; 2]) -> Result<(), Error> {
        // Copy input into wasm memory at audio_in pointers
        // Call wasm process()
        // Copy output from wasm memory at audio_out pointers
    }
}
```

We don't go through `clack-host` traits at M1 — too much surface to
implement at once. We expose a deliberately small `Plugin::process`
that the example uses directly. The `clack-host` adapter lands in M3.

**Verify:** `tests/load_clack_gain.rs` instantiates, processes one
block of all-1.0 samples, asserts the output equals 1.0 (gain's
default is unity).

### 6. Example: `tiny_host`

`examples/tiny_host.rs`:

```text
$ cargo run --release --example tiny_host -- clack-gain.wasm out.wav
```

Behavior:

1. Read the WCLAP bytes from argv[1].
2. `Engine::new()` → `Bundle::load(...)` → `Bundle::instantiate(plugins[0])`.
3. Generate a 1-second 440 Hz sine into a stereo `f32` buffer at 48 kHz.
4. Call `plugin.process(...)` in 256-frame blocks.
5. Write the result to argv[2] via `hound::WavWriter` as 32-bit float
   stereo at 48 kHz.

Use `tracing_subscriber` to surface plugin logs.

**Verify:** Run it. Open `out.wav` in any audio player. You should
hear a 1-second 440 Hz sine — unmodified, because `clack-gain` at
default param is unity. Compare sample values against the input — they
should match within `f32` rounding.

### 7. Integration test

`tests/load_clack_gain.rs`:

```rust
use wclap_host::{Engine, Bundle};

const PLUGIN_BYTES: &[u8] =
    include_bytes!("../../../apps/wclap-host/public/samples/clack-gain.wasm");

#[test]
fn process_unity_gain() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    let bundle = Bundle::load(&engine, PLUGIN_BYTES)?;
    let descriptor = &bundle.plugins()[0];
    let mut plugin = bundle.instantiate(&descriptor.id, /* config */)?;

    let l: [f32; 256] = [1.0; 256];
    let r: [f32; 256] = [0.5; 256];
    let mut lo = [0.0f32; 256];
    let mut ro = [0.0f32; 256];

    plugin.process([&l, &r], [&mut lo, &mut ro])?;

    for (i, &v) in lo.iter().enumerate() {
        assert!((v - 1.0).abs() < 1e-6, "L[{i}]={v}");
    }
    for (i, &v) in ro.iter().enumerate() {
        assert!((v - 0.5).abs() < 1e-6, "R[{i}]={v}");
    }
    Ok(())
}
```

**Verify:** `cargo test --release -p wclap-host` passes.

## Risks at M1

- **CLAP entry ABI in wasm.** The exact symbol name and shape needs to
  match what `wclap-cpp` emits. If something doesn't link, look at
  `vendor/wclap-host-js/cpp/wclap-cpp/include/wclap/_impl/` and the
  upstream `wclap-bridge` to mirror the host import side.
- **Pointer arithmetic in wasm memory.** `clap_process` is a C struct;
  field offsets must be computed against the layout `wclap-cpp` emits.
  Cross-check with the JS host code in
  `apps/wclap-host/src/wclap-runtime/clap-audioworkletprocessor.mjs`
  for which offsets/conversions they perform.
- **WASI imports we don't provide.** `wasmtime` will refuse to
  instantiate if the plugin imports a symbol we haven't supplied.
  Read the missing-import error and stub it; revisit during the
  wasi-surface design.

If `process()` produces zero output: check the worklet processor in
the browser host (linked above) — it's the working reference. Same
plugin, same bytes; if the JS path works and the Rust one doesn't, the
host imports or memory layout are wrong.

## After M1

Once `clack-gain` plays through `tiny_host`, M2 starts: unpack
`.wclap.tar.gz`, enumerate multiple plugins per bundle, and load
`signalsmith-basics`'s chorus the same way.
