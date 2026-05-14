//! Host-side imports the CLAP plugin pulls in (the `clap_host_t` callbacks
//! and any other host services the wasm needs).
//!
//! Concrete wiring happens once we've inspected what `clack-gain.wasm`
//! actually imports — see `bundle.imports()` and the `tiny_host` example.
//! For M1 we'll fill in stubs that satisfy the linker; real semantics
//! arrive when we have an audio loop to test them against.

// Placeholder for M1 step 4.
