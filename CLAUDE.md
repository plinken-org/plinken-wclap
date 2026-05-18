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

## Rust WCLAP plugin gotchas

Three sharp edges, all bitten before — read these before touching `crates/wclap-plugin` or scaffolding a new Rust plugin under `plugins/`.

### 1. `build.rs` needs BOTH linker flags

```rust
println!("cargo:rustc-cdylib-link-arg=--export-table");
println!("cargo:rustc-cdylib-link-arg=--growable-table");
```

Without `--growable-table`, rust-lld emits the wasm function table with `max == initial`. The host then fails at load with `RangeError: WebAssembly.Table.grow()` when it tries to install host trampolines. Copy `build.rs` from `plugins/com.plinken/vocal-limiter/` for any new plugin.

### 2. `clap_entry` must be the struct itself, not a pointer wrapper

`crates/wclap-plugin` declares:

```rust
#[no_mangle]
pub static mut clap_entry: ClapEntry = ClapEntry { … };
```

**Not** a separate `pub static clap_entry: StaticAddr = StaticAddr(addr_of!(ENTRY) as *const ())`. For a `pub static X`, rust-lld exports the wasm global with VALUE = the static's *address*, not its content. The wrapper form makes the host read `(slot_address + 12 / 16 / 20)` instead of `(ENTRY_address + 12 / 16 / 20)` — which lands in a neighbouring static and yields `WebAssembly.Table.get(): invalid address …`.

### 3. No `Box<dyn AudioUnit>` for fundsp graphs

Our `[profile.release]` has `lto = true, codegen-units = 1`. The linker drops trait methods it deems unreachable, but a dyn vtable still indexes them — first call into a dead slot traps with `RuntimeError: null function`. Always store fundsp graphs as concrete types:

```rust
// ❌ crashes at load/activate
struct Plug { unit: Box<dyn AudioUnit> }

// ✅ static dispatch, works
struct Plug { unit: An<Limiter<U2>> }

// ✅ also fine — hide the generic plumbing behind a builder
fn build() -> An<impl AudioNode<Sample = f32, Inputs = U2, Outputs = U2>> { … }
```

Call AudioUnit methods via UFCS to get the slice-based `tick` (because `An<X>`'s inherent `tick` takes a `Frame<f32, U2>` instead):

```rust
AudioUnit::tick(&mut self.unit, &buf_in, &mut buf_out);
AudioUnit::set_sample_rate(&mut self.unit, sample_rate);
```

Applies to every Rust plugin we ship (vocal-* trio, synome's future voice pool, etc.).

## Cmajor → WCLAP pipeline

Cmajor-authored plugins live alongside the Rust ones under
`plugins/<vendor>/<name>/` and ship the same `.wclap.tar.gz` artifact, so
`scripts/build-shelf.mjs` is one path for both. The build wrapper is
`scripts/build-cmaj-wclap.sh` and the canonical user is
`plugins/com.plinken/organ/` — copy that as the template for a new
Cmajor plugin.

Pipeline:

```
.cmajorpatch
   │  cmaj generate --target=clap --clapIncludePath=vendor/clap/include
   ▼
generated/clap/*.cpp           (self-contained CLAP C++, no JIT)
   │  ${WASI_SDK}/bin/clang++ --target=wasm32-wasi -fno-exceptions
   │    -fno-rtti -Oz  -Wl,--export=clap_entry --export-table --growable-table
   ▼
dist/<name>.wclap.wasm
   │  scripts/bundle-wclap.mjs (same as the Rust plugins use)
   ▼
dist/<name>.wclap.tar.gz       → picked up by build-shelf.mjs
```

### Tooling assumptions (not vendored)

- **`cmaj`** — install from
  [cmajor-lang/cmajor releases](https://github.com/cmajor-lang/cmajor/releases),
  put on `$PATH`, or `CMAJ=/path/to/cmaj`.
- **WASI-SDK** — unpack a release from
  [WebAssembly/wasi-sdk releases](https://github.com/WebAssembly/wasi-sdk/releases)
  to `/opt/wasi-sdk`, or `WASI_SDK=/path/to/it`.
- **CLAP headers** — `git submodule add https://github.com/free-audio/clap vendor/clap`.
  The headers are the CLAP ABI; they're tiny, version-stable, and
  shared by every CLAP language binding.

The build script fails with a one-line "install X / set ENV" message
when any of the three are missing — no wall of C++ template errors.

### Why the same `--export-table --growable-table` story applies

Same reason as Rust (see the rust-lld note above). `wclap-host-js` grows
the function table at runtime to install trampolines for host callbacks;
without `--growable-table` lld emits `max == initial` and the host traps
the first `WebAssembly.Table.grow()`.

### Why not use cmaj's CMakeLists.txt

It exists, but it targets native CLAP (JUCE-free webview, platform GUI).
Driving `clang++` directly keeps the wasm-specific link flags in one
place and avoids dragging in CMake just to compile a handful of `.cpp`
files. Switch to the cmaj CMake project the day we want feature parity
with native CLAP builds (signing, codesign, etc.).

## How a plugin UI iframe is routed

The flow from "plugin has a UI" to "iframe shows content" is non-obvious; we bit a bug on it shipping vocal-limiter. The full path:

1. **`plugin.json` declares `has_ui: true`** + panel sizes. This is a *hint* the host uses for chrome (rack strip layout, panel size). It does **not** trigger UI loading on its own.

2. **The plugin (wasm side) implements the `clap.webview/3` extension.** Our shared crate (`crates/wclap-plugin`) provides this — set `ui_path: Some(b"/ui/index.html\0")` in `PluginDef`. When the host queries `plugin.get_extension("clap.webview/3")`, the scaffold returns a populated `clap_plugin_webview` struct; otherwise the host concludes "no UI" and silently skips the iframe.

3. **`entry.init(modulePath)` delivers the per-instance path.** The host allocates a NUL-terminated string in plugin memory like `"/plugin/<hash>"` and passes its pointer to `entry.init`. Our scaffold stashes it. **This step is mandatory** — without it `webview.get_uri` can't produce a URI that matches the file map.

4. **`webview.get_uri` composes `file:<modulePath><ui_path>`.** Two-call probe: `cap=0` returns byte length; `cap>0` writes the bytes + NUL. Final string is e.g. `file:/plugin/abc123/ui/index.html`.

5. **The host strips `file:` and prepends `/plugin-proxy`**, yielding the iframe `src` = `/plugin-proxy/plugin/abc123/ui/index.html`.

6. **The plugin-proxy service worker intercepts** that fetch, asks the host page for the bytes, which calls `effect.getFile('/plugin/abc123/ui/index.html')` against the tarball's file map (keys are `/plugin/<hash>/<filename>`). Match — the file is served.

7. **Relative imports inside `ui/index.html`** (e.g. `import { Meter } from '../widgets/meter.mjs'`) resolve against the iframe URL and route through the SW the same way. The widgets/ folder is bundled into every plugin tarball with a UI by `scripts/bundle-wclap.mjs`.

**Common failure modes & what they look like:**

- *Plugin shows in rack but iframe is blank/white* → `get_uri` returned a URI without the `modulePath`. SW asks the host for a path that isn't in the file map, request 404s, no CSS loads, white background.
- *No iframe appears at all* → plugin's `get_extension("clap.webview/3")` returned 0. Likely `ui_path: None` or extension wiring missing.
- *Iframe loads, CSS works, but JS imports 404* → forgot to include `widgets/` in the bundler, or referenced path inside the iframe doesn't resolve back to a file-map key. Check `tar -tzf` of the built tarball.
