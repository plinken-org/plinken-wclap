# WCLAP plugin registry — manifest spec & hosting

`plinken.org` publishes an open catalog of WCLAP plugins at
[`plinken.org/shelf.json`](https://plinken.org/shelf.json). This document
specifies the JSON shape so anyone (a host UI, a CLI tool, the upcoming
MCP server, a third-party registry) can produce, consume, or mirror it.

> **Status:** v1 draft. Not (yet) part of any upstream WCLAP spec. The
> shape below describes what this repo's aggregator emits today and what
> [`plinken.org/shelf.json`](https://plinken.org/shelf.json) returns.

## Two files

| Where | What |
|---|---|
| `plugins/<vendor>/<name>/plugin.json` | Source-side per-plugin manifest. One file per WCLAP **bundle**. Committed by the plugin author in the source repo. |
| `<registry>/shelf.json` | Aggregated catalog served over HTTP with CORS open. Built from many `plugin.json` files (and optionally external entries). |

A "bundle" is a single artifact (`.wclap.wasm` or `.wclap.tar.gz`) that
contains one or more CLAP plugins exposed through its factory.

## `plugin.json` — per-bundle manifest

```json
{
  "manifest_version": 1,
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
  "format": "wasm",
  "has_ui": false,
  "plugins": [
    {
      "id": "com.plinken.auto-pan",
      "name": "Auto-Pan",
      "features": ["audio-effect", "utility"],
      "has_ui": false
    }
  ]
}
```

### Fields

| Field | Type | Req | Notes |
|---|---|---|---|
| `manifest_version` | int | ✓ | Must be `1`. Newer aggregators refuse formats they don't understand. |
| `id` | string | ✓ | Reverse-DNS bundle id, typically matches the CLAP plugin id when single-plugin. |
| `name` | string | ✓ | Human-readable bundle label. |
| `vendor` | string | ✓ | Bundle author. |
| `version` | string | ✓ | semver recommended. |
| `format` | `"wasm"` \| `"tar.gz"` | ✓ | Artifact shape. `wasm` = bare CLAP module, `tar.gz` = bundle with optional UI assets. |
| `artifact` | string | ✓ | Path to the built artifact, relative to the manifest. The aggregator copies this to the served URL. |
| `description` | string | — | One-line plain text. |
| `features` | string[] | — | CLAP feature tags (`audio-effect`, `instrument`, `filter`, …). See [plugin-features.h](https://github.com/free-audio/clap/blob/main/include/clap/plugin-features.h). |
| `license` | SPDX-id | — | e.g. `MIT`, `Apache-2.0`, `BSL-1.0`. |
| `homepage` | URL | — | Plugin or vendor home. |
| `source` | string | — | Path (or URL) to source files. |
| `has_ui` | bool | — | `true` if at least one plugin in the bundle ships a webview GUI. Fast hint; clients still probe per-plugin via CLAP's `clap.gui` extension. |
| `plugins` | array | — | The CLAP plugins exposed by this bundle. Required for multi-plugin bundles; encouraged for single-plugin. |
| `hint` | string | — | Short user-facing caveat (e.g. `"needs MIDI"`). |

### `plugins[]` entry

| Field | Type | Req | Notes |
|---|---|---|---|
| `id` | string | ✓ | CLAP plugin id (matches what the plugin's descriptor reports). |
| `name` | string | ✓ | CLAP plugin name. |
| `features` | string[] | — | Per-plugin CLAP features. |
| `has_ui` | bool | — | Whether this specific plugin reports a webview GUI. |

When `plugins` is omitted, the bundle is assumed to expose a single
plugin whose `id`/`name` match the bundle's top-level fields.

## `shelf.json` — aggregated catalog

```json
{
  "manifest_version": 1,
  "generatedAt": "2026-05-14T06:11:20.452Z",
  "items": [
    {
      "id": "com.plinken.auto-pan",
      "label": "Plinken: Auto-Pan",
      "url": "/wclap/com.plinken.auto-pan.wclap.wasm",
      "vendor": "Plinken",
      "version": "0.1.0",
      "license": "MIT",
      "source": "https://github.com/taluvi-dev/plinken-org/tree/main/plugins/com.plinken/auto-pan",
      "description": "Stereo auto-panner with sine LFO.",
      "features": ["audio-effect", "utility"],
      "has_ui": false,
      "plugins": [
        { "id": "com.plinken.auto-pan", "name": "Auto-Pan", "has_ui": false }
      ]
    }
  ]
}
```

Each entry mirrors the source manifest (filtered to fields useful to a
consumer) plus a registry-side `url` pointing at the artifact under the
same origin. URLs can be path-relative or absolute; the catalog at
`plinken.org/shelf.json` uses path-relative `/wclap/<id>.<ext>` so it
resolves correctly from whichever origin you fetch the JSON.

## Hosting requirements

A registry that serves `shelf.json` (and any referenced artifacts) MUST:

- Be reachable over HTTPS.
- Send `Access-Control-Allow-Origin: *` on `/shelf.json` and on every
  artifact URL.
- Send `Cross-Origin-Resource-Policy: cross-origin` so cross-origin
  isolated hosts (running under `COEP: require-corp`) can load the
  artifacts.
- Serve `.wasm` with `Content-Type: application/wasm` and `.tar.gz` with
  `application/gzip`.

`plinken-org` does this via the SvelteKit `static/_headers` file
(`apps/site/_headers`).

## Curation policy at plinken.org

`plinken.org/shelf.json` is the **stable channel**. Plugins land here
once they:

1. Have a valid `plugin.json` (passing the aggregator's validation),
2. Ship under an SPDX-compatible license (MIT/Apache-2.0 by default),
3. Load and run in `wclap-host` without uncaught errors, and
4. Don't introduce supply-chain risk (the built artifact is in the PR).

The contribution path is a PR adding the plugin folder under
`plugins/<vendor>/<plugin-name>/` of this repo. The aggregator script
(`scripts/build-shelf.mjs`) picks it up on the next build.

### Pulling in external repos

Vendors who maintain their plugins in their own repos can:

- Publish their own `shelf.json` under their domain (same shape) — any
  consumer that knows the URL can fetch it directly.
- Or open a PR that vendors their built artifact + `plugin.json` into
  `plugins/<their-vendor>/<plugin>/` here.

Both paths are fine. `plinken.org/shelf.json` will only list what's
explicitly added to this repo's `plugins/`; it does not aggregate
arbitrary remote registries at runtime.

## Versioning

- `manifest_version` is bumped only when a backwards-incompatible
  change is required. Additive fields keep the version stable;
  consumers should ignore unknown keys.
- `version` (per-bundle) is plugin-author-controlled. semver
  recommended.
- `generatedAt` on `shelf.json` is informational; consumers should not
  rely on it for cache invalidation. Use HTTP `Cache-Control` /
  `ETag` instead.

## Open questions

- **Per-plugin sub-IDs**: today we don't enumerate sub-plugin CLAP IDs
  automatically for multi-plugin bundles (e.g. Signalsmith's bundle has
  six plugins). Vendors who care can populate `plugins[]` in their
  `plugin.json` by hand; otherwise clients enumerate via
  `node.plugins()` at load time.
- **Screenshots & previews**: not in v1. Add `images: [{ url, alt }]`
  when there's appetite for a richer browse UX.
- **Mirroring across registries**: a future field like
  `mirrors: ["https://..."]` could list alternate hosts. v1 doesn't
  specify it.

## Reference implementation

- Aggregator: [`scripts/build-shelf.mjs`](./scripts/build-shelf.mjs)
- Manifest example: [`plugins/com.plinken/auto-pan/plugin.json`](./plugins/com.plinken/auto-pan/plugin.json)
- Live catalog: [`plinken.org/shelf.json`](https://plinken.org/shelf.json)
- Consumer: [`apps/wclap-host/src/main.ts`](./apps/wclap-host/src/main.ts) → `loadShelf()`
