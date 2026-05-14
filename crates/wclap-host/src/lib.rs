//! WCLAP host — load CLAP plugins compiled to wasm32 inside a Rust audio
//! engine via wasmtime.
//!
//! See `README.md` for the goal and `docs/implementation-plan.md` for the
//! spine. This crate is at **M1** (first sound) — the public API will
//! stabilise around M3.

mod engine;
mod bundle;
mod error;
mod imports;
mod wasi;

pub use engine::Engine;
pub use bundle::Bundle;
pub use error::{Error, Result};
