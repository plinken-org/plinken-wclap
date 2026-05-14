# @plinken/wclap-host

Browser proof-of-concept for hosting **WCLAP** (CLAP plugins compiled to `wasm32`) on the public side of Plinken. Drop a `.wclap` bundle onto the page and hear a 440&nbsp;Hz stereo test tone routed through it.

This app is the first concrete consumer of the [WebCLAP](https://github.com/WebCLAP) project's `wclap-host-js` runtime under the plinken.org banner.

## What's inside

- `index.html` / `src/main.ts` / `src/ui.ts` — page chrome, drag-drop, status, controls, RMS meters.
- `src/wclap-runtime/` — vendored runtime that bridges `wclap-host-js` and the browser's `AudioWorklet`:
  - `clap-audionode.mjs`, `clap-audioworkletprocessor.mjs`, `host-imports.mjs`, `cbor.mjs` — Signalsmith Audio's reference wrapper around `wclap-host-js` (MIT, see *Vendored code* below).
  - `host.wasm` — C++ host compiled to wasm32 via `wclap-js-instance` from the same Signalsmith repo. ~3&nbsp;MB.
- `vendor/wclap-host-js/` (at the monorepo root) — git submodule pointing at `github.com/WebCLAP/wclap-host-js`. Wired into the build via a Vite alias.

## Architecture

```
+--------------------------------------+
|  plinken.org/wclap-host page (TS)    |
|  ↓ drag-drop a .wclap                |
+------------------┬-------------------+
                   │
+------------------▼-------------------+
|  ClapAudioNode (vendored Signalsmith)|
|  - `getHost(host.wasm)`              |
|  - `getWclap(plugin bytes)`          |
|  - constructs AudioWorkletNode       |
+------------------┬-------------------+
                   │
+------------------▼-------------------+
|  wclap-host-js (WebCLAP submodule)   |
|  - loads C++ host wasm + WCLAP wasm  |
|  - bridges CLAP structs across wasm  |
+------------------┬-------------------+
                   │
+------------------▼-------------------+
|  host.wasm  (C++, wclap-js-instance) |
|  - walks clap_plugin_factory         |
|  - calls clap_plugin->process()      |
+--------------------------------------+
```

Audio graph in the page:

```
OscillatorNode (440 Hz, L) ┐
                            ├─► ChannelMerger ─► GainNode(0.25) ─► ClapAudioNode ─┬─► destination
OscillatorNode (440 Hz, R) ┘                                                       └─► AnalyserNodes (RMS meters)
```

## Running

From the monorepo root, with submodules initialized:

```sh
git submodule update --init --recursive
pnpm install
pnpm --filter @plinken/wclap-host dev
```

Open the printed URL in a Chromium-based browser (Firefox should also work; Safari support is best-effort).

You'll see a drop zone — drag any `.wclap` file onto it. The page compiles the plugin's wasm, instantiates it, and exposes Play / Stop controls. Press Play to hear the 440&nbsp;Hz test tone routed through the plugin.

## Cross-origin isolation

`wclap-host-js` uses `SharedArrayBuffer` (for the host's optional threaded path) when the page is **cross-origin isolated**. The dev server (`vite.config.ts`) and the production Worker (`worker/index.js`) both send:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`

If you embed the page in another origin or load cross-origin scripts without CORP, the browser falls back to the non-threaded path automatically.

## Known plugin sources

Anything compiled with [`wclap-cpp`](https://github.com/WebCLAP/wclap-cpp) or [`as-clap`](https://github.com/WebCLAP/as-clap) and packaged as `.tar.gz` should work. Known-good test bundles:

- [Signalsmith Basics](https://github.com/Signalsmith-Audio/basics) — prebuilt at `https://signalsmith-audio.github.io/wasm-clap-browserhost/examples/signalsmith-basics/basics.wclap.tar.gz`.
- [`signalsmith-clap-cpp`](https://github.com/geraintluff/signalsmith-clap-cpp) example plugins.

Save one locally (e.g. `curl -O <url>`) and drag it onto the page.

## Production deploy

```sh
pnpm --filter @plinken/wclap-host build
pnpm --filter @plinken/wclap-host deploy   # requires wrangler auth
```

`build` runs `tsc --noEmit` then `vite build`; output goes to `dist/`. `deploy` ships the worker in `worker/index.js` plus the built assets to Cloudflare Workers. The worker only exists to attach the cross-origin-isolation headers to every response — assets are served from the bound `ASSETS` namespace.

The Worker name in `wrangler.jsonc` is `plinken-org-wclap-host`; the custom-domain stanza for `wclap.plinken.org` is commented out until the zone is added to the CF account.

## Vite plumbing notes

- `resolve.alias` maps `@webclap/wclap-host-js` → `vendor/wclap-host-js/wclap.mjs`. The submodule has no `package.json`, so it can't be a pnpm workspace dep.
- `server.fs.allow: ['../..']` lets Vite serve files from `vendor/` (outside the app's own root).
- The `AudioWorkletProcessor` is bundled via the `?worker&url` Vite import suffix. Vite produces a self-contained ES module whose URL is passed to `audioContext.audioWorklet.addModule()`. `clap-audionode.mjs` was patched to skip its own internal `addModule` call (see the `PATCH (plinken-org)` comments in `src/wclap-runtime/`).

## Vendored code & attribution

The files under `src/wclap-runtime/` (apart from the `PATCH (plinken-org)` blocks) are copied verbatim from [`Signalsmith-Audio/wasm-clap-browserhost`](https://github.com/Signalsmith-Audio/wasm-clap-browserhost), copyright © 2022 Geraint Luff / Signalsmith Audio Ltd., MIT licensed. `host.wasm` is the compiled output of that repo's C++ side (`wclap-js-instance` + `wclap-cpp`), built from the same upstream sources.

The `vendor/wclap-host-js/` submodule is [`WebCLAP/wclap-host-js`](https://github.com/WebCLAP/wclap-host-js), same author, same license.

To rebuild `host.wasm` yourself, follow the build instructions in the upstream Signalsmith repo (`clap-audionode/host-dev/`) — it needs Emscripten and the `wclap-cpp`/`wclap-host-js` submodules. Drop the resulting `host.wasm` into `src/wclap-runtime/`.

## Status & scope

v1 deliberately omits:

- Plugin GUIs (`clap_plugin_gui` reports unsupported; the wrapper handles webview-style UIs but we don't surface them yet).
- MIDI events, parameter automation, transport.
- Multi-plugin chains.
- State save / load.
- Real input sources beyond the bundled test tone.

These are all reachable through the same `ClapEffectAudioNode` API surface — see `src/wclap-runtime/clap-audionode.mjs` for what's already on the node and `src/wclap-runtime.d.ts` for the methods we already type.
