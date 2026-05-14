//! `Engine` — a shared, immutable wasmtime engine. One per host process
//! (or per audio engine). All `Bundle`s and `Plugin`s share it.

use crate::error::Result;

#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: wasmtime::Engine,
}

impl Engine {
    /// Build a `wasmtime::Engine` configured for audio-plugin hosting:
    /// SIMD on, bulk memory on, threads off (M6), optimised cranelift output.
    pub fn new() -> Result<Self> {
        let mut config = wasmtime::Config::new();
        config
            .wasm_simd(true)
            .wasm_relaxed_simd(true)
            .wasm_bulk_memory(true)
            .wasm_threads(false)
            .cranelift_opt_level(wasmtime::OptLevel::Speed);
        // Cache compiled artifacts so subsequent loads of the same bundle are
        // cheap. Caller can override via env vars; defaults to off until we
        // pick a cache directory policy (see implementation-plan §9).
        let inner = wasmtime::Engine::new(&config)?;
        Ok(Engine { inner })
    }
}

impl Engine {
    /// Internal: borrow the underlying wasmtime engine for plumbing in
    /// other modules / examples. Public API uses higher-level types only.
    pub fn inner_ref(&self) -> &wasmtime::Engine {
        &self.inner
    }
}

impl Default for Engine {
    fn default() -> Self {
        // `unwrap`: only fails on invalid wasmtime::Config, which the static
        // construction above can't produce.
        Engine::new().expect("default wasmtime config is valid")
    }
}
