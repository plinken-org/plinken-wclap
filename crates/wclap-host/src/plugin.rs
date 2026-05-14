//! Factory walk shared by `getInfo` and `createPlugin`.
//!
//! `init32` is one-shot — JS asserts "WCLAP initialised twice" — so whichever
//! of the two JS calls arrives first owns the walk. We cache `clap_entry *`
//! and `clap_plugin_factory *` on `Hosted` for the other to reuse. Host
//! stubs are registered before `init32` because `registerHost32` rejects
//! post-init calls.
//!
//! Walk steps:
//!   1. Register the 4 host stubs (`get_extension`, `request_restart/_process/_callback`).
//!   2. `init32(instance)` → `clap_entry *`.
//!   3. `clap_entry.init(NULL)`. `is_ptr_to_fn=1` makes JS deref the
//!      function-pointer field for us, so we skip a `memcpyFromOther32`.
//!   4. `clap_entry.get_factory("clap.plugin-factory")` → factory pointer.
//!
//! `createPlugin` adds steps 5–7 on top: build `clap_host_t` in plugin
//! memory, write stub indices + identity strings into it, call
//! `factory.create_plugin(factory, host, plugin_id)`. `getInfo` walks the
//! factory's `get_plugin_count` / `get_plugin_descriptor(i)` instead and
//! CBOR-encodes the per-plugin ids back into the JS-visible bytes pool.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
use crate::{
    call::{read_result_u32, write_arg_f64, write_arg_u32, SLOT_SIZE},
    clap, host,
    host::HostStubIndices,
    host_stubs,
    instance::{
        call32, countUntil32, init32, malloc32, memcpyFromOther32, memcpyToOther32, registerHost32,
    },
};

#[allow(dead_code)] // fields read by wasm32 process / mainThread paths.
pub(crate) struct Plugin {
    pub(crate) instance_handle: u32,
    pub(crate) hosted_handle: u32,
    pub(crate) plugin_ptr: u32,
    /// `clap_process_t` allocated in plugin memory by `pluginStart`. Same
    /// struct is reused every block; `pluginProcess` only mutates
    /// `frames_count`.
    pub(crate) process_ptr: u32,
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

// ---------------------------------------------------------------------------
// Plugin-memory helpers
// ---------------------------------------------------------------------------

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

/// Read a NUL-terminated C string out of plugin memory.
///
/// `countUntil32` finds the offset of the terminator inside the plugin's
/// address space (item size 1, sentinel a NUL byte in our host memory);
/// `memcpyFromOther32` copies that many bytes into a fresh host `Vec`.
/// Empty if `plugin_ptr` is 0.
#[cfg(target_arch = "wasm32")]
unsafe fn read_cstr(inst: u32, plugin_ptr: u32, max_len: u32) -> Vec<u8> {
    if plugin_ptr == 0 {
        return Vec::new();
    }
    let nul: u8 = 0;
    let len = countUntil32(inst, plugin_ptr, &nul, 1, max_len);
    let mut buf = alloc::vec![0u8; len as usize];
    if len > 0 {
        memcpyFromOther32(inst, buf.as_mut_ptr(), plugin_ptr, len);
    }
    buf
}

#[cfg(target_arch = "wasm32")]
unsafe fn register_stubs(inst: u32, hosted_handle: u32) -> HostStubIndices {
    let reg = |fn_table_index: u32, sig: &[u8]| -> u32 {
        registerHost32(
            inst,
            hosted_handle, // host_data context echoed to the stub via JS shim binding.
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
        events_in_size: reg(host_stubs::events_in_size_index(), host_stubs::SIG_II),
        events_in_get: reg(host_stubs::events_in_get_index(), host_stubs::SIG_III),
        events_out_try_push: reg(host_stubs::events_out_try_push_index(), host_stubs::SIG_III),
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

// ---------------------------------------------------------------------------
// Shared factory walk
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
struct FactoryWalk {
    inst: u32,
    factory_ptr: u32,
}

#[cfg(target_arch = "wasm32")]
unsafe fn ensure_factory(hosted_handle: u32) -> Option<FactoryWalk> {
    // Cached path — common after the first `getInfo`/`createPlugin`.
    {
        let hosted = host::get(hosted_handle);
        if let Some(factory_ptr) = hosted.factory_ptr {
            return Some(FactoryWalk {
                inst: hosted.instance_handle,
                factory_ptr,
            });
        }
    }

    let inst = host::get(hosted_handle).instance_handle;

    // 1. Pre-register host stubs (must precede init32).
    if host::get(hosted_handle).stubs.is_none() {
        let s = register_stubs(inst, hosted_handle);
        host::get(hosted_handle).stubs = Some(s);
    }

    // 2. init32 → clap_entry pointer.
    let clap_entry_ptr = init32(inst);
    if clap_entry_ptr == 0 {
        return None;
    }

    // 3. clap_entry.init(NULL).
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
        return None;
    }

    // 4. clap_entry.get_factory("clap.plugin-factory") → factory pointer.
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
        return None;
    }

    let hosted = host::get(hosted_handle);
    hosted.entry_ptr = Some(clap_entry_ptr);
    hosted.factory_ptr = Some(factory_ptr);
    Some(FactoryWalk { inst, factory_ptr })
}

// ---------------------------------------------------------------------------
// CBOR encoding (hand-rolled, just enough for `{plugins:[{id:"..."}, ...]}`)
// ---------------------------------------------------------------------------
//
// CBOR major types we use:
//   3 (text-string)  0x60 | len
//   4 (array)        0x80 | len
//   5 (map)          0xa0 | len
// For lengths 24..=255 the count is emitted as a separate u8 with `| 24`,
// 256..=65535 as u16 BE with `| 25`. We never hit those in practice for M1
// id lists, but support them anyway.

fn cbor_uint_header(out: &mut Vec<u8>, major: u8, n: u64) {
    if n < 24 {
        out.push(major | n as u8);
    } else if n < 256 {
        out.push(major | 24);
        out.push(n as u8);
    } else if n < 65536 {
        out.push(major | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(major | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    }
}

fn cbor_text(out: &mut Vec<u8>, s: &[u8]) {
    cbor_uint_header(out, 0x60, s.len() as u64);
    out.extend_from_slice(s);
}

fn cbor_map_header(out: &mut Vec<u8>, n: u64) {
    cbor_uint_header(out, 0xa0, n);
}

fn cbor_array_header(out: &mut Vec<u8>, n: u64) {
    cbor_uint_header(out, 0x80, n);
}

// ---------------------------------------------------------------------------
// JS-facing exports
// ---------------------------------------------------------------------------

/// `getInfo(hostedHandle, bytesHandle) -> u32`
///
/// Enumerate the wclap's plugin factory and publish a CBOR document of the
/// shape `{plugins: [{id: "..."}, ...]}` to the JS-visible bytes pool.
/// Returns the host-memory pointer of the bytes buffer (JS doesn't actually
/// read it — it goes through `getBytesData(handle)` — but the C++ host
/// returns it and `wclap-host-js` tolerates the parity).
///
/// M1-minimum: only `id` per plugin. `name`/`vendor`/`features` etc. land
/// once a downstream caller needs them.
#[no_mangle]
pub extern "C" fn getInfo(hosted_handle: u32, bytes_handle: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let walk = match ensure_factory(hosted_handle) {
            Some(w) => w,
            None => {
                // Publish an empty list so JS sees `info.plugins == []`
                // instead of throwing on the CBOR decode.
                return crate::bytes::set(bytes_handle, &[0xa1, 0x67, b'p', b'l', b'u', b'g', b'i', b'n', b's', 0x80]);
            }
        };

        // `get_plugin_count(factory) -> u32`
        let mut arg = [0u8; SLOT_SIZE];
        let mut result = [0u8; SLOT_SIZE];
        write_arg_u32(&mut arg, walk.factory_ptr);
        call32(
            walk.inst,
            walk.factory_ptr + clap::factory::GET_PLUGIN_COUNT as u32,
            1,
            result.as_mut_ptr(),
            arg.as_ptr(),
            1,
        );
        let plugin_count = read_result_u32(&result);

        // Build CBOR. One outer map (1 entry: "plugins") wrapping an array
        // of per-plugin maps (1 entry: "id").
        let mut cbor = Vec::with_capacity(32 + plugin_count as usize * 64);
        cbor_map_header(&mut cbor, 1);
        cbor_text(&mut cbor, b"plugins");
        cbor_array_header(&mut cbor, plugin_count as u64);

        let mut args2 = [0u8; SLOT_SIZE * 2];
        for i in 0..plugin_count {
            // `get_plugin_descriptor(factory, i) -> descriptor*`
            write_arg_u32((&mut args2[0..SLOT_SIZE]).try_into().unwrap(), walk.factory_ptr);
            write_arg_u32((&mut args2[SLOT_SIZE..]).try_into().unwrap(), i);
            call32(
                walk.inst,
                walk.factory_ptr + clap::factory::GET_PLUGIN_DESCRIPTOR as u32,
                1,
                result.as_mut_ptr(),
                args2.as_ptr(),
                2,
            );
            let desc_ptr = read_result_u32(&result);

            // Read the descriptor's `id` field (offset clap::descriptor::ID,
            // a u32 pointer into plugin memory pointing at a C string).
            let mut id_ptr_bytes = [0u8; 4];
            if desc_ptr != 0 {
                memcpyFromOther32(
                    walk.inst,
                    id_ptr_bytes.as_mut_ptr(),
                    desc_ptr + clap::descriptor::ID as u32,
                    4,
                );
            }
            let id_ptr = u32::from_le_bytes(id_ptr_bytes);
            let id = read_cstr(walk.inst, id_ptr, 256);

            cbor_map_header(&mut cbor, 1);
            cbor_text(&mut cbor, b"id");
            cbor_text(&mut cbor, &id);
        }

        crate::bytes::set(bytes_handle, &cbor)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (hosted_handle, bytes_handle);
        0
    }
}

/// `createPlugin(hostedHandle, pluginIdBytesHandle) -> pluginHandle`
///
/// JS calls `encodeString(pluginId)` → bytes-pool handle holding the raw
/// UTF-8 id (no NUL). We C-string it in plugin memory and pass to
/// `factory.create_plugin`. Returns a Rust-side u32 handle; JS treats it
/// opaquely.
#[no_mangle]
pub extern "C" fn createPlugin(hosted_handle: u32, plugin_id_bytes: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let walk = match ensure_factory(hosted_handle) {
            Some(w) => w,
            None => return 0,
        };

        let stubs = host::get(hosted_handle)
            .stubs
            .expect("host stubs must be registered by ensure_factory");
        let host_ptr = build_host_struct(walk.inst, hosted_handle, &stubs);

        let pid_bytes = crate::bytes::view(plugin_id_bytes);
        let plugin_id_cstr = alloc_cstr(walk.inst, pid_bytes);

        let mut args3 = [0u8; SLOT_SIZE * 3];
        write_arg_u32(
            (&mut args3[0..SLOT_SIZE]).try_into().unwrap(),
            walk.factory_ptr,
        );
        write_arg_u32(
            (&mut args3[SLOT_SIZE..2 * SLOT_SIZE]).try_into().unwrap(),
            host_ptr,
        );
        write_arg_u32(
            (&mut args3[2 * SLOT_SIZE..]).try_into().unwrap(),
            plugin_id_cstr,
        );
        let mut result = [0u8; SLOT_SIZE];
        call32(
            walk.inst,
            walk.factory_ptr + clap::factory::CREATE_PLUGIN as u32,
            1,
            result.as_mut_ptr(),
            args3.as_ptr(),
            3,
        );
        let plugin_ptr = read_result_u32(&result);
        if plugin_ptr == 0 {
            return 0;
        }

        let p = pool();
        let id = p.next_id;
        p.next_id += 1;
        p.map.insert(
            id,
            Plugin {
                instance_handle: walk.inst,
                hosted_handle,
                plugin_ptr,
                process_ptr: 0,
            },
        );
        host::get(hosted_handle).plugins.push(id);
        id
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (hosted_handle, plugin_id_bytes);
        0
    }
}

// ---------------------------------------------------------------------------
// pluginStart / pluginProcess / pluginMainThread (M1 doc steps 6, 7)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
unsafe fn setup_ports(
    inst: u32,
    plugin_ptr: u32,
    audio_ports_ptr: u32,
    info_ptr: u32,
    is_input: u32,
    n_ports: u32,
    audio_buffer_array_base: u32,
    max_frames: u32,
) -> Vec<Vec<u32>> {
    // Returns one inner Vec per port: the per-channel buffer pointers (in
    // plugin memory). These are what JS will wrap in Float32Array views.
    let mut port_channels = Vec::with_capacity(n_ports as usize);
    for i in 0..n_ports {
        // `audio_ports.get(plugin, index, is_input, info_struct)` fills info.
        let mut args4 = [0u8; SLOT_SIZE * 4];
        write_arg_u32((&mut args4[0..SLOT_SIZE]).try_into().unwrap(), plugin_ptr);
        write_arg_u32((&mut args4[SLOT_SIZE..2 * SLOT_SIZE]).try_into().unwrap(), i);
        write_arg_u32(
            (&mut args4[2 * SLOT_SIZE..3 * SLOT_SIZE]).try_into().unwrap(),
            is_input,
        );
        write_arg_u32(
            (&mut args4[3 * SLOT_SIZE..4 * SLOT_SIZE]).try_into().unwrap(),
            info_ptr,
        );
        let mut result = [0u8; SLOT_SIZE];
        call32(
            inst,
            audio_ports_ptr + clap::audio_ports::GET as u32,
            1,
            result.as_mut_ptr(),
            args4.as_ptr(),
            4,
        );

        // Read channel_count from the filled info struct.
        let mut cc_buf = [0u8; 4];
        memcpyFromOther32(
            inst,
            cc_buf.as_mut_ptr(),
            info_ptr + clap::audio_port_info::CHANNEL_COUNT as u32,
            4,
        );
        let channel_count = u32::from_le_bytes(cc_buf);

        // Allocate the per-channel buffer pointer array + each f32 buffer.
        let channel_array = if channel_count > 0 {
            malloc32(inst, channel_count * 4)
        } else {
            0
        };
        let mut channel_ptrs = Vec::with_capacity(channel_count as usize);
        for _ in 0..channel_count {
            let buf = malloc32(inst, max_frames * 4);
            channel_ptrs.push(buf);
        }

        // Write the channel-pointer array into plugin memory.
        if channel_count > 0 {
            let mut ca_bytes: Vec<u8> = Vec::with_capacity(channel_count as usize * 4);
            for &cp in &channel_ptrs {
                ca_bytes.extend_from_slice(&cp.to_le_bytes());
            }
            memcpyToOther32(
                inst,
                channel_array,
                ca_bytes.as_ptr(),
                ca_bytes.len() as u32,
            );
        }

        // Build the audio_buffer struct (24 bytes) and copy it into the
        // host's allocation at `audio_buffer_array_base + i * SIZE`.
        let mut ab = [0u8; clap::audio_buffer::SIZE];
        ab[clap::audio_buffer::DATA32..clap::audio_buffer::DATA32 + 4]
            .copy_from_slice(&channel_array.to_le_bytes());
        // DATA64 stays 0 (we don't host wasm64 plugins).
        ab[clap::audio_buffer::CHANNEL_COUNT..clap::audio_buffer::CHANNEL_COUNT + 4]
            .copy_from_slice(&channel_count.to_le_bytes());
        // LATENCY = 0, CONSTANT_MASK = 0 — pre-zeroed by the array init.
        let ab_dest = audio_buffer_array_base + clap::audio_buffer::SIZE as u32 * i;
        memcpyToOther32(inst, ab_dest, ab.as_ptr(), ab.len() as u32);

        port_channels.push(channel_ptrs);
    }
    port_channels
}

#[cfg(target_arch = "wasm32")]
unsafe fn cbor_port_list(out: &mut Vec<u8>, ports: &[Vec<u32>]) {
    // [[ptrL, ptrR], ...]
    cbor_array_header(out, ports.len() as u64);
    for channels in ports {
        cbor_array_header(out, channels.len() as u64);
        for &ptr in channels {
            // CBOR major 0 (unsigned int) — u32 fits in 4-byte BE form.
            cbor_uint_header(out, 0x00, ptr as u64);
        }
    }
}

/// `pluginStart(pluginHandle, sampleRate, minFrames, maxFrames, bytesHandle)`
///
/// Allocates audio buffers + `clap_audio_buffer` structs + `clap_process` in
/// plugin memory, queries the `clap.audio-ports` extension, calls
/// `clap_plugin.activate` then `start_processing`, and publishes the
/// per-port channel pointer map back to JS as CBOR:
///   `{ inputs: [[ptrL, ptrR], ...], outputs: [[ptrL, ptrR], ...] }`
/// Returns the host-memory pointer of the bytes-pool buffer.
#[no_mangle]
pub extern "C" fn pluginStart(
    plugin_handle: u32,
    sample_rate: f64,
    min_frames: u32,
    max_frames: u32,
    bytes_handle: u32,
) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let (inst, plugin_ptr, hosted_handle) = {
            let p = get(plugin_handle);
            (p.instance_handle, p.plugin_ptr, p.hosted_handle)
        };
        let stubs = host::get(hosted_handle)
            .stubs
            .expect("host stubs must be registered by ensure_factory");

        // 1. plugin.get_extension(plugin, "clap.audio-ports") → audio_ports*
        let ext_id_ptr = alloc_cstr(inst, b"clap.audio-ports");
        let mut args2 = [0u8; SLOT_SIZE * 2];
        write_arg_u32((&mut args2[0..SLOT_SIZE]).try_into().unwrap(), plugin_ptr);
        write_arg_u32((&mut args2[SLOT_SIZE..]).try_into().unwrap(), ext_id_ptr);
        let mut result = [0u8; SLOT_SIZE];
        call32(
            inst,
            plugin_ptr + clap::plugin::GET_EXTENSION as u32,
            1,
            result.as_mut_ptr(),
            args2.as_ptr(),
            2,
        );
        let audio_ports_ptr = read_result_u32(&result);

        // 2. audio_ports.count(plugin, is_input) for each direction.
        let count = |is_input: u32| -> u32 {
            if audio_ports_ptr == 0 {
                return 0;
            }
            let mut a2 = [0u8; SLOT_SIZE * 2];
            write_arg_u32((&mut a2[0..SLOT_SIZE]).try_into().unwrap(), plugin_ptr);
            write_arg_u32((&mut a2[SLOT_SIZE..]).try_into().unwrap(), is_input);
            let mut r = [0u8; SLOT_SIZE];
            call32(
                inst,
                audio_ports_ptr + clap::audio_ports::COUNT as u32,
                1,
                r.as_mut_ptr(),
                a2.as_ptr(),
                2,
            );
            read_result_u32(&r)
        };
        let n_in = count(1);
        let n_out = count(0);

        // 3. Allocate audio_buffer arrays + info scratch.
        let info_ptr = malloc32(inst, clap::audio_port_info::SIZE as u32);
        let audio_inputs = if n_in > 0 {
            malloc32(inst, clap::audio_buffer::SIZE as u32 * n_in)
        } else {
            0
        };
        let audio_outputs = if n_out > 0 {
            malloc32(inst, clap::audio_buffer::SIZE as u32 * n_out)
        } else {
            0
        };
        let in_ptrs = if n_in > 0 && audio_ports_ptr != 0 {
            setup_ports(
                inst,
                plugin_ptr,
                audio_ports_ptr,
                info_ptr,
                1,
                n_in,
                audio_inputs,
                max_frames,
            )
        } else {
            Vec::new()
        };
        let out_ptrs = if n_out > 0 && audio_ports_ptr != 0 {
            setup_ports(
                inst,
                plugin_ptr,
                audio_ports_ptr,
                info_ptr,
                0,
                n_out,
                audio_outputs,
                max_frames,
            )
        } else {
            Vec::new()
        };

        // 4. Build empty in_events / out_events lists in plugin memory.
        let in_events_ptr = malloc32(inst, clap::input_events::SIZE as u32);
        {
            let mut ev = [0u8; clap::input_events::SIZE];
            ev[clap::input_events::SIZE_FN..clap::input_events::SIZE_FN + 4]
                .copy_from_slice(&stubs.events_in_size.to_le_bytes());
            ev[clap::input_events::GET..clap::input_events::GET + 4]
                .copy_from_slice(&stubs.events_in_get.to_le_bytes());
            memcpyToOther32(inst, in_events_ptr, ev.as_ptr(), ev.len() as u32);
        }
        let out_events_ptr = malloc32(inst, clap::output_events::SIZE as u32);
        {
            let mut ev = [0u8; clap::output_events::SIZE];
            ev[clap::output_events::TRY_PUSH..clap::output_events::TRY_PUSH + 4]
                .copy_from_slice(&stubs.events_out_try_push.to_le_bytes());
            memcpyToOther32(inst, out_events_ptr, ev.as_ptr(), ev.len() as u32);
        }

        // 5. Build clap_process struct.
        let process_ptr = malloc32(inst, clap::process::SIZE as u32);
        {
            let mut proc = [0u8; clap::process::SIZE];
            // steady_time = -1 (CLAP convention: unset)
            proc[clap::process::STEADY_TIME..clap::process::STEADY_TIME + 8]
                .copy_from_slice(&(-1i64).to_le_bytes());
            // frames_count set per-block by pluginProcess.
            // transport = 0 (no transport at M1)
            proc[clap::process::AUDIO_INPUTS..clap::process::AUDIO_INPUTS + 4]
                .copy_from_slice(&audio_inputs.to_le_bytes());
            proc[clap::process::AUDIO_OUTPUTS..clap::process::AUDIO_OUTPUTS + 4]
                .copy_from_slice(&audio_outputs.to_le_bytes());
            proc[clap::process::AUDIO_INPUTS_COUNT..clap::process::AUDIO_INPUTS_COUNT + 4]
                .copy_from_slice(&n_in.to_le_bytes());
            proc[clap::process::AUDIO_OUTPUTS_COUNT..clap::process::AUDIO_OUTPUTS_COUNT + 4]
                .copy_from_slice(&n_out.to_le_bytes());
            proc[clap::process::IN_EVENTS..clap::process::IN_EVENTS + 4]
                .copy_from_slice(&in_events_ptr.to_le_bytes());
            proc[clap::process::OUT_EVENTS..clap::process::OUT_EVENTS + 4]
                .copy_from_slice(&out_events_ptr.to_le_bytes());
            memcpyToOther32(inst, process_ptr, proc.as_ptr(), proc.len() as u32);
        }
        get(plugin_handle).process_ptr = process_ptr;

        // 6. plugin.activate(plugin, sample_rate, min_frames, max_frames).
        let mut args4 = [0u8; SLOT_SIZE * 4];
        write_arg_u32((&mut args4[0..SLOT_SIZE]).try_into().unwrap(), plugin_ptr);
        write_arg_f64(
            (&mut args4[SLOT_SIZE..2 * SLOT_SIZE]).try_into().unwrap(),
            sample_rate,
        );
        write_arg_u32(
            (&mut args4[2 * SLOT_SIZE..3 * SLOT_SIZE]).try_into().unwrap(),
            min_frames,
        );
        write_arg_u32(
            (&mut args4[3 * SLOT_SIZE..4 * SLOT_SIZE]).try_into().unwrap(),
            max_frames,
        );
        call32(
            inst,
            plugin_ptr + clap::plugin::ACTIVATE as u32,
            1,
            result.as_mut_ptr(),
            args4.as_ptr(),
            4,
        );
        if read_result_u32(&result) == 0 {
            return crate::bytes::set(bytes_handle, &[]);
        }

        // 7. plugin.start_processing(plugin).
        let mut arg1 = [0u8; SLOT_SIZE];
        write_arg_u32(&mut arg1, plugin_ptr);
        call32(
            inst,
            plugin_ptr + clap::plugin::START_PROCESSING as u32,
            1,
            result.as_mut_ptr(),
            arg1.as_ptr(),
            1,
        );
        // ignore start_processing return for now; some plugins return true,
        // some void-via-bool. If false, the worklet will hear silence rather
        // than crashing.

        // 8. CBOR-encode the channel-pointer map.
        let mut cbor = Vec::with_capacity(128);
        cbor_map_header(&mut cbor, 2);
        cbor_text(&mut cbor, b"inputs");
        cbor_port_list(&mut cbor, &in_ptrs);
        cbor_text(&mut cbor, b"outputs");
        cbor_port_list(&mut cbor, &out_ptrs);
        crate::bytes::set(bytes_handle, &cbor)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (plugin_handle, sample_rate, min_frames, max_frames, bytes_handle);
        0
    }
}

/// `pluginProcess(pluginHandle, framesCount) -> process_status`
///
/// Per-block: stamp `framesCount` into the `clap_process` struct (other
/// fields are stable from `pluginStart`), call `clap_plugin.process`,
/// return the plugin's status int unchanged.
#[no_mangle]
pub extern "C" fn pluginProcess(plugin_handle: u32, frames_count: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let (inst, plugin_ptr, process_ptr) = {
            let p = get(plugin_handle);
            (p.instance_handle, p.plugin_ptr, p.process_ptr)
        };
        if process_ptr == 0 {
            return 0; // not started yet
        }

        // Patch frames_count.
        let fc = frames_count.to_le_bytes();
        memcpyToOther32(
            inst,
            process_ptr + clap::process::FRAMES_COUNT as u32,
            fc.as_ptr(),
            4,
        );

        // plugin.process(plugin, &process)
        let mut args2 = [0u8; SLOT_SIZE * 2];
        write_arg_u32((&mut args2[0..SLOT_SIZE]).try_into().unwrap(), plugin_ptr);
        write_arg_u32(
            (&mut args2[SLOT_SIZE..]).try_into().unwrap(),
            process_ptr,
        );
        let mut result = [0u8; SLOT_SIZE];
        call32(
            inst,
            plugin_ptr + clap::plugin::PROCESS as u32,
            1,
            result.as_mut_ptr(),
            args2.as_ptr(),
            2,
        );
        read_result_u32(&result)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (plugin_handle, frames_count);
        0
    }
}

/// `pluginGetInfo(pluginHandle, bytesHandle) -> u32`
///
/// Per-plugin descriptor metadata. The AWP forwards the decoded result to
/// the main thread, which destructures `{desc, webview, methods}`. Page
/// code only reads `desc.name` and `desc.vendor`; everything else gets a
/// CBOR-null placeholder for now.
#[no_mangle]
pub extern "C" fn pluginGetInfo(plugin_handle: u32, bytes_handle: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let (inst, plugin_ptr) = {
            let p = get(plugin_handle);
            (p.instance_handle, p.plugin_ptr)
        };

        // wclap_plugin.desc (offset 0) is a pointer-to-descriptor in plugin memory.
        let mut desc_p_bytes = [0u8; 4];
        memcpyFromOther32(
            inst,
            desc_p_bytes.as_mut_ptr(),
            plugin_ptr + clap::plugin::DESC as u32,
            4,
        );
        let desc_ptr = u32::from_le_bytes(desc_p_bytes);

        let read_descriptor_str = |field_offset: usize| -> Vec<u8> {
            if desc_ptr == 0 {
                return Vec::new();
            }
            let mut p_bytes = [0u8; 4];
            memcpyFromOther32(
                inst,
                p_bytes.as_mut_ptr(),
                desc_ptr + field_offset as u32,
                4,
            );
            let str_ptr = u32::from_le_bytes(p_bytes);
            read_cstr(inst, str_ptr, 256)
        };
        let id = read_descriptor_str(clap::descriptor::ID);
        let name = read_descriptor_str(clap::descriptor::NAME);
        let vendor = read_descriptor_str(clap::descriptor::VENDOR);

        // CBOR: {desc: {id, name, vendor}, webview: null}
        let mut cbor = Vec::with_capacity(128);
        cbor_map_header(&mut cbor, 2);
        cbor_text(&mut cbor, b"desc");
        cbor_map_header(&mut cbor, 3);
        cbor_text(&mut cbor, b"id");
        cbor_text(&mut cbor, &id);
        cbor_text(&mut cbor, b"name");
        cbor_text(&mut cbor, &name);
        cbor_text(&mut cbor, b"vendor");
        cbor_text(&mut cbor, &vendor);
        cbor_text(&mut cbor, b"webview");
        cbor.push(0xf6); // CBOR null

        crate::bytes::set(bytes_handle, &cbor)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (plugin_handle, bytes_handle);
        0
    }
}

/// `pluginMainThread(pluginHandle)` — called from the AWP for single-threaded
/// plugins after each `process` call (see `clap-audioworkletprocessor.mjs`
/// line 209). Calls `clap_plugin.on_main_thread`.
#[no_mangle]
pub extern "C" fn pluginMainThread(plugin_handle: u32) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let (inst, plugin_ptr) = {
            let p = get(plugin_handle);
            (p.instance_handle, p.plugin_ptr)
        };
        let mut arg = [0u8; SLOT_SIZE];
        write_arg_u32(&mut arg, plugin_ptr);
        let mut result = [0u8; SLOT_SIZE];
        call32(
            inst,
            plugin_ptr + clap::plugin::ON_MAIN_THREAD as u32,
            1,
            result.as_mut_ptr(),
            arg.as_ptr(),
            1,
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = plugin_handle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbor_text_short() {
        let mut out = Vec::new();
        cbor_text(&mut out, b"id");
        assert_eq!(out, [0x62, b'i', b'd']);
    }

    #[test]
    fn cbor_map_one_entry() {
        let mut out = Vec::new();
        cbor_map_header(&mut out, 1);
        cbor_text(&mut out, b"plugins");
        cbor_array_header(&mut out, 0);
        // 0xa1 (map(1)) + 0x67 "plugins" + "plugins" bytes + 0x80 (array(0))
        let mut want = alloc::vec![0xa1, 0x67];
        want.extend_from_slice(b"plugins");
        want.push(0x80);
        assert_eq!(out, want);
    }

    #[test]
    fn cbor_length_24_uses_one_byte_count() {
        let mut out = Vec::new();
        cbor_text(&mut out, &[b'a'; 24]);
        assert_eq!(out[0], 0x60 | 24);
        assert_eq!(out[1], 24);
        assert_eq!(out.len(), 26);
    }
}
