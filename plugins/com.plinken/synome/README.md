# Synome

Polyphonic synthesizer plugin for the Plinken WCLAP host.

**Status: Phase A scaffold — silent.** This crate builds a WCLAP plugin that
loads cleanly, declares its audio + note ports, and renders silence. The DSP
will be ported in subsequent phases.

## Architecture goal

Anti-aliased polyphonic synth with:

- BLEP-corrected analog-style oscillators (saw / square / pulse / triangle)
- ADSR envelope per voice
- State-variable filter
- Voice-pool with steal-oldest fallback

## Where the DSP comes from

> **TODO (DSP port — not yet copied into this repo).** The existing Rust
> implementation lives in a private repo:
>
>     /Volumes/Music/TECH41/gitroot/plinken-synome/plugin/src/lib/rust/synth/
>
> Files of interest:
>
> - `synth.rs`        — top-level synth
> - `voice_pool.rs`   — polyphony + voice stealing
> - `voice.rs`        — per-voice render
> - `adsr.rs`         — envelopes
> - `filter.rs`       — state-variable filter
> - `table.rs`        — BLEP oscillator + wavetable system
> - `midi.rs`         — note / CC event handling
> - `peak_limiter.rs` — output limiter
> - `upsampler.rs`    — oversampling for high-quality BLEP
>
> Porting steps: strip `web-sys` / `wasm-bindgen` (they were used for the
> previous WAM wrapper), make the crate `no_std + alloc`, expose a single
> `pub fn process(...)` that the WCLAP `plugin.process` calls. The wrapper in
> `src/lib.rs` is where each of those pieces gets plugged in.

## Build

```sh
pnpm --filter @plinken/synome build
```

Produces `dist/synome.wclap.tar.gz`.

To test in the wclap-host page, paste the bundle URL into the URL bar above
the shelf (or drop the file from disk).
