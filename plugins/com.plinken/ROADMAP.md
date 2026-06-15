# Plugin Roadmap — `com.plinken`

The next batch of first-party WCLAP plugins. Scope: **core mixing FX** — the
everyday channel-strip effects that fill the empty category buckets in the Plinken
DAW's plugin browser (`PluginPicker` already has **EQ, Dynamics, Reverb, Delay,
Modulation, Distortion** filters with nothing in them yet).

All six are authored in **Rust + fundsp**, copying the `vocal-limiter/` template,
and use the public [`widgets/`](../../widgets) GUI library (auto-bundled into each
tarball by `scripts/bundle-wclap.mjs`). No external/private UI framework — these
ship from this public repo and stay self-contained.

## Status

All six are **built and shelved** — each compiles to `wasm32-unknown-unknown`,
bundles to a `.wclap.tar.gz`, and is listed in `shelf.json` (so the public
`wclap-host` demo loads them). DSP is hand-rolled Rust on the shared
`crates/wclap-plugin` scaffold (no `fundsp` dependency), with `widgets/`-based UIs.

| | id | name | category tags | key params | UI widgets |
|---|---|---|---|---|---|
| [x] | `com.plinken.eq` | Parametric EQ | `audio-effect`,`equalizer`,`eq` | low shelf (freq/gain), 2 peaks (freq/gain/Q), high shelf (freq/gain) | Pot, Meter |
| [x] | `com.plinken.compressor` | Compressor | `audio-effect`,`compressor`,`dynamics` | threshold, ratio, attack, release, makeup, knee | Pot, GR Meter |
| [x] | `com.plinken.delay` | Delay | `audio-effect`,`delay`,`echo` | time, feedback, tone (LP), mix, ping-pong | Pot, Toggle, Meter |
| [x] | `com.plinken.reverb` | Reverb | `audio-effect`,`reverb` | size/decay, damping, pre-delay, mix, width | Pot, Meter |
| [x] | `com.plinken.chorus` | Chorus | `audio-effect`,`chorus`,`modulation` | rate, depth, voices, mix, spread | Pot, Meter |
| [x] | `com.plinken.saturator` | Saturation | `audio-effect`,`distortion`,`saturation` | drive, type (tanh/tube/fold), tone, mix, output | Pot, Meter |

Each plugin's feature tags map onto one of the DAW picker's existing FX category
buckets, so it lands under the right filter with no host-side change.

### Follow-ups

- Runtime audio QA in the `wclap-host` browser demo (DSP was verified by
  construction + compile; in-browser listening is the remaining check).
- Surface them inside the **Plinken DAW** (catalog row + artifact hosting — a
  separate, private-repo step; see "Beyond this repo" below).

## How to build each one

The pattern is the same for all six — copy an existing Rust plugin and swap the DSP.

1. **Copy the template** — `plugins/com.plinken/vocal-limiter/`. You get:
   - `Cargo.toml` — `crate-type = ["cdylib"]`, depends on
     `wclap-plugin = { path = "../../../crates/wclap-plugin" }`.
   - `build.rs` — emits the two mandatory linker flags (see gotchas below).
   - `src/lib.rs` — a `static PLUGIN_DEF: PluginDef`, the `Plugin` trait impl, and
     `#[no_mangle] pub fn _initialize() { init_plugin::<MyPlugin>(&PLUGIN_DEF); }`.
   - `plugin.json` — manifest (bump `id`, `name`, `features`, `artifact`, `ui` sizes).
   - `package.json` — `build:wasm` + `build` scripts.

2. **Implement the DSP** against the shared scaffold in
   [`crates/wclap-plugin/src/lib.rs`](../../crates/wclap-plugin/src/lib.rs):
   - `PluginDef` — id/name/vendor/version/features, `audio_inputs/outputs = 1`,
     `ui_path: Some(b"/ui/index.html\0")`.
   - `Plugin` trait — `new`, `activate(sample_rate, max_frames)`, `process(ctx)`,
     plus the param hooks `params() -> &[ParamDef]`, `get_param`, `set_param`,
     and `latency_samples()` for the lookahead-based ones.
   - In `process`, grab `ctx.stereo_io()` for the common stereo case; push meter /
     GR values to the UI with `ctx.send_to_ui(&cbor_bytes)`.

3. **Build the UI** (`ui/index.html`) importing from the bundled widgets:
   `../widgets/pot.mjs`, `fader.mjs`, `meter.mjs`, `toggle.mjs`, and
   `cbor.mjs` for the wasm↔UI messages. Widget `id` must match the plugin's
   `ParamDef.id`. Set the matching `compact_size`/`expanded_size` in `plugin.json`.

4. **Bundle & list** —
   - `node scripts/bundle-wclap.mjs plugins/com.plinken/<name>` → `dist/<name>.wclap.tar.gz`
     (this also injects `widgets/` into the tarball).
   - `node scripts/build-shelf.mjs` → regenerates `shelf.json` for the host + site.

### Rust pre-flight checklist (gotchas — see root `CLAUDE.md`)

- **`build.rs` needs BOTH flags:** `--export-table` *and* `--growable-table`.
  Without the growable table the host traps on `WebAssembly.Table.grow()` at load.
- **`clap_entry` is the struct itself**, not a pointer-wrapper static (the scaffold
  already declares it correctly — don't reintroduce a wrapper).
- **No `Box<dyn AudioUnit>` for fundsp graphs** — LTO drops the vtable's "unused"
  methods and the first call traps with `null function`. Store concrete types
  (`An<...>`) and call methods via UFCS (`AudioUnit::tick(&mut self.unit, ...)`).

## Beyond this repo

Authoring + bundling here makes a plugin loadable by the public `wclap-host`
demo (via `shelf.json`). Surfacing it inside the **Plinken DAW** additionally
needs a catalog entry (the private `plinken` repo's plugin catalog + artifact
hosting). That step is tracked separately and is **not** part of this public repo.
