# `wclap-host` implementation plan

Living spine doc for the Rust WCLAP host crate. Decisions made here are
binding until something measurable contradicts them; aspirational stuff
belongs in the [README](../README.md) instead.

Scope of this doc: tech-stack choices, module layout, error/threading
model, testing strategy. Per-milestone work plans live alongside
(`m1-first-sound.md`, `m2-bundles.md`, …) and are written when that
milestone is the next thing to ship.

## 1. Tech stack

| Dependency | Use | Why |
|---|---|---|
| `wasmtime` (≥ 25) | wasm runtime | Production-grade, used by Fastly/Shopify, has solid WASI support, cwasm pre-compilation, threads feature. Mature enough that we don't have to babysit it. |
| `wasmtime-wasi` | WASI surface | Curated; we whitelist imports rather than expose the whole thing. |
| `clack-host` | CLAP host bindings | Already in our ecosystem — `clack-gain` and `clack-polysynth` on the shelf come from the same project. Same author, same idioms. |
| `tar` + `flate2` | unpack `.wclap.tar.gz` | Pure-Rust, no system deps. Matches what `apps/wclap-host` does in JS via the same tar+gz logic. |
| `thiserror` | library errors | Typed errors for crate consumers. No `anyhow` in lib code. |
| `tracing` | logging | De facto Rust standard. Lets host engines wire spans through. |
| `hound` (dev-dep) | WAV in examples | Tiny, no fuss, fine for the smoke test. |
| `criterion` (dev-dep, M7+) | benchmarks | Added when we measure realtime overhead. Not now. |

**Not** going to use:
- `wasmer` — fine runtime but we'd duplicate `wasmtime`'s ecosystem (cwasm cache, threads, WASI). One runtime is enough.
- `anyhow` in library code — only in `examples/` and `tests/`.
- A custom wasm runtime — premature.

Pin minor versions in `Cargo.toml`; renovate updates them later.

## 2. Public API sketch

The crate exposes one `Engine` (shared, immutable after construction),
many `Bundle`s (one per `.wclap.*`), and many `Plugin`s (one per
loaded slot). `Plugin` implements the `clack-host` consumer traits so
the rest of the DAW doesn't see wasm at all.

```rust
// crates/wclap-host/src/lib.rs (sketch)

pub struct Engine { /* wasmtime::Engine + caches */ }

impl Engine {
    pub fn new() -> Result<Self>;
    pub fn with_config(cfg: EngineConfig) -> Result<Self>;
}

pub struct Bundle { /* Module + extracted files map */ }

impl Bundle {
    pub fn load(engine: &Engine, bytes: &[u8]) -> Result<Bundle>;
    pub fn plugins(&self) -> &[PluginDescriptor];
    pub fn instantiate(&self, plugin_id: &str, config: ActivateConfig)
        -> Result<Plugin>;
}

pub struct Plugin { /* wasmtime::Store + clack-host plumbing */ }

impl Plugin {
    pub fn descriptor(&self) -> &PluginDescriptor;
    pub fn process(&mut self, audio: AudioBuffers, events: EventBuffers)
        -> Result<ProcessStatus>;
    pub fn save_state(&mut self) -> Result<Vec<u8>>;        // M3
    pub fn load_state(&mut self, state: &[u8]) -> Result<()>; // M3
    pub fn open_gui(&mut self, host: GuiHostHandle) -> Result<()>; // M5
}
```

`AudioBuffers`, `EventBuffers`, `ProcessStatus` are re-exports / thin
wrappers around `clack-host` types; the goal is that a DAW that
already uses `clack-host` swaps in `wclap_host::Plugin` and the call
sites compile as-is.

The exact shape is final-design at M1. This sketch is for orientation.

## 3. Module layout

```
src/
├── lib.rs           # re-exports, top-level docs, prelude
├── engine.rs        # Engine: wasmtime::Engine config, cwasm cache
├── bundle.rs        # Bundle: sniff (wasm vs tar.gz), unpack, compile
├── instance.rs      # wasmtime::Store + Instance + Memory wiring
├── imports.rs       # host-side fn imports (CLAP host iface)
├── wasi.rs          # curated WASI surface
├── factory.rs       # call clap_entry, enumerate descriptors
├── plugin.rs        # Plugin: activate + process loop
├── events.rs        # CLAP event marshaling (M3+)
├── gui.rs           # webview glue (M5+)
├── threads.rs       # threaded process path (M6+)
└── error.rs         # `Error` enum (thiserror)
```

`gui.rs` and `threads.rs` are placeholder files at M1 with a single
`unimplemented!()` and a `// MILESTONE: M5 / M6` comment — keeps the
shape visible without committing code.

## 4. Threading model

- **Control thread:** `Engine::new`, `Bundle::load`,
  `Bundle::instantiate`, `Plugin::save_state` / `load_state`,
  `Plugin::open_gui`. Allowed to allocate, take locks, talk to the
  filesystem.
- **Audio thread:** `Plugin::process` only. No allocation, no
  blocking I/O, no `Mutex::lock`. The wasm memory was sized at
  `instantiate` time; the audio loop reads/writes the existing
  `wasmtime::Memory` view.

Each `Plugin` owns its `wasmtime::Store`. `Store` is `!Sync`, so
moving a plugin between threads requires `mem::take` / channel
hand-off — but in practice a DAW pins each plugin to a worker thread
and processes there.

For threaded plugins (M6), the inner CLAP plugin can spawn its own
wasm-side workers via wasmtime's `threads` feature; the host doesn't
care because that's behind the wasm boundary.

## 5. Error handling

```rust
// src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid WCLAP bundle: {0}")]
    Bundle(String),

    #[error("wasm compilation failed: {0}")]
    Compile(#[from] wasmtime::Error),

    #[error("wasm instantiation failed: {0}")]
    Instantiate(String),

    #[error("CLAP plugin '{id}' not found in bundle")]
    PluginNotFound { id: String },

    #[error("CLAP ABI error: {0}")]
    ClapAbi(String),

    #[error("WASI denied: {0}")]
    WasiDenied(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
```

Library returns `Result<_, Error>`. Examples/tests use `anyhow`.
`Error: Send + Sync + 'static` so it's `?`-able everywhere.

## 6. Testing

Minimum integration test set, in order of how much they prove:

1. **`tests/bundle_sniff.rs`** — load every artifact from
   `apps/wclap-host/public/samples/` and assert format detection
   matches what we expect (wasm vs tar.gz). Pure parsing, no
   wasmtime. Fast.

2. **`tests/load_clack_gain.rs`** — load `clack-gain.wasm`,
   instantiate, send a known stereo block in (full-scale sine),
   assert output is gained by the default parameter value. Proves
   the CLAP factory + process loop work end-to-end.

3. **`tests/multi_plugin.rs`** (M2+) — load `signalsmith-basics.wclap.tar.gz`,
   enumerate plugins, confirm the count matches what the browser host
   shows.

4. **`tests/state_roundtrip.rs`** (M3+) — set params, save state, load
   in a fresh instance, confirm rendered output is bit-identical.

5. **`tests/note_routing.rs`** (M4+) — drive a wasm synth with a
   note-on, confirm non-silent output.

Tests source their bundles from `../../apps/wclap-host/public/samples/`
via a small helper so we don't duplicate the binaries.

`cargo test --release` is the canonical command. Debug builds of
wasmtime are slow enough that test runs feel broken; release-mode
testing is the norm in this crate.

## 7. CI (later)

Out of scope at M0. When we add it:

- `cargo build --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --release`
- `cargo fmt --check`
- Matrix: Linux + macOS. Windows when we know the realtime story.

No need to publish from CI until M8 (v1.0).

## 8. Out of scope at the crate level

Things this crate explicitly does **not** do, so a DAW author knows
where to draw the line:

- **Voice management.** The crate hosts plugins; the DAW assigns
  notes to instances, pools voices, handles polyphony policy.
- **Plugin registry / discovery.** Use
  [`plinken.org/shelf.json`](https://plinken.org/shelf.json) (or any
  registry following [`REGISTRY.md`](../../../REGISTRY.md)) from the
  DAW side. This crate accepts bytes, not URLs.
- **Audio I/O.** No `cpal`, no system audio. The crate's input is a
  `f32` buffer; output is a `f32` buffer.
- **MIDI device handling.** Same — the DAW reads MIDI, normalizes to
  CLAP events, passes them in.

The crate's contract: bytes in → CLAP-spec plugin behavior out. Everything
upstream/downstream is the DAW's problem.

## 9. Decisions still to make

These are the open design questions that need a concrete answer
**before** writing code in their respective milestones, not while
writing it:

- **Memory model**: do we always provide `Memory::new(min, max)` or
  always let the wasm declare its own? Browser host imports memory
  with a 1024-page cap (per `cap-wasm-memory.ts`); native may not
  need the cap if we're generous with address space. — M1
- **cwasm caching**: where do compiled modules cache? `XDG_CACHE_HOME`?
  Plugin-provided directory? Disabled by default? — M1
- **Plugin ABI version skew**: CLAP is at 1.2.7. Do we refuse plugins
  that report a newer minor than we know about, or accept and pray? — M1
- **GUI host interface**: webview crate (`wry`?), windowing surface
  (DAW owns the parent window), param sync transport (postMessage in
  the browser; something else here). — M5
- **Threading capability negotiation**: how does the plugin discover
  whether the host's wasmtime supports threads, and fall back? — M6
