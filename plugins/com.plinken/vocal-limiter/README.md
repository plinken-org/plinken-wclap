# Vocal Limiter

Lookahead peak limiter tuned for vocals. Part of the Plinken vocal trio
(limiter / compressor / EQ) — built on Rust + `fundsp` + the shared
[`wclap-plugin`](../../../crates/wclap-plugin) scaffold.

## MVP scope

Defaults are hardcoded; `clap.params` lands once the extension is added
to the shared scaffold (then ceiling/attack/release/drive become
automatable).

| Setting | Value |
|---|---|
| Ceiling | −1 dBFS |
| Attack | 5 ms |
| Release | 50 ms |
| Stereo link | Yes (handled by `fundsp::limiter_stereo`) |

## Build

```sh
pnpm --filter @plinken/vocal-limiter install
pnpm --filter @plinken/vocal-limiter build
```

Output: `dist/vocal-limiter.wclap.tar.gz`.

## Shipping to the wclap-host shelf

Copy the tarball into
`apps/wclap-host/public/samples/com.plinken.vocal-limiter.wclap.tar.gz`
and add a matching entry to `apps/wclap-host/public/shelf.json`.

## License

[MIT](./LICENSE).
