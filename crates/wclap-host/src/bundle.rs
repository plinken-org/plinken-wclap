//! `Bundle` — a loaded WCLAP artifact. M1 only handles bare `.wasm`;
//! `.wclap.tar.gz` unpacking is M2.

use crate::engine::Engine;
use crate::error::{Error, Result};

pub struct Bundle {
    pub(crate) module: wasmtime::Module,
}

impl Bundle {
    /// Internal: borrow the wasmtime module. Public API uses the higher
    /// level types, but the example/diagnostic needs raw access while we
    /// figure the M1 instantiation flow out.
    pub fn module_ref(&self) -> &wasmtime::Module {
        &self.module
    }
}

impl Bundle {
    pub fn load(engine: &Engine, bytes: &[u8]) -> Result<Bundle> {
        // Sniff the header. Bare wasm starts with `\0asm`; tar.gz starts
        // with the gzip magic 1f 8b. Anything else is rejected.
        match bytes {
            [0x00, 0x61, 0x73, 0x6d, ..] => {
                let module = wasmtime::Module::from_binary(&engine.inner, bytes)
                    .map_err(|e| Error::Compile(e.to_string()))?;
                Ok(Bundle { module })
            }
            [0x1f, 0x8b, ..] => Err(Error::Bundle(
                ".wclap.tar.gz bundles arrive in M2".into(),
            )),
            _ => Err(Error::Bundle(format!(
                "unrecognised header (expected wasm `\\0asm` or gzip 1f 8b), got: {:02x?}",
                &bytes[..bytes.len().min(8)]
            ))),
        }
    }

    /// List every wasm-side import the module declares — useful while we
    /// figure out what host imports + WASI calls a real plugin actually
    /// needs at M1.
    pub fn imports(&self) -> Vec<(String, String)> {
        self.module
            .imports()
            .map(|imp| (imp.module().to_string(), imp.name().to_string()))
            .collect()
    }

    /// Names + types of everything the module exports. Useful diagnostically
    /// while we map CLAP's expected entry-point conventions onto a specific
    /// toolchain's wasm output (clack vs. wclap-cpp vs. as-clap).
    pub fn exports(&self) -> Vec<(String, String)> {
        self.module
            .exports()
            .map(|exp| {
                let kind = match exp.ty() {
                    wasmtime::ExternType::Func(_) => "func",
                    wasmtime::ExternType::Global(_) => "global",
                    wasmtime::ExternType::Memory(_) => "memory",
                    wasmtime::ExternType::Table(_) => "table",
                };
                (exp.name().to_string(), kind.to_string())
            })
            .collect()
    }
}
