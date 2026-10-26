# Spiral303

A Roland TB-303 emulation: a monophonic bassline synthesizer with a built-in
step sequencer, in one WCLAP bundle.

## What is in it

- **Oscillator** — one wave shape that morphs sine → saw → square, with an
  analogue-style drift (`Warmth`) on pitch and cutoff.
- **Filter** — the TB-303 ladder with envelope on the cutoff, plus six other
  models to choose from: 12/24 dB slopes, zero-delay variants, a vowel
  formant filter, and a Moog ladder. Accent drives the envelope, not just
  the volume.
- **Distortion** — tanh, triode, wavefolder, five tape models, clean and
  crunch; type changes crossfade so they do not click.
- **Sequencer** — 4..32 steps, 12 scales, ties, slides, accents, per-step
  velocity, rotation and a 16-sector **gate wheel** that shapes note length,
  swing and clock division.
- **Interface** — the whole editor is drawn on a canvas at a fixed 1180×780
  design and scaled to the window: scope with the filter's response curve
  (draggable, it is also a control), the step grid, the gate wheel, a preset
  field with a 102-preset factory bank, MIDI import and WAV export.

## Using it

The plugin produces no sound on its own: it needs MIDI. In a host with MIDI,
turn `SEQ` on and the pattern plays from a keyboard note; with `SEQ` off the
incoming notes run through the same filter and distortion.

`Sync = Host` follows the transport; otherwise set the BPM by hand.

## Building

The bundle in `dist/` is produced by the source repository:

```sh
git clone https://github.com/Spiralfractalplugins/spiral303
cd spiral303
tools/build_wclap.sh        # → dist/Spiral303-<version>-wclap-registry.tar.gz
```

The registry artifact is packed **flat** (`module.wasm`, `index.html`,
`app.js`, `style.css` at the root of the tarball) because that is what this
repo's browser host resolves. The openDAW build of the same module keeps a
`Spiral303.wclap/` top directory instead, so the two are not
interchangeable.

## License

Apache-2.0 — see [LICENSE](LICENSE). See also the
[vendor README](../README.md) for why the wasm bundle is permissive while
the native build of the same code is GPL-3.0.
