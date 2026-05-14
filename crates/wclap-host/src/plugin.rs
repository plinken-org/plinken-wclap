//! `createPlugin` — walks the plugin's CLAP entry point, registers our four
//! host stubs in the plugin's function table, builds a `clap_host_t` in
//! plugin memory, calls `clap_plugin_factory.create_plugin`, and returns a
//! Rust-side u32 handle for the resulting `clap_plugin_t *`.
//!
//! Sequence (matches the M1 doc):
//!   1. `init32(instance)` → `clap_entry *` (a wasm pointer in plugin memory).
//!   2. `clap_entry.init(NULL)` via `call32` with `is_ptr_to_fn=1`
//!      (lets JS deref the entry struct's `init` field — no `memcpyFromOther32`).
//!   3. `clap_entry.get_factory("clap.plugin-factory")` → `factory *`.
//!   4. Register the 4 host stubs once per `Hosted` (cached).
//!   5. Build `clap_host_t` in plugin memory with stub indices + identity strings.
//!   6. `factory.create_plugin(factory, host, plugin_id_cstr)` → `clap_plugin_t *`.

use alloc::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
use crate::{
    call::{read_result_u32, write_arg_u32, SLOT_SIZE},
    clap, host,
    host::HostStubIndices,
    host_stubs,
    instance::{call32, init32, malloc32, memcpyToOther32, registerHost32},
};

#[allow(dead_code)] // fields read by wasm32 process path (step 7).
pub(crate) struct Plugin {
    pub(crate) instance_handle: u32,
    pub(crate) plugin_ptr: u32,
}

pub(crate) struct PluginPool {
    next_id: u32,
    map: BTreeMap<u32, Plugin>,
}

static mut POOL: PluginPool = PluginPool {
    next_id: 1,
    map: BTreeMap::new(),
};

fn pool() -> &'static mut PluginPool {
    unsafe { &mut *core::ptr::addr_of_mut!(POOL) }
}

#[allow(dead_code)] // first reader arrives at step 5 (`pluginGetInfo`).
pub(crate) fn get(handle: u32) -> &'static mut Plugin {
    pool().map.get_mut(&handle).expect("bad plugin handle")
}

#[cfg(target_arch = "wasm32")]
unsafe fn alloc_cstr(inst: u32, bytes: &[u8]) -> u32 {
    let total = (bytes.len() + 1) as u32;
    let ptr = malloc32(inst, total);
    if !bytes.is_empty() {
        memcpyToOther32(inst, ptr, bytes.as_ptr(), bytes.len() as u32);
    }
    let nul = 0u8;
    memcpyToOther32(inst, ptr + bytes.len() as u32, &nul, 1);
    ptr
}

#[cfg(target_arch = "wasm32")]
unsafe fn register_stubs(inst: u32, hosted_handle: u32) -> HostStubIndices {
    let reg = |fn_table_index: u32, sig: &[u8]| -> u32 {
        registerHost32(
            inst,
            hosted_handle, // host_data context (echoed to the stub via JS shim binding)
            fn_table_index,
            sig.as_ptr(),
            sig.len() as u32,
        )
    };
    HostStubIndices {
        get_extension: reg(host_stubs::get_extension_index(), host_stubs::SIG_III),
        request_restart: reg(host_stubs::request_restart_index(), host_stubs::SIG_VI),
        request_process: reg(host_stubs::request_process_index(), host_stubs::SIG_VI),
        request_callback: reg(host_stubs::request_callback_index(), host_stubs::SIG_VI),
    }
}

#[cfg(target_arch = "wasm32")]
unsafe fn build_host_struct(inst: u32, hosted_handle: u32, stubs: &HostStubIndices) -> u32 {
    let host_ptr = malloc32(inst, clap::host::SIZE as u32);

    let name_p = alloc_cstr(inst, b"Plinken");
    let vendor_p = alloc_cstr(inst, b"Plinken");
    let url_p = alloc_cstr(inst, b"https://plinken.org");
    let ver_p = alloc_cstr(inst, b"0.0.1");

    let mut buf = [0u8; clap::host::SIZE];
    let write_u32 = |buf: &mut [u8; clap::host::SIZE], off: usize, v: u32| {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    };
    // wclap_version = (1, 2, 2) — matches CLAP 1.2.2 the plugins target.
    write_u32(&mut buf, clap::host::VERSION, 1);
    write_u32(&mut buf, clap::host::VERSION + 4, 2);
    write_u32(&mut buf, clap::host::VERSION + 8, 2);
    write_u32(&mut buf, clap::host::HOST_DATA, hosted_handle);
    write_u32(&mut buf, clap::host::NAME, name_p);
    write_u32(&mut buf, clap::host::VENDOR, vendor_p);
    write_u32(&mut buf, clap::host::URL, url_p);
    write_u32(&mut buf, clap::host::VERSION_STR, ver_p);
    write_u32(&mut buf, clap::host::GET_EXTENSION, stubs.get_extension);
    write_u32(&mut buf, clap::host::REQUEST_RESTART, stubs.request_restart);
    write_u32(&mut buf, clap::host::REQUEST_PROCESS, stubs.request_process);
    write_u32(&mut buf, clap::host::REQUEST_CALLBACK, stubs.request_callback);

    memcpyToOther32(inst, host_ptr, buf.as_ptr(), buf.len() as u32);
    host_ptr
}

#[cfg(target_arch = "wasm32")]
unsafe fn create_plugin_inner(hosted_handle: u32, plugin_id_bytes: u32) -> u32 {
    let hosted = host::get(hosted_handle);
    let inst = hosted.instance_handle;

    // 1. init32 — fires the plugin's `_initialize()` and returns `clap_entry`.
    let clap_entry_ptr = init32(inst);
    if clap_entry_ptr == 0 {
        return 0;
    }

    // 2. clap_entry.init(NULL). `is_ptr_to_fn=1` makes JS treat
    //    `clap_entry_ptr + INIT` as the address of the function-pointer field
    //    rather than a direct function index — no struct-read needed.
    let mut arg = [0u8; SLOT_SIZE];
    let mut result = [0u8; SLOT_SIZE];
    write_arg_u32(&mut arg, 0);
    call32(
        inst,
        clap_entry_ptr + clap::entry::INIT as u32,
        1,
        result.as_mut_ptr(),
        arg.as_ptr(),
        1,
    );
    if read_result_u32(&result) == 0 {
        return 0;
    }

    // 3. clap_entry.get_factory("clap.plugin-factory") → factory pointer.
    let fac_id_ptr = alloc_cstr(inst, b"clap.plugin-factory");
    write_arg_u32(&mut arg, fac_id_ptr);
    call32(
        inst,
        clap_entry_ptr + clap::entry::GET_FACTORY as u32,
        1,
        result.as_mut_ptr(),
        arg.as_ptr(),
        1,
    );
    let factory_ptr = read_result_u32(&result);
    if factory_ptr == 0 {
        return 0;
    }

    // 4. Build clap_host_t in plugin memory. Stubs were registered ahead
    //    of step 1 — `registerHost32` rejects post-init calls.
    let stubs = hosted.stubs.expect("host stubs must be registered before init32");
    let host_ptr = build_host_struct(inst, hosted_handle, &stubs);

    // 6. Plugin id from the bytes pool → C string in plugin memory.
    let pid_bytes = crate::bytes::view(plugin_id_bytes);
    let plugin_id_cstr = alloc_cstr(inst, pid_bytes);

    // 7. factory.create_plugin(factory, host, plugin_id).
    let mut args3 = [0u8; SLOT_SIZE * 3];
    write_arg_u32(
        (&mut args3[0..SLOT_SIZE]).try_into().unwrap(),
        factory_ptr,
    );
    write_arg_u32(
        (&mut args3[SLOT_SIZE..2 * SLOT_SIZE]).try_into().unwrap(),
        host_ptr,
    );
    write_arg_u32(
        (&mut args3[2 * SLOT_SIZE..]).try_into().unwrap(),
        plugin_id_cstr,
    );
    call32(
        inst,
        factory_ptr + clap::factory::CREATE_PLUGIN as u32,
        1,
        result.as_mut_ptr(),
        args3.as_ptr(),
        3,
    );
    let plugin_ptr = read_result_u32(&result);
    if plugin_ptr == 0 {
        return 0;
    }

    // 8. Register and return our handle.
    let p = pool();
    let id = p.next_id;
    p.next_id += 1;
    p.map.insert(
        id,
        Plugin {
            instance_handle: inst,
            plugin_ptr,
        },
    );
    hosted.plugins.push(id);
    id
}

// JS bridge entry point. `plugin_id_bytes` is the bytes-pool handle
// `encodeString(pluginId)` returned on the JS side (raw UTF-8 bytes, no NUL).
#[no_mangle]
pub extern "C" fn createPlugin(hosted_handle: u32, plugin_id_bytes: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    {
        // JS asserts `registerHost32` runs *before* `init32` — so we must
        // pre-register the host stubs here, ahead of `create_plugin_inner`'s
        // step 1. Cached on `Hosted` so multi-plugin hosts only register once.
        let hosted = host::get(hosted_handle);
        let inst = hosted.instance_handle;
        if hosted.stubs.is_none() {
            unsafe {
                hosted.stubs = Some(register_stubs(inst, hosted_handle));
            }
        }
        unsafe { create_plugin_inner(hosted_handle, plugin_id_bytes) }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (hosted_handle, plugin_id_bytes);
        0
    }
}
