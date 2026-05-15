# com.plinken.spectrum

Real-time stereo spectrum analyzer. Audio passes through unchanged.

- **DSP path:** `fundsp::hacker32::multipass::<U2>()` — stereo passthrough.
- **Analysis tap:** mono-sum (L+R)/2 → 1024-sample Hann-windowed ring →
  `fundsp::fft::real_fft` → 64 log-spaced magnitude bands between 20 Hz
  and Nyquist → CBOR `{ "spec": <byte string of f32 BE dB> }` pushed to
  the iframe via `clap_host_webview.send` at ~30 Hz.
- **UI:** canvas bar graph, peak-hold attack + exponential release driven
  by the `Smooth` param. `Floor` controls the bottom of the dB scale.

## Build

```sh
pnpm --filter @plinken/spectrum build
```

Produces `dist/spectrum.wclap.tar.gz`. Run `node scripts/build-shelf.mjs`
from the repo root to mirror it into both shelf consumers.
