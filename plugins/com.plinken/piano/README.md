# Piano

Additive piano with stretched (slightly inharmonic) tuning. Six sine
partials per voice, one per-voice decay envelope whose time constant
scales with note pitch (low notes ring for seconds, high notes die in
fractions of one), 8-voice polyphony, 200 ms release on NoteOff. Second
user of the `cmaj → WASI-SDK → .wclap` pipeline alongside the
[`organ`](../organ/).

Not aiming for "Steinway in your browser". Aiming for "drops on the
timeline, plays a chord, sounds piano-shaped enough to keep working".

## Why authored here instead of vendoring upstream

The official Cmajor [`Piano`](https://github.com/cmajor-lang/cmajor/tree/main/examples/patches/Piano)
example is a sampled piano — five `.ogg` files pitch-shifted across the
keyboard — and it's dual-licensed GPLv3 / commercial. Vendoring it would
either drag GPL onto every downstream Plinken build or force a per-plugin
license override in this otherwise-MIT repo. Cheaper to author from
scratch in Cmajor, sounds *worse* but stays MIT, no upstream-drift
maintenance burden.

## How the synthesis works

```
MIDI → MPEConverter → VoiceAllocator(8) → Voice[8] → out
```

Each voice:

```
NoteOn ──┬── PianoHarmonics ──┬── f1 → osc1
         │                    ├── f2 → osc2
         │                    ├── ...
         │                    └── f6 → osc6
         │
         └── PianoEnvelope ──── gainOut

(envelope * Σ osc_n × weight_n) → out
```

Two pieces do the work:

**`PianoHarmonics`** — stretched-octave tuning. Each partial's frequency
ratio is `n × √(1 + B·n²)` with `B = 4×10⁻⁴` (Railsback-ish constant for
middle-of-the-keyboard piano strings). Pure integer harmonics sound
organ-y; this small detuning gives the partials the slight beating that
makes ears hear "piano".

**`PianoEnvelope`** — one-pole exponential decay, time constant
`6 × 0.5^((pitch - 36)/24)` seconds:

| MIDI | Note | Decay |
|-----:|:-----|------:|
| 21   | A0   | ~12 s |
| 36   | C2   | 6 s   |
| 60   | C4   | 2.1 s |
| 84   | C6   | 0.75 s |
| 108  | C8   | 0.27 s |

NoteOff doesn't kill the note — it switches to a 200 ms one-pole release
(damper landing), which only matters if the key comes up while the
string is still ringing audibly.

Partial weights `[1.0, 0.6, 0.4, 0.25, 0.15, 0.08]` are hand-tuned for a
fundamental-heavy spectrum — close to what a struck-string analysis
gives you.

## Source layout

```
piano/
├── Piano.cmajor          # graph + Voice + PianoHarmonics + PianoEnvelope
├── Piano.cmajorpatch     # patch manifest cmaj reads
├── plugin.json           # WCLAP / shelf manifest
├── package.json          # pnpm scripts
└── README.md
```

No samples, no externals — pure synthesis, same build pipeline as the
organ (`scripts/build-cmaj-wclap.sh`).

## Build

```sh
pnpm --filter @plinken/piano build
```

Toolchain assumptions (cmaj on `$PATH`, `WASI_SDK`, `vendor/clap`) are
identical to the organ — see [`com.plinken/organ/README.md`](../organ/README.md#build).

## License

[MIT](../../../LICENSE).
