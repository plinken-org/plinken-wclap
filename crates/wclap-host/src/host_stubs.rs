//! Host-side callback stubs the plugin invokes through `clap_host_t` and
//! the event-list structs in `clap_process_t`.
//!
//! Each stub is a `#[no_mangle]` `extern "C"` so wasm32 places it in the
//! `__indirect_function_table` when something takes its address — that table
//! index is what `_wclapInstance.registerHost32` exposes to the plugin.
//!
//! Calling convention (see wclap-cpp's `registerHost32<Return, Args...>`):
//!   wasm signature is `Return(void *context, Args...)` — JS's
//!   `bind(null, context)` prepends `context` before the plugin-visible args.
//!   Our stubs accept `_ctx` as the first arg and ignore it (we don't need
//!   per-instance context at M1; `Hosted` is reachable via globals).
//!
//! Signature strings the JS dispatcher (`generate-forwarding-wasm.mjs`)
//! consumes describe ONLY the plugin-visible args — no `ctx`:
//!   first char is return type (`I`=i32, `L`=i64, `F`=f32, `D`=f64, `v`=void),
//!   following chars are argument types.

#![cfg(target_arch = "wasm32")]

// ---------------------------------------------------------------------------
// clap_host_t callbacks
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _wclap_host_get_extension(
    _ctx: u32,
    _host_ptr: u32,
    _ext_id_ptr: u32,
) -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn _wclap_host_request_restart(_ctx: u32, _host_ptr: u32) {}

#[no_mangle]
pub extern "C" fn _wclap_host_request_process(_ctx: u32, _host_ptr: u32) {}

#[no_mangle]
pub extern "C" fn _wclap_host_request_callback(_ctx: u32, _host_ptr: u32) {}

// ---------------------------------------------------------------------------
// clap_input_events / clap_output_events callbacks
// M1 hosts no events: input list is always empty, output try_push silently
// drops. Plugins that emit param-update / note events at M2 will route
// through this path — for now ignore them.
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _wclap_events_in_size(_ctx: u32, _list_ptr: u32) -> u32 {
    0
}

// Never invoked when `size` returns 0, but the table slot still needs a fn
// of the right type for the plugin to take its address.
#[no_mangle]
pub extern "C" fn _wclap_events_in_get(_ctx: u32, _list_ptr: u32, _index: u32) -> u32 {
    0
}

// CLAP returns `bool`; wasm represents it as `i32`. 0 = "rejected".
#[no_mangle]
pub extern "C" fn _wclap_events_out_try_push(
    _ctx: u32,
    _list_ptr: u32,
    _event_ptr: u32,
) -> u32 {
    0
}

// ---------------------------------------------------------------------------
// Signature strings — plugin-visible args only.
// ---------------------------------------------------------------------------

pub const SIG_II: &[u8] = b"II"; //  i32(i32) -> i32       — events_in.size, events_out (unused)
pub const SIG_III: &[u8] = b"III"; // i32(i32, i32) -> i32 — get_extension, events_in.get, events_out.try_push
pub const SIG_VI: &[u8] = b"vI"; //  void(i32)            — request_restart / _process / _callback

// ---------------------------------------------------------------------------
// Table-index getters. Cast through `*const ()` to silence the
// "direct cast of function item to integer" lint.
// ---------------------------------------------------------------------------

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
pub fn events_in_size_index() -> u32 {
    _wclap_events_in_size as *const () as u32
}
pub fn events_in_get_index() -> u32 {
    _wclap_events_in_get as *const () as u32
}
pub fn events_out_try_push_index() -> u32 {
    _wclap_events_out_try_push as *const () as u32
}
