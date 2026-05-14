use alloc::collections::BTreeMap;
use alloc::vec::Vec;

// Per-hosted-plugin bookkeeping. Carries the wclapInstance handle (used as
// `handle` in every `_wclapInstance.*` call), plugin handles created via
// `createPlugin`, and — once the first plugin is built — the four host-stub
// table indices `registerHost32` returned. Stubs are registered lazily so
// hosting zero plugins doesn't grow the plugin's function table.
pub(crate) struct Hosted {
    pub(crate) instance_handle: u32,
    pub(crate) plugins: Vec<u32>,
    pub(crate) stubs: Option<HostStubIndices>,
    /// `clap_entry *` cache. `init32` is one-shot (JS asserts
    /// "WCLAP initialised twice"), so the first caller into the factory
    /// walk — `getInfo` or `createPlugin` — owns the call and caches the
    /// result for the other.
    pub(crate) entry_ptr: Option<u32>,
    /// `clap_plugin_factory *` cache, populated alongside `entry_ptr`.
    pub(crate) factory_ptr: Option<u32>,
}

#[derive(Copy, Clone)]
pub(crate) struct HostStubIndices {
    // clap_host_t callbacks
    pub(crate) get_extension: u32,
    pub(crate) request_restart: u32,
    pub(crate) request_process: u32,
    pub(crate) request_callback: u32,
    // clap_input_events / clap_output_events callbacks (used by pluginStart)
    pub(crate) events_in_size: u32,
    pub(crate) events_in_get: u32,
    pub(crate) events_out_try_push: u32,
    // clap_host_webview.send — plugin → iframe push
    pub(crate) host_webview_send: u32,
}

pub(crate) struct HostedPool {
    next_id: u32,
    map: BTreeMap<u32, Hosted>,
}

static mut POOL: HostedPool = HostedPool {
    next_id: 1,
    map: BTreeMap::new(),
};

pub(crate) fn pool() -> &'static mut HostedPool {
    unsafe { &mut *core::ptr::addr_of_mut!(POOL) }
}

#[allow(dead_code)] // first caller arrives at step 4 (`createPlugin`).
pub(crate) fn get(handle: u32) -> &'static mut Hosted {
    pool().map.get_mut(&handle).expect("bad hosted handle")
}

#[no_mangle]
pub extern "C" fn makeHosted(wclap_instance_ptr: u32) -> u32 {
    let p = pool();
    let id = p.next_id;
    p.next_id += 1;
    p.map.insert(
        id,
        Hosted {
            instance_handle: wclap_instance_ptr,
            plugins: Vec::new(),
            stubs: None,
            entry_ptr: None,
            factory_ptr: None,
        },
    );
    id
}

/// Called when the page is done enumerating a wclap module (see `plugins()`
/// in `clap-audionode.mjs`). M1 just drops the bookkeeping; nothing to call
/// into the plugin — its memory and instance live on inside `wclap-host-js`.
#[no_mangle]
pub extern "C" fn removeHosted(handle: u32) {
    pool().map.remove(&handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_hosted_returns_distinct_nonzero() {
        let a = makeHosted(0x1000);
        let b = makeHosted(0x2000);
        assert_ne!(a, b);
        assert!(a > 0 && b > 0);

        assert_eq!(get(a).instance_handle, 0x1000);
        assert_eq!(get(b).instance_handle, 0x2000);
        assert!(get(a).plugins.is_empty());
    }
}
