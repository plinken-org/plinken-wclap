use alloc::collections::BTreeMap;
use alloc::vec::Vec;

// Per-hosted-plugin bookkeeping. At M1 this only carries the wclapInstance
// handle (used as `handle` in every `_wclapInstance.*` call) and a slot for
// plugins created later in steps 4–7. Host-stub registry indices and the
// clap_host_t scratch area land here at step 4.
#[allow(dead_code)] // fields consumed starting at step 4 (`createPlugin`).
pub(crate) struct Hosted {
    pub(crate) instance_handle: u32,
    pub(crate) plugins: Vec<u32>,
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
        },
    );
    id
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
