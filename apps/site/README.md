# @plinken/site

SvelteKit app for **plinken.org** (and `www.plinken.org`). Built on **Svelte 5** with runes (`$state`, `$derived`, `$effect`, `$props`). Deployed as a **Cloudflare Worker** with static assets via `@sveltejs/adapter-cloudflare`.

## Develop

```sh
pnpm install
pnpm --filter @plinken/site dev          # Vite dev server (fast iteration)
pnpm --filter @plinken/site preview      # wrangler dev — closer to production
```

## Deploy

```sh
pnpm --filter @plinken/site deploy
```

Wrangler will prompt for auth on first run (`wrangler login`).

## Secrets

> **This is a public repo.** Never commit `.env`, `.dev.vars`, API tokens, or account IDs.

- Local dev: put values in `apps/site/.dev.vars` (gitignored).
- Production: `wrangler secret put <NAME>` or set in the Cloudflare dashboard.
- After adding bindings to `wrangler.jsonc`, run `pnpm cf-typegen` to refresh `Env` types — never hand-write them.

## Domains

`plinken.org` and `www.plinken.org` are wired up either via the Cloudflare dashboard (Workers → Custom domains) or by uncommenting the `routes` block in `wrangler.jsonc`.
