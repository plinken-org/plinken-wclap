// Tell wasm-ld to export `__indirect_function_table` so `wclap-host-js` can
// find it. The JS bridge scans the host's exports for any `WebAssembly.Table`
// and uses the last function-bearing one as the source of host-stub indices
// (`vendor/wclap-host-js/es6/wclap.mjs` ~line 262). Rust's wasm32 lld doesn't
// export the table by default.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-cdylib-link-arg=--export-table");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
