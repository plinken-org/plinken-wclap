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

### Core mixing FX

The everyday channel-strip effects, all hand-rolled Rust DSP on the shared
`crates/wclap-plugin` scaffold, with `widgets/`-based UIs:

| Plugin         | Category    | Notes |
|----------------|-------------|-------|
| `eq`           | EQ          | 4-band parametric (low shelf, 2 peaks, high shelf), RBJ biquads |
| `compressor`   | Dynamics    | Feed-forward, soft knee, peak detector, GR meter |
| `delay`        | Delay       | Stereo, feedback-path tone, ping-pong, tape-style time glide |
| `reverb`       | Reverb      | Freeverb (8 combs + 4 allpass / ch), pre-delay, width |
| `chorus`       | Modulation  | Up to 3 LFO voices, stereo spread |
| `saturator`    | Distortion  | tanh / tube / fold waveshaper, tone + dry/wet, DC-blocked |

See [`ROADMAP.md`](./ROADMAP.md) for the build pattern and status.

## Contact

- Web: [plinken.com](https://plinken.com)
- Open-source side: [plinken.org](https://plinken.org)
- Source: [github.com/taluvi-dev/plinken-org](https://github.com/taluvi-dev/plinken-org)

## License

Plinken's plugins shipped here are released under **MIT** unless a
specific plugin's `LICENSE` file says otherwise.
