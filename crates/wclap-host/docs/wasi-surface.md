# WASI surface

Scope of WASI in the web-only port. Short doc — most of the WASI mess belongs to the plugin and the JS glue, not to us.

## Two WASI surfaces, only one is ours

```
                  +----------------------------+
                  | wclap-host-js  (JS glue)   |
                  |  - provides WASI to BOTH    |
                  |    host.wasm and plugin.wasm|
                  +--------┬──────────┬--------+
                           │          │
                           ▼          ▼
                   host.wasm     plugin.wasm
                  (our crate)    (clack-gain etc.)
```

**Plugin-side WASI** is whatever the plugin author's toolchain links — typically `wasi-libc` for clock, random, stdio, and (for threaded plugins) `wasi.thread-spawn`. JS supplies all of it directly to each plugin's `WebAssembly.Instance`. **None of it routes through our crate**, and we don't influence which calls plugins can make. If a plugin gets the wrong WASI, that's a `wclap-host-js` (or embedder) concern.

**Host-side WASI** is whatever Rust's `std` (or transitive deps) pulls in when compiling for `wasm32-unknown-unknown` after `extern crate std`. This doc is about that surface.

## Default target: minimize host WASI

The C++ `host.wasm` imports a long preview1 list because `wasi-libc` is the libc behind every printf, malloc-tracking, and clock call C++ makes. A Rust `no_std + alloc` build can ship **zero** WASI imports — the allocator (`dlmalloc` in Rust's wasm target) is self-contained and we route logging through `env.log` rather than `fd_write`.

The plan is therefore:

- **M0 default**: `#![no_std]` + `extern crate alloc`. The crate's wasm imports should be exactly the `_wclapInstance.*` + `env.*` set; no `wasi_snapshot_preview1.*`.
- **If a useful dep forces `std`**, accept a slim WASI surface (likely `fd_write` for stdio + `proc_exit` for panic). Document the import list in this file when it lands.
- **`tracing`** stays compiled-in for `Level::Error` and routes through `env.log`. Higher levels are compile-time gated to keep the wasm tight.

## Forbidden imports (host side)

Even if a future `std` build is unavoidable, these stay out:

- `fd_read`, `fd_seek`, `fd_close` and any other `fd_*` beyond `fd_write` (no filesystem).
- `path_*` — no path resolution.
- `random_get` — if we ever need randomness, route through `env`-side imports we control. Plugins get their own randomness; ours doesn't have to share.
- `sock_*` — no sockets.
- `clock_time_get` for non-monotonic clocks — if we need timing it's `Monotonic` only.

The reasoning is consistency: anything we import is something the embedder (the example app, plinken-app, future hosts) has to provide and trust. Keeping the surface minimal keeps the trust surface minimal.

## Forbidden imports (audio thread)

This applies whether the WASI call comes from us or from a plugin — but again, plugins are not our problem; we only enforce on our own side.

`pluginProcess` and `pluginMainThread` must not transitively call any WASI function. No `println!`, no logging at `Level::Info`/`Trace`, no error reporting that allocates. Errors in the process path return a status code through the function result; they do not log.

If a panic happens on the audio thread, `std::panic::set_hook` redirects to `env.log` (`severity = error`). Logging in JS is non-blocking from the audio worklet's perspective (`console.log` enqueues; it doesn't yield).

## Threading (M7)

`wasi.thread-spawn` is reserved for plugins that use it. We do not spawn host-side threads — the worklet is one thread, `pluginProcess` is one call.

When M7 lands, `_wclapInstance.runThread` is invoked by JS to re-enter the plugin on a worker; our crate forwards through whatever bookkeeping the plugin instance needs. No WASI on our side; the worker is the plugin's instance, with its own WASI surface.

## Verification

When M0/M1 builds land, dump the imports of our `host.wasm`:

```sh
wasm-objdump -j Import -x \
  target/wasm32-unknown-unknown/release/wclap_host.wasm
```

The output should contain `_wclapInstance.*`, `env.*`, and nothing else. If `wasi_snapshot_preview1.*` shows up unexpectedly, track down which dep is the source (`cargo tree`, `cargo bloat`) and either feature-gate it off or replace it.
