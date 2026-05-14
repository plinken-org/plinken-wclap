//! Typed errors for the public API. Library code returns `Result<T, Error>`;
//! consumers can match on variants without `Display`-parsing.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid WCLAP bundle: {0}")]
    Bundle(String),

    #[error("wasm compilation failed: {0}")]
    Compile(String),

    #[error("wasm instantiation failed: {0}")]
    Instantiate(String),

    #[error("CLAP plugin '{id}' not found in bundle")]
    PluginNotFound { id: String },

    #[error("CLAP ABI error: {0}")]
    ClapAbi(String),

    #[error("WASI denied: {0}")]
    WasiDenied(String),

    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Error::Compile(err.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
