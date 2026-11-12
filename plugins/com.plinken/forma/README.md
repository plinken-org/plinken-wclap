# FORMA

Two-slot synthesizer for the Plinken WCLAP host. Each slot plays its own
source sample through one of two engines, and the two slots are blended.

**Status: playable.** Both engines, the mix, sample import from the
plugin's own UI, state persistence and the UI (expanded + compact) work
and are verified in `apps/wclap-host`. See [Not yet built](#not-yet-built).

## Layout

```
 slot A (left)                               slot B (right)
 source → Grain | Spectral → mid/side   ┐  ┌  source → Grain | Spectral → mid/side
                                        ├──┤
                              Fade or Ring (per voice)
                                        │
                          amp envelope → filter → volume
```

- **Either engine in either slot.** Default: Grain in A, Spectral in B.
- **Mix:** `Fade` crossfades A ↔ B (equal power). `Ring` goes A alone →
  ring product A×B at the centre → B alone.
- **Placement is mid/side.** Per slot, `Width` scales everything stereo
  the slot makes (0 = one point, 2 = doubled), `Balance` says where that
  point sits, and `Spread` is how much stereo there is to scale: per-grain
  pan, left-right detune, and a fan that puts successive notes on
  alternating sides. Default: A left, B right (±60 %), both at full width,
  so the two slots open the field between them.
- **Output:** a zero-latency peak limiter (ceiling ≈ −1 dBFS, 25 ms hold,
  120 ms release, channels linked) follows the filter, so a resonant peak
  or a big chord is turned down instead of clipped. The panel's meter
  turns amber while it works.
- 12 voices; retrigger → free → oldest released → oldest.

## Grain engine (`src/grain.rs`)

Grains are spawned by a phase accumulator at the grain rate. With
`Rate follows key` on, the rate is the played note's frequency (offset by
`Rate`, in octaves), so the source snippet becomes the waveform of an
oscillator: **rate sets the pitch, Size and Pitch set the timbre**. Onsets
are sub-sample accurate. With key tracking off the rate is relative to C3
and reaches down to slow, audible grains.

For a source with a clear pitch, grains start a whole number of source
cycles apart, so neighbouring grains line up in phase: moving `Position` or
`Scan` then changes the timbre smoothly instead of roughening it.

`Size` (grain length in grain periods), `Shape` (window roundness),
`Pitch` (grain playback speed), `Scatter` (random read position),
`Jitter` (random timing — turns a tone into a cloud), `Pitch follows key`
(grain speed tracks the note relative to the source's detected root).

## Spectral engine (`src/analysis.rs`, `src/spectral.rs`)

On import the source is analysed into **four static sounds, each with a
gain envelope over the length of the source**:

1. STFT (2048, Hann) → 64-band magnitude spectrogram, at most 1024 frames.
2. Rank-4 non-negative matrix factorisation (Euclidean, 60 iterations,
   fixed seed — the same source always gives the same model).
3. A second STFT pass splits each full-resolution frame between the layers
   by the band model's share, giving each layer its own spectrum.
4. Pitch detection (autocorrelation on the loudest frame). Each layer's
   spectrum is sampled at the harmonics of that root and stored as
   band-limited single-cycle wavetables (9 mip levels), all with the same
   partial phases so layers mix as spectra.

Playback reads the four tables at one phase and mixes them by the
envelopes at the current `Position`. Controls: `Pure` (crossfade to tables
with only the strongest partials), `Warp` (phase distortion — adds
overtones), `FM` + `Ratio` (phase modulation), `Smooth` (averages out the
envelopes' fast movement), `Tilt` (fades out the brightest or the darkest
layers).

The analysis runs **inside `process`, one small unit per 128 frames**, so
it never blocks the audio thread: a 20 s source takes about 2200 units
(≈ 6 s at 48 kHz); the worst single unit measured 0.5 ms native. The Grain
engine plays a new source immediately; a Spectral slot is silent until its
model lands.

`Position` glides to a new value over about 60 ms (a knob arrives as a
stream of small jumps), and both engines even out the source's own
loudness along its length. `Position` and `Scan` are shared by both engines, so switching a slot's
engine keeps its place in the source.

## Sources

- A fresh instance plays built-in factory sources, so it sounds before
  anything is imported.
- **Import:** drop an audio file on a slot's ring (or `Load`). The UI
  decodes it, mixes to mono, truncates to 20 s and sends it as PLSP chunks
  (`crates/sample-core`), slot 0 = A, 1 = B. `Reset` sends a clear.
- **Persistence:** imported sources are saved in the plugin state
  (`save_extra_state`, magic `FRM2`: per slot a flag, then sample rate,
  frame count and 16-bit PCM), so a project reloads in any host without the
  original file. The model is re-derived on load. A 20 s source adds
  ≈ 1.9 MB to the state. (`FRM1`, 32-bit float, is still read.)

## UI ↔ plugin messages

Standard `widgets/transport.mjs` for params, plus:

| Direction | Message | Meaning |
|---|---|---|
| UI → plugin | CBOR `{"smp": bytes}` | PLSP sample chunk |
| UI → plugin | CBOR text `"forma?"` | send the slot overviews again |
| plugin → UI | CBOR `{"forma": bytes}`, first byte `1` | slot overview: slot, status, progress, flags, root Hz, seconds, 128-point level curve, 4 × 128 layer envelopes |
| plugin → UI | CBOR `{"forma": bytes}`, first byte `2` | 20 Hz tick: playheads, voice count, analysis progress |

### Generated sources (host extension)

A host that can create audio from a text prompt can offer it inside FORMA.
The plugin itself is unchanged: generated audio enters through the same
import path as a dropped file. The messages are plain-object
`postMessage`s between the UI iframe and its parent, in the `plinkenHost`
family (they are not forwarded to the wasm):

1. UI → host on mount: `{ plinkenHost: 'capabilities' }`.
2. Host → UI: `{ plinkenHost: 'capabilities', generateSource: true,
   plinsPerSecond }` — only then does each slot show a prompt field.
   `plinsPerSecond` (number or `null`) is the host's price; the button
   shows the cost of the chosen length before the click.
3. UI → host: `{ plinkenHost: 'generateSource', slot, prompt, seconds }`.
4. Host → UI: `{ plinkenHost: 'sourceGenerated', slot, ok: true, audio }`
   with `audio` an `ArrayBuffer` of an encoded audio file, or
   `{ …, ok: false, error }` with a message the UI shows in the slot.

The Plinken app implements this; `apps/wclap-host` does not, so the
prompt field stays hidden there.

## Not yet built

- Grainferno-style per-grain filter, compressor and feedback blur.
- Modulation (LFOs, a second envelope, MPE) and an FX section.
- Layer spectra are sampled at harmonics of one root: an inharmonic or
  polyphonic source is approximated by a harmonic one.
- Warp and FM are not oversampled; high notes alias at high settings.
- NKS metadata and factory presets.

## Build & test

```sh
cargo test -p com-plinken-forma
cargo test -p com-plinken-forma --release -- --ignored --nocapture   # block costs
pnpm --filter @plinken/forma build        # → dist/forma.wclap.tar.gz
```

Then load it from the shelf in `apps/wclap-host` and play it over MIDI.
