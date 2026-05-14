#[link(wasm_import_module = "_wclapInstance")]
extern "C" {
    pub fn init32(handle: u32) -> u32;
    pub fn malloc32(handle: u32, size: u32) -> u32;
    pub fn memcpyToOther32(handle: u32, dest_p: u32, src: *const u8, count: u32) -> u32;
    pub fn memcpyFromOther32(handle: u32, dest: *mut u8, src_p: u32, count: u32) -> u32;
    pub fn call32(
        handle: u32,
        wasm_fn: u32,
        is_ptr_to_fn: u32,
        result_ptr: *mut u8,
        args_ptr: *const u8,
        args_count: u32,
    ) -> u32;
    pub fn registerHost32(
        handle: u32,
        ctx: u32,
        fn_idx: u32,
        sig: *const u8,
        sig_len: u32,
    ) -> u32;
    pub fn countUntil32(
        handle: u32,
        start_p: u32,
        until: *const u8,
        item_size: u32,
        max_count: u32,
    ) -> u32;
    pub fn runThread(handle: u32, thread_id: u32, start_arg: u64);
    pub fn release(handle: u32);
}

#[link(wasm_import_module = "env")]
extern "C" {
    pub fn log(plugin_ptr: u32, severity: u32, msg_ptr: *const u8, len: u32);
    pub fn paramsRescan(plugin_ptr: u32, flags: u32);
    pub fn stateMarkDirty(plugin_ptr: u32);
    pub fn webviewSend(plugin_ptr: u32, ptr: *const u8, len: u32);
    pub fn eventsOutTryPush(plugin_ptr: u32, ptr: *const u8, len: u32) -> u32;
}

// Keep every declared import live until real call sites land in later M1 steps.
// Without this, the linker strips unreferenced extern fns and the wasm imports
// section ends up empty, so step 1's "imports match the C++ host" verify can't
// be checked.
#[used]
static _KEEP_IMPORTS: extern "C" fn() = _keep_imports;

extern "C" fn _keep_imports() {
    use core::hint::black_box;
    unsafe {
        black_box(init32(black_box(0)));
        black_box(malloc32(black_box(0), black_box(0)));
        black_box(memcpyToOther32(
            black_box(0),
            black_box(0),
            black_box(core::ptr::null()),
            black_box(0),
        ));
        black_box(memcpyFromOther32(
            black_box(0),
            black_box(core::ptr::null_mut()),
            black_box(0),
            black_box(0),
        ));
        black_box(call32(
            black_box(0),
            black_box(0),
            black_box(0),
            black_box(core::ptr::null_mut()),
            black_box(core::ptr::null()),
            black_box(0),
        ));
        black_box(registerHost32(
            black_box(0),
            black_box(0),
            black_box(0),
            black_box(core::ptr::null()),
            black_box(0),
        ));
        black_box(countUntil32(
            black_box(0),
            black_box(0),
            black_box(core::ptr::null()),
            black_box(0),
            black_box(0),
        ));
        runThread(black_box(0), black_box(0), black_box(0));
        release(black_box(0));
        log(
            black_box(0),
            black_box(0),
            black_box(core::ptr::null()),
            black_box(0),
        );
        paramsRescan(black_box(0), black_box(0));
        stateMarkDirty(black_box(0));
        webviewSend(black_box(0), black_box(core::ptr::null()), black_box(0));
        black_box(eventsOutTryPush(
            black_box(0),
            black_box(core::ptr::null()),
            black_box(0),
        ));
    }
}
