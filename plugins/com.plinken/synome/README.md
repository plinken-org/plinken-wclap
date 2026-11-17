# Synome

Moog-style polyphonic synthesizer plugin for the Plinken WCLAP host.

**Status: DSP ported — makes sound.** Notes arrive through the CLAP note
port (wclap-plugin event-queue hooks), params through `clap.params` /
webview, knob state persists via `clap.state`. No UI yet (`has_ui: false`);
next phases add the sample oscillator mode and a `widgets/` UI.

## What's inside

- 2 oscillators with saw/pulse morphing, FM between them, hard sync; each
  can instead scan a wavetable or play the loaded sample
- Moog ladder filter (2/4 pole, LP/BP/HP) with env/LFO/keytrack modulation
- 3 ADSR envelopes (amp, filter, mod)
- LFO (5 shapes, onset delay, retrig), vibrato
- White/pink noise mixer channel
- Effects: chorus/phaser/flanger, delay, comb+allpass reverb
- 16-voice pool (4/8/12/16 selectable) with retrigger → idle →
  oldest-releasing stealing; mono/legato modes; glide
- Master drive + soft clip

The DSP primitives live in the shared [`plinken-dsp`](../../../crates/plinken-dsp)
crate (vendored from the private monorepo — the copy here is canonical).
`src/synth.rs` is a near-verbatim port of the monorepo Synome engine;
`src/params.rs` freezes param ids 0–73 (they match the `synome.json` UI and
saved state blobs — only append).

Arpeggiator params (61–66) are declared but inert, same as the source
engine. The NNUE match-sound feature is not ported yet.

## TODO

- **Oscillator engines — implement Gran / Phys.** The OSC Mode pot offers
  five engines (`Anlg` / `WTbl` / `Gran` / `Phys` / `Smpl`); **Analog**,
  **Wavetable** and **Sample** work. `Granular` and `Physical` still fall
  through to `process_analog` in `crates/plinken-dsp/src/osc.rs` (marked
  `// TODO`) and sound identical to Analog. Either implement them (granular
  cloud, Karplus-Strong) or hide those positions from the Mode pot
  (`ui/index.html` `OSC_MODES` + pot `max`) before shipping.
- **Wavetable: import and a display.** Only the generated factory bank is
  selectable. User tables (Serum-style WAV, 2048 samples per frame) need an
  FFT of the imported cycles into `Spectrum`, a state chunk to carry them,
  and a waveform view beside the faceplate's table menu.
- **Analog oscillator aliases.** `process_analog` is a naive saw and a fixed
  50 % pulse; it needs PolyBLEP and a pulse width.

## Wavetable engine

In `WTbl` mode an oscillator scans one of ten factory tables (Basic, PWM,
Harm, Reso, Vowel, Sync, FM, Fold, Organ, Glass), 32 frames each, chosen from
the `Wavetable` menu in the oscillator card's header. Choosing a table while
the oscillator is in another mode switches it to `WTbl`.
`Shape` is the position in the table. `W.LFO` / `W.Env` route the LFO and the
mod envelope to the position, by the shared `W.Mod` amount (bipolar) — the
same arrangement as `P.LFO` / `P.Env` / `P.Mod` for pitch.

The tables are computed, not shipped (`plinken-dsp/src/wavetable_factory.rs`):
each is a recipe evaluated once per module instance, on the first
`activate`. Every frame is kept as a harmonic series and rendered once per
octave with the partials above Nyquist left out (`wavetable.rs`), so a
table does not alias at any pitch (FM and hard sync still can, as on the
analog engine). The bank takes about 5.5 MB of plugin memory.

A table is saved by its index in `FACTORY_TABLE_NAMES`: append tables, never
reorder them. The faceplate carries its own copy of the names; a cargo test
fails if the two lists differ.

## Build & test

```sh
cargo test -p com-plinken-synome         # includes an audibility test
pnpm --filter @plinken/synome build      # → dist/synome.wclap.tar.gz
```

To test in the wclap-host page, paste the bundle URL into the URL bar above
the shelf (or drop the file from disk) and play notes via MIDI / the host
keyboard.
