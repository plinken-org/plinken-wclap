//! Curated WASI surface. M1 wires stdio through to the host so plugin
//! `console.log` / `printf` shows up in the terminal during development.
//! Filesystem and clock policy comes in `docs/wasi-surface.md`.

// Placeholder. Concrete WASI setup happens once we see what `clack-gain`
// imports — see `bundle.imports()` output from the `tiny_host` example.
