# Pulze

MPC-style 16-pad drum machine plugin for the Plinken WCLAP host.

**Status: sample playback landed.** Notes trigger pads through the CLAP
note port; pad samples arrive from the host as PLSP chunks over the
webview byte channel (`plinken-sample-core`'s `SampleAssembler`); per-pad
level / tune / pan / AD envelope / Moog filter / mute groups all work.
No UI yet — the pad grid lives app-side first (drop target for audio
assets); a `widgets/` UI for the standalone host comes later. A pad with
no sample delivered is silent.

Open source (MIT), like everything in this repo — see [`LICENSE`](./LICENSE).

## Architecture goal

A pad-grid drum instrument in the spirit of the Akai MPC:

- **Dynamic pads** — the pad set grows as needed, grouped in 4×4 banks
  (A/B/C/D…) rather than a fixed 16; one drum voice per pad
- **Bank switch above the pads** — the UI mirrors the Akai hardware: a
  pad-bank selector row (A/B/C/D…) sits above the 4×4 grid, and bank
  switching follows the MPC's note offsets — so a hardware MPC controller
  plugged in over MIDI plays Pulze directly, banks and all, with no
  remapping
- **Akai pad/sample structure** — each pad carries the same structure as an
  MPC program pad: up to 4 sample layers with velocity ranges, plus per-pad
  level, tune, pan, attack/decay, filter, and mute/choke group. Keeping the
  pad model field-compatible with Akai's makes **importing Akai patches**
  (MPC `.xpm` programs, and older `.pgm`) a straightforward field-for-field
  mapping instead of a translation layer.
- **Akai MPC note layout** — bank A pads 1–16 use the classic MPC mapping:

  | Row | Pads | MIDI notes |
  |-----|------|------------|
  | bottom | 1–4  | 37 36 42 82 |
  |        | 5–8  | 40 38 46 44 |
  |        | 9–12 | 48 47 45 43 |
  | top    | 13–16| 49 55 51 53 |

  (kick on pad 2 = 36, snare on pad 6 = 38, closed hat on pad 3 = 42,
  open hat on pad 7 = 46 — the layout every MPC finger-drummer knows)
- **Synthesized kits first** — kick (pitched sine sweep + click), snare
  (tone + filtered noise), closed/open hats (metallic noise bursts), toms,
  clap, percussion. No sample I/O needed, so the whole kit ships inside the
  wasm.
- **Per-pad params** — level, tune, decay — exposed via `ParamDef` so the
  host can automate them
- **Choke groups** — open hat is silenced by closed hat (MPC behaviour)
- **Pad-grid UI** (`widgets/`-based) with velocity-sensitive click-to-audition

Sample-based pads (user drops a one-shot on a pad, or imports an Akai
`.xpm`/`.pgm` program with its samples) are a later phase and need a
state/asset story; the synthesized kit doesn't wait for it, but the pad
model is Akai-shaped from day one so the import lands without a schema
change.

## Blocked on

Note delivery: `crates/wclap-plugin` declares the CLAP note port but does not
yet parse the process event queue (`clap_process.in_events`), so Rust plugins
can't receive note-on/off events. Adding the event iterator to the shared
crate unblocks both Pulze and Synome's DSP phase.

## Build

```sh
pnpm run build     # cargo build (wasm32) + bundle → dist/pulze.wclap.tar.gz
```

Not yet in the publish set (`plinken-api/scripts/publish-wclap-plugins.sh`
excludes silent scaffolds) — it joins the instrument picker next to
piano/organ once it makes sound.
