# Organ

Hammond-style additive tonewheel organ — the first **Cmajor**-authored
plugin in `com.plinken` and the proving ground for the
`cmaj → WASI-SDK → .wclap` pipeline.

Nine drawbars at the classic Hammond ratios, summed per voice; eight-voice
polyphony; per-voice attack/release. No vibrato/chorus/key-click yet — the
point of v0 is "drops onto the timeline and sounds like an organ", not
shipping a B3 clone.

| Drawbar | Harmonic | Ratio | Default |
|--------:|---------:|------:|--------:|
| 16'     | sub      | 0.5×  | 8       |
| 5 1/3'  | 3rd of sub | 1.5× | 8     |
| 8'      | fund.    | 1.0×  | 8       |
| 4'      | 2nd      | 2.0×  | 0       |
| 2 2/3'  | 3rd      | 3.0×  | 0       |
| 2'      | 4th      | 4.0×  | 0       |
| 1 3/5'  | 5th      | 5.0×  | 0       |
| 1 1/3'  | 6th      | 6.0×  | 0       |
| 1'      | 8th      | 8.0×  | 0       |

Default voicing `888 000 000` — full bottom three bars, everything else off
— is the canonical "jazz organ" registration; pull more bars to get the
brighter rock / gospel registrations.

## Source layout

```
organ/
├── Organ.cmajor          # DSP — graph + voice + harmonic-frequency processor
├── Organ.cmajorpatch     # patch manifest cmaj reads
├── plugin.json           # WCLAP / shelf manifest (Plinken)
├── package.json          # pnpm scripts (build:wasm, build)
└── README.md
```

`dist/` and `generated/` are gitignored — they hold the cmaj C++ output and
the final `module.wasm` / `.wclap.tar.gz`.

## Build

Two toolchains required, both one-time setups:

```sh
# 1. Cmajor CLI
#    https://github.com/cmajor-lang/cmajor/releases  → put `cmaj` on $PATH

# 2. WASI-SDK
#    https://github.com/WebAssembly/wasi-sdk/releases
#    Unpack to /opt/wasi-sdk, or export WASI_SDK=/path/to/it

# 3. CLAP headers (small — vendored)
git submodule add https://github.com/free-audio/clap vendor/clap
```

Then:

```sh
pnpm --filter @plinken/organ build
```

What happens:

1. `scripts/build-cmaj-wclap.sh` runs `cmaj generate --target=clap` →
   `generated/clap/*.cpp` (self-contained CLAP C++).
2. The same script invokes `${WASI_SDK}/bin/clang++ --target=wasm32-wasi`
   with `-Wl,--export=clap_entry --export-table --growable-table` → emits
   `dist/organ.wclap.wasm`.
3. `scripts/bundle-wclap.mjs` wraps the wasm + `plugin.json` into
   `dist/organ.wclap.tar.gz` (POSIX ustar + gzip — same bundle layout the
   Rust plugins ship).
4. `scripts/build-shelf.mjs` (already auto-discovers `plugins/*/*/plugin.json`)
   picks the tarball up on the next host build and copies it into
   `apps/wclap-host/public/samples/` + `apps/site/static/wclap/`.

## Why Cmajor

Same reason Tracktion ships it: DSP authors write the signal flow in a
language designed for DSP (clocked endpoints, statically-checked graphs,
SIMD-friendly value types) instead of fighting C++ template error messages
in a CLAP boilerplate. The generated C++ is a self-contained
zero-dependency project — no JIT, no Cmajor runtime — which makes it a
clean target for WASI-SDK.

Once the pipeline works for `Organ`, *any* `.cmajorpatch` in the Cmajor
examples folder (Pro54, TX81Z, ElectricPiano, Freeverb, …) drops into
`plugins/<vendor>/<name>/` with a copy-paste `package.json` and lands on
the shelf with no further work.

## License

[MIT](../../../LICENSE) — same as the rest of `com.plinken`.
