# com.plinken

Plugins authored by **Plinken** ([plinken.com](https://plinken.com) /
[plinken.org](https://plinken.org)) — the first vendor folder in
`plugins/`.

## Plugins

| Plugin            | Authored in | Notes |
|-------------------|-------------|-------|
| `auto-pan`        | AssemblyScript | LFO panner |
| `spectrum`        | Rust (fundsp)  | FFT analyser, UI-only |
| `synome`          | Rust           | Polysynth scaffold |
| `vocal-limiter`   | Rust (fundsp)  | Lookahead brickwall |
| `organ`           | **Cmajor**     | Hammond drawbar synth — first user of the `cmaj → WASI-SDK → .wclap` pipeline (`scripts/build-cmaj-wclap.sh`) |
| `piano`           | **Cmajor**     | Additive piano w/ stretched tuning — second pipeline user, pure-synthesis variant (no externals) |

## Roadmap

See [`ROADMAP.md`](./ROADMAP.md) for the next batch — **core mixing FX** that fill
the DAW picker's empty category buckets, all Rust (fundsp):

- `eq` — Parametric EQ
- `compressor` — Compressor
- `delay` — Delay / echo
- `reverb` — Algorithmic reverb
- `chorus` — Chorus
- `saturator` — Saturation / distortion

## Contact

- Web: [plinken.com](https://plinken.com)
- Open-source side: [plinken.org](https://plinken.org)
- Source: [github.com/taluvi-dev/plinken-org](https://github.com/taluvi-dev/plinken-org)

## License

Plinken's plugins shipped here are released under **MIT** unless a
specific plugin's `LICENSE` file says otherwise.
