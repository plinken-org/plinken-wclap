# Auto-Pan

Stereo auto-panner WCLAP plugin authored by Plinken — the first plugin
under `plugins/com.plinken/`.

## What it does

Two parameters drive a sine LFO that sweeps the input signal across the
stereo field using equal-power panning.

| Param | Range | Default | Notes |
|---|---|---|---|
| Speed | 0.1 – 20 Hz | **5 Hz** | LFO rate |
| Wet/Dry | 0 – 1 | **1** | Linear mix; at 0 input passes through untouched |

At the defaults you get a clearly audible auto-pan at ~5 Hz with full effect.

## Build

```sh
pnpm --filter @plinken/auto-pan install
pnpm --filter @plinken/auto-pan build
```

Output lands in `dist/auto-pan.wclap.wasm` (release) and
`dist/auto-pan.debug.wclap.wasm` (debug).

Built on:
- [`as-clap`](https://github.com/WebCLAP/as-clap) — AssemblyScript bindings
  to the CLAP plugin API
- [`@assemblyscript/wasi-shim`](https://github.com/AssemblyScript/wasi-shim)

## Shipping to the shelf

The release build at `dist/auto-pan.wclap.wasm` is copied to
`apps/wclap-host/public/samples/com.plinken.auto-pan.wclap.wasm` and
referenced from the `SHELF` array in `apps/wclap-host/src/main.ts`.
Future work: an aggregator script + manifest registry so the shelf is
generated from each plugin's `plugin.json`.

## License

[MIT](./LICENSE) — see file for the full text.
