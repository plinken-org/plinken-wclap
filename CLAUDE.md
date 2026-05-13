# plinken-org

Public community monorepo for Plinken projects.

## Repo layout

```
plinken-org/
├── apps/
│   ├── site/              # plinken.org — SvelteKit 5 (runes) on Cloudflare Workers
│   └── wclap-host/        # WCLAP proof-of-concept (Vite + TS)
├── crates/                # future Rust packages
├── plugins/               # future authored WCLAPs (vocoder, etc.)
├── vendor/
│   └── wclap-host-js/     # git submodule
├── pnpm-workspace.yaml
├── turbo.json
├── Cargo.toml             # workspace root for crates/ (unused at v1)
└── CLAUDE.md              # this file
```

## ⚠️ Secrets policy — this repo is PUBLIC

**Never commit secrets, credentials, or sensitive identifiers.** Anything pushed here is world-readable forever (git history is permanent — rewriting it after a leak is not a fix).

Do **not** commit:

- `.env`, `.env.*`, `.dev.vars` (or any variant)
- API keys, tokens, signing keys, webhook secrets
- Cloudflare account IDs, Worker secrets, R2/KV credentials
- Database connection strings with credentials
- Private keys, certificates, `.pem` / `.key` files
- Personal access tokens (GitHub, npm, etc.)
- OAuth client secrets

Where credentials *do* belong:

- **Cloudflare Workers:** `wrangler secret put <NAME>` or the Cloudflare dashboard. Never inline in `wrangler.jsonc`.
- **Local dev:** `.dev.vars` (Workers) or `.env.local` (Vite) — both gitignored.
- **CI:** GitHub Actions secrets / environment secrets.
- **Examples / templates:** use `.example` / `.sample` suffixed files with placeholder values only.

Before committing, sanity-check:

```sh
git diff --cached | grep -iE 'secret|token|api[_-]?key|password|bearer|private[_-]?key'
```

If a secret *does* leak: rotate it immediately at the source, **then** scrub history. Rotation is the fix; history rewrites are damage control.

## Stack

- **pnpm** workspaces (`apps/*`, `plugins/*`, `vendor/wclap-host-js`)
- **Turborepo** for task orchestration
- **Cargo** workspace for future Rust crates
- **Cloudflare Workers** for the marketing site (`apps/site`)

## Common commands

```sh
pnpm install                              # install JS/TS deps
pnpm --filter @plinken/site dev           # run the site locally (Vite)
pnpm --filter @plinken/site preview       # run the site under wrangler dev
pnpm --filter @plinken/site deploy        # build + deploy to Cloudflare Workers
```

## Conventions

- New JS/TS packages go under `apps/` or `plugins/` and use `@plinken/<name>` as their npm name.
- Cloudflare config (`wrangler.jsonc`) holds **non-secret** settings only. Bindings → run `wrangler types` to regenerate `Env`; never hand-write the type.
- Svelte components in `apps/site` use **Svelte 5 runes** (`$state`, `$derived`, `$effect`, `$props`) — not the legacy reactive `$:` / store syntax.
