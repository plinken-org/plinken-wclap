# Vendor plugins

This is where authored **WCLAP** plugins live, contributed by vendors.

`wclap-host` (at `apps/wclap-host`) hosts these plugins in the browser; this
directory is the source of truth for the bundles we ship on the shelf at
[wclap.plinken.org](https://wclap.plinken.org).

## Directory shape

```
plugins/
├── <reverse-dns-vendor>/         # e.g. com.plinken, com.example
│   ├── README.md                 # who you are, contact, licensing
│   └── <plugin-name>/            # one folder per authored plugin
│       ├── plugin.json           # plugin manifest (see below)
│       ├── package.json          # if it's a pnpm/npm build (AS, JS tooling)
│       ├── README.md             # what the plugin does
│       ├── LICENSE               # MIT or Apache-2.0, required
│       ├── assembly/ | src/      # plugin sources (Rust, C++, AS, …)
│       └── dist/                 # built artifact, ignored by git;
│                                 # ships at apps/wclap-host/public/samples/
```

Vendor folders use **reverse-DNS naming** (Java package style). The directory
name maps to the vendor's domain backwards — `com.plinken`, `com.signalsmith`,
`io.example`. Dots in directory names are fine on every supported OS, in git,
and in pnpm globs.

## Plugin manifest (`plugin.json`)

Every plugin folder ships a small `plugin.json` describing the bundle. The
fields mirror what the CLAP descriptor exposes at runtime, plus build/ship
hints. This file is the contract that a future **MCP-driven registry** will
read to expose plugins to AI agents and to auto-update the host's shelf
from PRs.

```json
{
  "id": "com.plinken.auto-pan",
  "name": "Auto-Pan",
  "vendor": "Plinken",
  "version": "0.1.0",
  "description": "Stereo auto-panner with sine LFO.",
  "features": ["audio-effect", "utility"],
  "license": "MIT",
  "homepage": "https://plinken.org",
  "source": "assembly/",
  "artifact": "dist/auto-pan.wclap.wasm",
  "format": "wasm"
}
```

`format` is either `"wasm"` (bare WCLAP wasm — no webview UI assets) or
`"tar.gz"` (a `.wclap.tar.gz` bundle that ships HTML and resources for the
plugin GUI).

## How to contribute a plugin

1. Fork [`plinken-org/plinken-wclap`](https://github.com/plinken-org/plinken-wclap).
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
under `apps/wclap-host/public/samples/`. Until the aggregator script lands,
shipping a plugin to the live shelf takes two manual steps in the same PR:

1. Copy your built artifact into the host's samples directory using the
   plugin id as the filename:
   - `dist/<plugin>.wclap.wasm` → `apps/wclap-host/public/samples/<id>.wclap.wasm`
   - `dist/<plugin>.wclap.tar.gz` → `apps/wclap-host/public/samples/<id>.wclap.tar.gz`
2. Add an entry to the `SHELF` array in `apps/wclap-host/src/main.ts`
   pointing at `/samples/<id>.<ext>`.

### Coming: MCP-driven plugin registry

The next step is an aggregator that walks `plugins/*/*/plugin.json`,
collects manifests + built artifacts, and:

- writes a single `shelf.json` consumed at runtime by `wclap-host`
  (replacing the hard-coded `SHELF` constant), and
- exposes the same manifest via an **MCP server** so AI agents (and other
  hosts) can discover and load community plugins programmatically.

With that in place, merging a plugin PR is enough — no host code edit.
That's also why we want every plugin folder to carry a complete
`plugin.json` from day one.

## License of this directory

The `plugins/` directory is part of `plinken-org`, which is MIT-licensed.
Each vendor's plugin keeps its own `LICENSE` file — we don't relicense your
work. We only ship plugins whose license is MIT- or Apache-2.0-compatible
for the public shelf.

## Public repo — no secrets

This repo is public. **Do not commit** API keys, signing keys, private
endpoints, or anything else you wouldn't paste in a chat to a stranger.
See the top-level [CLAUDE.md](../CLAUDE.md) for the full secrets policy.
