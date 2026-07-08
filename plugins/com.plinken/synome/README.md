# Synome

Moog-style polyphonic synthesizer plugin for the Plinken WCLAP host.

**Status: DSP ported — makes sound.** Notes arrive through the CLAP note
port (wclap-plugin event-queue hooks), params through `clap.params` /
webview, knob state persists via `clap.state`. No UI yet (`has_ui: false`);
next phases add the sample oscillator mode and a `widgets/` UI.

## What's inside

- 2 oscillators with saw/pulse morphing, FM between them, hard sync
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

- **Oscillator engines — implement WTbl / Gran / Phys.** The OSC Mode pot
  offers five engines (`Anlg` / `WTbl` / `Gran` / `Phys` / `Smpl`) but only
  **Analog** and **Sample** actually work today. Two gaps:
  1. `crates/plinken-dsp/src/osc.rs` — `Oscillator::process` matches on mode,
     but `Wavetable`, `Granular` and `Physical` all fall through to
     `process_analog` (marked `// TODO`). Implement real wavetable scan,
     granular cloud, and Karplus-Strong physical modelling.
  2. `src/synth.rs` — the per-voice oscillator's `set_mode` is **never
     called** (only `== OscMode::Sample` is tested, lines ~376/401/421), so
     even once the DSP exists the mode must be pushed to `voice.osc1` /
     `voice.osc2` each block for non-Sample modes.

  Until then, selecting WTbl/Gran/Phys sounds identical to Analog. Options if
  we ship before implementing: hide those positions from the Mode pot
  (`ui/index.html` `OSC_MODES` + pot `max`) so the UI doesn't advertise
  engines that don't exist.

## Build & test

```sh
cargo test -p com-plinken-synome         # includes an audibility test
pnpm --filter @plinken/synome build      # → dist/synome.wclap.tar.gz
```

To test in the wclap-host page, paste the bundle URL into the URL bar above
the shelf (or drop the file from disk) and play notes via MIDI / the host
keyboard.
