# Vendor plugins

This is where authored **WCLAP** plugins live, contributed by vendors.

`wclap-host` (at `apps/wclap-host`) hosts these plugins in the browser; this
directory is the source of truth for the bundles we ship on the shelf at
[wclap.plinken.org](https://wclap.plinken.org).

## Directory shape

```
plugins/
├── <reverse-dns-vendor>/        # e.g. com.plinken, com.example
│   ├── README.md                # who you are, contact, licensing
│   └── <plugin-name>/           # one folder per authored plugin
│       ├── README.md            # what the plugin does
│       ├── LICENSE              # MIT or Apache-2.0, required
│       ├── src/                 # plugin sources (Rust, C++, AS, …)
│       └── dist/                # built artifact: <plugin>.wclap.tar.gz
│                                # or bare <plugin>.wasm
```

Vendor folders use **reverse-DNS naming** (Java package style). The directory
name maps to the vendor's domain backwards — `com.plinken`, `com.signalsmith`,
`io.example`. Dots in directory names are fine on every supported OS, in git,
and in pnpm globs.

## How to contribute a plugin

1. Fork [`taluvi-dev/plinken-org`](https://github.com/taluvi-dev/plinken-org).
2. Pick (or create) your vendor folder under `plugins/<your-reverse-dns>/`. If
   you haven't published here before, add a `README.md` in your vendor folder
   (name, contact, license preference).
3. Add your plugin under `plugins/<your-vendor>/<plugin-name>/`:
   - `README.md` describing the plugin
   - `LICENSE` — MIT or Apache-2.0
   - `src/` — sources, if you want them in this repo (optional but
     appreciated; lets others learn from your build)
   - `dist/` — the built artifact:
     - `<plugin>.wclap.tar.gz` for bundles with HTML UI assets, or
     - `<plugin>.wasm` for bare-wasm plugins
4. Open a **pull request** against `main`. Keep the PR scoped to your folder.
5. The PR will be reviewed for: license compatibility, no secrets in the
   bundle, builds load cleanly in `wclap-host`. We're not gatekeeping
   musical taste.

## What gets shipped on the shelf

The shelf chips in `apps/wclap-host/src/main.ts` reference plugin bundles
under `apps/wclap-host/public/samples/`. To get your plugin on the live
shelf, your PR can additionally:

- Copy your `dist/<plugin>.wclap.tar.gz` (or `.wasm`) into
  `apps/wclap-host/public/samples/<vendor>-<plugin>.<ext>`
- Add a new entry to the `SHELF` array in `apps/wclap-host/src/main.ts`

Or just publish the plugin here and we'll wire the shelf entry in a
follow-up.

## License of this directory

The `plugins/` directory is part of `plinken-org`, which is MIT-licensed.
Each vendor's plugin keeps its own `LICENSE` file — we don't relicense your
work. We only ship plugins whose license is MIT- or Apache-2.0-compatible
for the public shelf.

## Public repo — no secrets

This repo is public. **Do not commit** API keys, signing keys, private
endpoints, or anything else you wouldn't paste in a chat to a stranger.
See the top-level [CLAUDE.md](../CLAUDE.md) for the full secrets policy.
