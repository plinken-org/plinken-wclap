//! `_wclapInstanceCreate` + `_wclapInstanceSetPath` — JS bridge calls these
//! before any plugin operation. Together they own the per-plugin `Instance`
//! record whose u32 handle becomes the `handle` argument every
//! `_wclapInstance.*` import receives (see `src/instance.rs`).
//!
//! Flow:
//!   1. JS loads the plugin wasm.
//!   2. JS calls `_wclapInstanceCreate(is64)` here → handle.
//!   3. JS calls `_wclapInstanceSetPath(handle, len)` → host-memory pointer.
//!      JS writes the plugin's path bytes there; we hold them in `path`.
//!   4. JS records the handle in its `#wclapMap`.
//!   5. Host wasm later calls `init32(handle)` / `call32(handle, …)` —
//!      JS uses the handle to dispatch to the right plugin instance.
//!
//! Source of truth: `vendor/wclap-host-js/es6/wclap.mjs` `startWclap`.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

pub(crate) struct Instance {
    /// Plugin path JS handed us. Kept alive so a host-memory pointer to its
    /// bytes (returned by `_wclapInstanceSetPath`) stays valid for the
    /// lifetime of the instance.
    pub(crate) path: Vec<u8>,
}

pub(crate) struct InstancePool {
    next_id: u32,
    map: BTreeMap<u32, Instance>,
}

static mut POOL: InstancePool = InstancePool {
    next_id: 1,
    map: BTreeMap::new(),
};

fn pool() -> &'static mut InstancePool {
    unsafe { &mut *core::ptr::addr_of_mut!(POOL) }
}

#[allow(dead_code)] // first reader arrives once the plugin path matters (M2 bundles).
pub(crate) fn get(handle: u32) -> &'static mut Instance {
    pool().map.get_mut(&handle).expect("bad instance handle")
}

// `is64 != 0` would mean wasm64-pointer plugins, which M1 doesn't host.
// `wclap.mjs` already rejects 64-bit WCLAPs at `startWclap` ("wasm64 WCLAP
// isn't supported yet"), so we should never see is64==1 in practice — but
// be defensive: return 0 to signal "creation failed" if it ever happens.
#[no_mangle]
pub extern "C" fn _wclapInstanceCreate(is64: u32) -> u32 {
    if is64 != 0 {
        return 0;
    }
    let p = pool();
    let id = p.next_id;
    p.next_id += 1;
    p.map.insert(id, Instance { path: Vec::new() });
    id
}

#[no_mangle]
pub extern "C" fn _wclapInstanceSetPath(handle: u32, len: u32) -> u32 {
    let inst = pool().map.get_mut(&handle).expect("bad instance handle");
    inst.path.resize(len as usize, 0);
    inst.path.as_mut_ptr() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_set_path() {
        let h = _wclapInstanceCreate(0);
        assert!(h > 0);

        let ptr = _wclapInstanceSetPath(h, 12);
        assert_ne!(ptr, 0);

        // Path buffer has the right length and is zero-initialised; resize
        // again returns a stable pointer (Vec doesn't reallocate when
        // shrinking and we never grow past the first resize).
        assert_eq!(get(h).path.len(), 12);
        assert!(get(h).path.iter().all(|&b| b == 0));
    }

    #[test]
    fn is64_rejected() {
        assert_eq!(_wclapInstanceCreate(1), 0);
    }
}
