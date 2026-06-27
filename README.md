<div align="center">

# PLINKEN WCLAP

### OPEN WCLAP HOST &amp; PLUGINS

The community / open-source side of **[Plinken](https://plinken.com)**.<br>
We're implementing **[WebCLAP](https://github.com/WebCLAP)** in the open — a browser host and a handful of authored plugins (vocoder &amp; friends), running as CLAP modules compiled to `wasm32`.

[**plinken.org**](https://plinken.org) · [plinken.com](https://plinken.com) · [WebCLAP](https://github.com/WebCLAP)

![status](https://img.shields.io/badge/status-PoC%20%C2%B7%20WIP-925db3?style=flat-square)
![license](https://img.shields.io/badge/license-MIT-6773be?style=flat-square)
![public](https://img.shields.io/badge/repo-public-4ea3a5?style=flat-square)

</div>

---

## WHY THIS EXISTS

[**Plinken**](https://plinken.com) is a collaborative video / audio / 3D / MIDI editor with AI built in. It needs to host audio plugins in the browser — and we think the right way to do that is via **WCLAP**: the CLAP plugin format compiled to `wasm32`, designed for the web.

**`plinken.org`** is where that work happens in public. Anything in this repo is:

- a **proof-of-concept host** for WCLAP plugins in the browser,
- a **set of reference WCLAP plugins** (starting with a vocoder),
- and **scaffolding** so others can build on top.

The upstream project is [github.com/WebCLAP](https://github.com/WebCLAP) — we contribute upstream where it makes sense, and ship the rest here.

## REPO LAYOUT

```
plinken-org/
├── apps/
│   ├── site/              # plinken.org — SvelteKit 5 (runes) on Cloudflare Workers
│   └── wclap-host/        # WCLAP browser host PoC (Vite + TS)
├── crates/                # placeholder — future Rust packages
├── plugins/               # placeholder — authored WCLAPs (vocoder, …)
├── vendor/
│   └── wclap-host-js/     # git submodule → WebCLAP/wclap-host-js
├── pnpm-workspace.yaml
├── turbo.json
├── Cargo.toml             # workspace root for crates/ (unused at v1)
└── CLAUDE.md
```

## STACK

| Layer       | Tech                                                        |
|-------------|-------------------------------------------------------------|
| Site        | SvelteKit · Svelte 5 (runes) · TypeScript · Vite            |
| Host        | TypeScript · WebAudio · AudioWorklet · `wclap-host-js`      |
| Plugins     | C++ / AssemblyScript → CLAP → `wasm32`                      |
| Build       | pnpm workspaces · Turborepo · Cargo workspace               |
| Hosting     | Cloudflare Workers (`@sveltejs/adapter-cloudflare`)         |
| Upstream    | [WebCLAP](https://github.com/WebCLAP) (host JS / bridge / examples) |

## GETTING STARTED

```sh
git clone --recurse-submodules git@github.com:plinken-org/plinken-wclap.git
cd plinken-wclap
pnpm install
```

Run the marketing site locally:

```sh
pnpm --filter @plinken/site dev          # Vite dev server
pnpm --filter @plinken/site preview      # wrangler dev — closer to prod
pnpm --filter @plinken/site deploy       # build + ship to Cloudflare Workers
```

The WCLAP host PoC (`apps/wclap-host/`) is scaffolded but empty pending the first vendor wire-up.

## :warning: SECRETS POLICY — PUBLIC REPO

**This repository is public.** Anything pushed here is world-readable forever — git history rewrites are damage control, not a fix.

> **Never commit secrets.** That includes `.env`, `.dev.vars`, API tokens, Cloudflare Worker secrets, account IDs you consider sensitive, signing keys, private certs, DB URLs with creds, or OAuth client secrets.

Where credentials *do* belong:

- **Cloudflare Workers** → `wrangler secret put <NAME>` or the Cloudflare dashboard. Never inline in `wrangler.jsonc`.
- **Local dev** → `.dev.vars` (Workers) or `.env.local` (Vite). Both gitignored.
- **CI** → GitHub Actions secrets / environment secrets.
- **Templates** → `.example` / `.sample` suffixed files with placeholders only.

Sanity-check before pushing:

```sh
git diff --cached | grep -iE 'secret|token|api[_-]?key|password|bearer|private[_-]?key'
```

If a credential ever lands here: **rotate it first**, scrub history second.

Full policy lives in [CLAUDE.md](./CLAUDE.md).

## STATUS

- [x] Monorepo scaffold (pnpm · Turbo · Cargo)
- [x] `apps/site/` — plinken.org on SvelteKit 5 + Cloudflare Workers
- [ ] `apps/wclap-host/` — browser WCLAP host PoC
- [ ] `vendor/wclap-host-js` — wire submodule
- [ ] `plugins/vocoder` — first authored WCLAP
- [ ] Custom domains (`plinken.org`, `www.plinken.org`) on the Worker

## RELATED

- **[plinken.com](https://plinken.com)** — the commercial product (Taluvi)
- **[github.com/WebCLAP](https://github.com/WebCLAP)** — the WebCLAP project (upstream)
  - [`wclap-host-js`](https://github.com/WebCLAP/wclap-host-js) — JS host library
  - [`browser-test-host`](https://github.com/WebCLAP/browser-test-host) — reference browser host
  - [`wclap-bridge`](https://github.com/WebCLAP/wclap-bridge) — CLAP ↔ wasm bridge
  - [`as-clap`](https://github.com/WebCLAP/as-clap) — author WCLAPs in AssemblyScript
  - [`examples`](https://github.com/WebCLAP/examples) — sample plugins

## LICENSE

[MIT](./LICENSE) — see file for details.

<div align="center">

<sub>built alongside <a href="https://plinken.com"><b>plinken.com</b></a> · upstream <a href="https://github.com/WebCLAP"><b>WebCLAP</b></a> · © 2026</sub>

</div>
