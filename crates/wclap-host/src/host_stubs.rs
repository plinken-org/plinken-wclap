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
// clap_host_t + event-list callbacks
//
// Each stub touches a unique static so LLVM can't merge identical-shaped
// stubs into one function. We learned the hard way: with `-> 0` bodies
// LLVM collapsed all three `(u32, u32, u32) -> u32` stubs into one, all
// three `(u32, u32) -> void` stubs into another. JS's `registerHost32`
// keys entries by table index ("hostFn"+fnIndex) — collisions there meant
// only the last-registered stub at each shared index got installed in the
// plugin's function table; the others left their slots null, and the
// plugin's first `call_indirect` (auto-pan's hostGetExtensionUtf8) trapped
// with "null function". Each TAG_* address read forces a distinct body.
// ---------------------------------------------------------------------------

static TAG_GET_EXTENSION: u8 = 0;
static TAG_REQUEST_RESTART: u8 = 0;
static TAG_REQUEST_PROCESS: u8 = 0;
static TAG_REQUEST_CALLBACK: u8 = 0;
static TAG_EVENTS_IN_SIZE: u8 = 0;
static TAG_EVENTS_IN_GET: u8 = 0;
static TAG_EVENTS_OUT_TRY_PUSH: u8 = 0;
static TAG_HOST_WEBVIEW_SEND: u8 = 0;
static TAG_STATE_WRITE: u8 = 0;
static TAG_STATE_READ: u8 = 0;

#[inline(never)]
fn touch(tag: &'static u8) {
    core::hint::black_box(tag);
}

/// `host.get_extension(host, ext_id)` — the only smart host stub. Reads
/// the ext_id C string out of plugin memory and matches against the
/// extensions we expose:
///   - `clap.webview/3` → per-plugin `clap_host_webview` struct pointer
///     (cached on `Plugin.host_webview_struct_ptr` by `createPlugin`).
/// Other ids fall through to 0 (= unsupported). All extension lookups
/// for this hosted plugin route through this single registered slot.
#[no_mangle]
pub extern "C" fn _wclap_host_get_extension(
    ctx: u32,
    host_ptr: u32,
    ext_id_ptr: u32,
) -> u32 {
    touch(&TAG_GET_EXTENSION);

    // ctx was set at `registerHost32` time to the `Hosted` handle.
    let inst = crate::host::get(ctx).instance_handle;

    // Read up to 32 bytes of ext_id (more than enough for any CLAP id).
    // memcpyFromOther32 clamps at plugin-memory bounds, leaving the tail
    // of `buf` zero from initialisation — safe even for short strings.
    let mut buf = [0u8; 32];
    unsafe {
        crate::instance::memcpyFromOther32(inst, buf.as_mut_ptr(), ext_id_ptr, buf.len() as u32);
    }

    if buf.starts_with(crate::clap::EXT_WEBVIEW) {
        // Find the calling plugin via clap_host.host_data, which we set
        // to the plugin handle at build_host_struct time.
        let mut ph_bytes = [0u8; 4];
        unsafe {
            crate::instance::memcpyFromOther32(
                inst,
                ph_bytes.as_mut_ptr(),
                host_ptr + crate::clap::host::HOST_DATA as u32,
                4,
            );
        }
        let plugin_handle = u32::from_le_bytes(ph_bytes);
        return crate::plugin::get(plugin_handle).host_webview_struct_ptr;
    }

    0
}

#[no_mangle]
pub extern "C" fn _wclap_host_request_restart(_ctx: u32, _host_ptr: u32) {
    touch(&TAG_REQUEST_RESTART);
}

#[no_mangle]
pub extern "C" fn _wclap_host_request_process(_ctx: u32, _host_ptr: u32) {
    touch(&TAG_REQUEST_PROCESS);
}

#[no_mangle]
pub extern "C" fn _wclap_host_request_callback(_ctx: u32, _host_ptr: u32) {
    touch(&TAG_REQUEST_CALLBACK);
}

/// Reads `clap_input_events.ctx` / `clap_output_events.ctx` (both at offset
/// 0) — populated by `pluginStart` with the plugin handle — so the stubs
/// can find their per-plugin queue.
#[inline]
fn plugin_handle_from_list(inst: u32, list_ptr: u32) -> u32 {
    let mut bytes = [0u8; 4];
    unsafe {
        crate::instance::memcpyFromOther32(inst, bytes.as_mut_ptr(), list_ptr, 4);
    }
    u32::from_le_bytes(bytes)
}

#[no_mangle]
pub extern "C" fn _wclap_events_in_size(ctx: u32, list_ptr: u32) -> u32 {
    touch(&TAG_EVENTS_IN_SIZE);
    let inst = crate::host::get(ctx).instance_handle;
    let plugin_handle = plugin_handle_from_list(inst, list_ptr);
    crate::plugin::get(plugin_handle).current_event_ptrs.len() as u32
}

#[no_mangle]
pub extern "C" fn _wclap_events_in_get(ctx: u32, list_ptr: u32, index: u32) -> u32 {
    touch(&TAG_EVENTS_IN_GET);
    let inst = crate::host::get(ctx).instance_handle;
    let plugin_handle = plugin_handle_from_list(inst, list_ptr);
    let ptrs = &crate::plugin::get(plugin_handle).current_event_ptrs;
    *ptrs.get(index as usize).unwrap_or(&0)
}

/// Plugin emitted an output event during its `process` call. Forward to
/// `env.eventsOutTryPush`, which the AWP routes through `clapRouting` to
/// every connected node's input queue (consumed on their next block).
/// Returns 1 (accepted) — AWP's outputEvent is async-fire-and-forget; the
/// plugin doesn't need a real success/fail signal.
#[no_mangle]
pub extern "C" fn _wclap_events_out_try_push(
    ctx: u32,
    list_ptr: u32,
    event_ptr: u32,
) -> u32 {
    touch(&TAG_EVENTS_OUT_TRY_PUSH);
    let inst = crate::host::get(ctx).instance_handle;
    let plugin_handle = plugin_handle_from_list(inst, list_ptr);

    // First u32 of clap_event_header is the event's total byte size.
    let mut size_bytes = [0u8; 4];
    unsafe {
        crate::instance::memcpyFromOther32(inst, size_bytes.as_mut_ptr(), event_ptr, 4);
    }
    let event_size = u32::from_le_bytes(size_bytes);
    if event_size == 0 {
        return 0;
    }

    unsafe {
        crate::instance::eventsOutTryPush(plugin_handle, event_ptr as *const u8, event_size);
    }
    1
}

/// `clap_ostream.write(stream, buf, size) -> i64` — the plugin pushes a
/// chunk of bytes during `clap.state.save`. We pull the bytes out of plugin
/// memory and append them to the per-plugin `state_save_buf`. Return value
/// is bytes-written (or -1 on error); we always accept the full chunk.
#[no_mangle]
pub extern "C" fn _wclap_state_write(
    ctx: u32,
    stream_ptr: u32,
    buf_ptr: u32,
    size: u64,
) -> i64 {
    touch(&TAG_STATE_WRITE);
    let inst = crate::host::get(ctx).instance_handle;
    let plugin_handle = plugin_handle_from_list(inst, stream_ptr); // ctx at offset 0
    let n = size as usize;
    if n == 0 {
        return 0;
    }
    let mut tmp = alloc::vec![0u8; n];
    unsafe {
        crate::instance::memcpyFromOther32(inst, tmp.as_mut_ptr(), buf_ptr, n as u32);
    }
    let plugin = crate::plugin::get(plugin_handle);
    plugin.state_save_buf.extend_from_slice(&tmp);
    size as i64
}

/// `clap_istream.read(stream, buf, size) -> i64` — the plugin pulls a
/// chunk of bytes during `clap.state.load`. We feed it from the
/// pre-populated `state_load_buf` at the current cursor. Returns
/// bytes-read (0 at EOF, -1 on error).
#[no_mangle]
pub extern "C" fn _wclap_state_read(
    ctx: u32,
    stream_ptr: u32,
    buf_ptr: u32,
    size: u64,
) -> i64 {
    touch(&TAG_STATE_READ);
    let inst = crate::host::get(ctx).instance_handle;
    let plugin_handle = plugin_handle_from_list(inst, stream_ptr);
    let plugin = crate::plugin::get(plugin_handle);

    let avail = plugin
        .state_load_buf
        .len()
        .saturating_sub(plugin.state_load_cursor);
    let to_read = core::cmp::min(avail, size as usize);
    if to_read == 0 {
        return 0;
    }
    let start = plugin.state_load_cursor;
    let end = start + to_read;
    unsafe {
        crate::instance::memcpyToOther32(
            inst,
            buf_ptr,
            plugin.state_load_buf[start..end].as_ptr(),
            to_read as u32,
        );
    }
    plugin.state_load_cursor = end;
    to_read as i64
}

/// `clap_host_webview.send(host, buf, size)` — plugin → iframe push.
/// AWP's `env.webviewSend` looks up its `instancePluginMap` by the
/// plugin handle returned from `createPlugin`. We stash that handle in
/// `clap_host.host_data` per-plugin and read it back here so each send
/// routes to the right iframe.
#[no_mangle]
pub extern "C" fn _wclap_host_webview_send(
    ctx: u32,
    host_ptr: u32,
    buf_ptr: u32,
    size: u32,
) -> u32 {
    touch(&TAG_HOST_WEBVIEW_SEND);
    let inst = crate::host::get(ctx).instance_handle;
    let mut ph_bytes = [0u8; 4];
    unsafe {
        crate::instance::memcpyFromOther32(
            inst,
            ph_bytes.as_mut_ptr(),
            host_ptr + crate::clap::host::HOST_DATA as u32,
            4,
        );
        let plugin_handle = u32::from_le_bytes(ph_bytes);
        crate::instance::webviewSend(plugin_handle, buf_ptr as *const u8, size);
    }
    1 // bool true
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
pub fn host_webview_send_index() -> u32 {
    _wclap_host_webview_send as *const () as u32
}
pub fn state_write_index() -> u32 {
    _wclap_state_write as *const () as u32
}
pub fn state_read_index() -> u32 {
    _wclap_state_read as *const () as u32
}
