# Plugin Roadmap — `com.plinken`

The next batch of first-party WCLAP plugins. Scope: **core mixing FX** — the
everyday channel-strip effects that fill the empty category buckets in the Plinken
DAW's plugin browser (`PluginPicker` already has **EQ, Dynamics, Reverb, Delay,
Modulation, Distortion** filters with nothing in them yet).

All six are authored in **Rust + fundsp**, copying the `vocal-limiter/` template,
and use the public [`widgets/`](../../widgets) GUI library (auto-bundled into each
tarball by `scripts/bundle-wclap.mjs`). No external/private UI framework — these
ship from this public repo and stay self-contained.

## Todo

| | id | name | author | category tags | key params | UI widgets |
|---|---|---|---|---|---|---|
| [ ] | `com.plinken.eq` | Parametric EQ | Rust (fundsp) | `audio-effect`,`equalizer`,`eq` | 4 bands × (freq, gain, Q); low/high shelf toggles | Pot, Toggle, Meter |
| [ ] | `com.plinken.compressor` | Compressor | Rust (fundsp) | `audio-effect`,`compressor`,`dynamics` | threshold, ratio, attack, release, makeup, knee | Pot, Fader, GR Meter |
| [ ] | `com.plinken.delay` | Delay | Rust (fundsp) | `audio-effect`,`delay`,`echo` | time (ms/sync), feedback, mix, tone (LP), ping-pong | Pot, Toggle, Meter |
| [ ] | `com.plinken.reverb` | Reverb | Rust (fundsp) | `audio-effect`,`reverb` | size/decay, damping, pre-delay, mix, width | Pot, Meter |
| [ ] | `com.plinken.chorus` | Chorus | Rust (fundsp) | `audio-effect`,`chorus`,`modulation` | rate, depth, voices, mix, spread | Pot, Toggle |
| [ ] | `com.plinken.saturator` | Saturation | Rust (fundsp) | `audio-effect`,`distortion`,`saturation` | drive, type (tanh/tube/fold), tone, mix, output | Pot, Selector/Toggle, Meter |

Each plugin's feature tags map onto one of the DAW picker's existing FX category
buckets, so it lands under the right filter with no host-side change.

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
