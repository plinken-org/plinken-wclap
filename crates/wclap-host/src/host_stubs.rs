//! Host-side callback stubs the plugin invokes through `clap_host_t`.
//!
//! Each stub is a `#[no_mangle]` `extern "C"` so wasm32 places it in the
//! `__indirect_function_table` when something takes its address — that table
//! index is what `_wclapInstance.registerHost32` exposes to the plugin.
//!
//! All four are no-ops at M1:
//! - `get_extension` returns 0 ("no extension supported").
//! - `request_*` are control-flow signals from plugin to host that the M1
//!   render loop doesn't need to observe.
//!
//! Signatures the JS dispatcher expects (`generate-forwarding-wasm.mjs`):
//! first char is the return type, remainder are argument types. Uppercase
//! letters are wasm value types (`I`=i32, `L`=i64, `F`=f32, `D`=f64);
//! lowercase `v` only appears as the leading "no return" marker.

#![cfg(target_arch = "wasm32")]

// Plugin pointer arguments are the `clap_host_t *` JS allocated in plugin
// memory in step 4c — we don't deref them here.

#[no_mangle]
pub extern "C" fn _wclap_host_get_extension(_host_ptr: u32, _ext_id_ptr: u32) -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn _wclap_host_request_restart(_host_ptr: u32) {}

#[no_mangle]
pub extern "C" fn _wclap_host_request_process(_host_ptr: u32) {}

#[no_mangle]
pub extern "C" fn _wclap_host_request_callback(_host_ptr: u32) {}

/// Signature strings — exact bytes the JS dispatcher decodes per
/// `generate-forwarding-wasm.mjs`.
pub const SIG_III: &[u8] = b"III"; // i32(i32, i32) -> i32
pub const SIG_VI: &[u8] = b"vI"; //  void(i32)

/// Table-index getters. On wasm32 a function reference cast through a raw
/// pointer to `u32` is the `__indirect_function_table` slot the compiler
/// emitted for that function; `_wclapInstance.registerHost32` consumes that
/// value. The two-stage cast (`as *const ()` then `as u32`) silences rustc's
/// "direct cast of function item" lint.
pub fn get_extension_index() -> u32 {
    _wclap_host_get_extension as *const () as u32
}
pub fn request_restart_index() -> u32 {
    _wclap_host_request_restart as *const () as u32
}
pub fn request_process_index() -> u32 {
    _wclap_host_request_process as *const () as u32
}
pub fn request_callback_index() -> u32 {
    _wclap_host_request_callback as *const () as u32
}
