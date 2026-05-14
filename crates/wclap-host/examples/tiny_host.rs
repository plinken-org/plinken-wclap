//! Tiny example host. M1 step 6 will turn this into the
//! "render a 440 Hz sine through clack-gain" demo. Right now it just
//! loads the bundle and prints the wasm imports so we know what host
//! functions + WASI surface to wire next.
//!
//! Usage:
//!   cargo run --example tiny_host -- <path-to-.wasm-or-.tar.gz>

use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use wclap_host::{Bundle, Engine};

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("../../apps/wclap-host/public/samples/clack-gain.wasm")
        });

    let bytes = fs::read(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    println!("loaded {} bytes from {}", bytes.len(), path.display());

    let engine = Engine::new()?;
    let bundle = match Bundle::load(&engine, &bytes) {
        Ok(b) => b,
        Err(e) => bail!("Bundle::load: {e}"),
    };

    println!("\n== wasm imports ==");
    let imports = bundle.imports();
    if imports.is_empty() {
        println!("  (none — plugin is self-contained)");
    } else {
        let mut by_module: std::collections::BTreeMap<&str, Vec<&str>> =
            std::collections::BTreeMap::new();
        for (m, n) in &imports {
            by_module.entry(m).or_default().push(n);
        }
        for (m, names) in by_module {
            println!("  {m}");
            for n in names {
                println!("    - {n}");
            }
        }
    }

    println!("\n== wasm exports ==");
    for (name, kind) in bundle.exports() {
        println!("  [{kind:7}] {name}");
    }

    // Instantiate. clack plugins have no imports, so we don't need a linker.
    let mut store: wasmtime::Store<()> = wasmtime::Store::new(&engine.inner_ref(), ());
    let instance = wasmtime::Instance::new(&mut store, &bundle.module_ref(), &[])
        .context("instantiate wasm module")?;
    println!("\n== instantiated ==");

    // Read the `clap_entry` global — it holds the address (i32 offset into
    // linear memory) of the `clap_plugin_entry_t` struct.
    let clap_entry_global = instance
        .get_global(&mut store, "clap_entry")
        .context("missing `clap_entry` export")?;
    let entry_ptr = match clap_entry_global.get(&mut store) {
        wasmtime::Val::I32(v) => v as u32,
        v => bail!("clap_entry global isn't i32 (got {v:?})"),
    };
    println!("clap_entry struct address: 0x{entry_ptr:x}");

    // Dump the struct. The CLAP ABI layout is:
    //   struct clap_plugin_entry_t {
    //     clap_version_t clap_version;       // 3 × u32 (major, minor, rev)
    //     bool (*init)(const char *plugin_path);              // fn idx
    //     void (*deinit)(void);                               // fn idx
    //     const void *(*get_factory)(const char *factory_id); // fn idx
    //   };
    // wasm32 pointers / fn-table indices are 4 bytes.
    let memory = instance
        .get_memory(&mut store, "memory")
        .context("missing `memory` export")?;
    let mut buf = [0u8; 24];
    memory
        .read(&mut store, entry_ptr as usize, &mut buf)
        .context("read clap_entry struct")?;
    let read_u32 = |i: usize| u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
    println!(
        "  clap_version: {}.{}.{}",
        read_u32(0),
        read_u32(4),
        read_u32(8)
    );
    println!("  init fn idx:        {}", read_u32(12));
    println!("  deinit fn idx:      {}", read_u32(16));
    println!("  get_factory fn idx: {}", read_u32(20));

    Ok(())
}
