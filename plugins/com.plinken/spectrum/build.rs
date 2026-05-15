// rust-lld defaults to a fixed-size function table; the WCLAP host needs to
// grow it at runtime to install trampolines, so both flags are required.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-cdylib-link-arg=--export-table");
        println!("cargo:rustc-cdylib-link-arg=--growable-table");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
