use alloc::collections::BTreeMap;
use alloc::vec::Vec;

struct BytesPool {
    next_id: u32,
    map: BTreeMap<u32, Vec<u8>>,
}

// Single-threaded by construction: the wasm module runs in one JS context.
// Reconsider when threads land at M7 (will need `wasi.thread-spawn` + a real
// synchronisation primitive).
static mut POOL: BytesPool = BytesPool {
    next_id: 1,
    map: BTreeMap::new(),
};

fn pool() -> &'static mut BytesPool {
    unsafe { &mut *core::ptr::addr_of_mut!(POOL) }
}

#[no_mangle]
pub extern "C" fn createBytes() -> u32 {
    let p = pool();
    let id = p.next_id;
    p.next_id += 1;
    p.map.insert(id, Vec::new());
    id
}

#[no_mangle]
pub extern "C" fn resizeBytes(handle: u32, len: u32) -> u32 {
    let buf = pool().map.get_mut(&handle).expect("bad bytes handle");
    buf.resize(len as usize, 0);
    buf.as_mut_ptr() as u32
}

#[no_mangle]
pub extern "C" fn getBytesData(handle: u32) -> u32 {
    let buf = pool().map.get_mut(&handle).expect("bad bytes handle");
    buf.as_mut_ptr() as u32
}

#[no_mangle]
pub extern "C" fn getBytesLength(handle: u32) -> u32 {
    let buf = pool().map.get(&handle).expect("bad bytes handle");
    buf.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    // One #[test] so the test runner doesn't parallel-mutate the static pool.
    // On native targets the u32 pointers returned by `resizeBytes`/`getBytesData`
    // are truncated (real pointers are 64-bit), so we verify state through the
    // pool directly rather than dereferencing the export return values. The
    // pointer-validity check is gated to wasm32 where the u32 *is* a real ptr.
    #[test]
    fn roundtrip() {
        let a = createBytes();
        let b = createBytes();
        assert_ne!(a, b, "handles must be distinct");
        assert!(a > 0 && b > 0, "handles are 1-based");

        let resized = resizeBytes(a, 4);
        assert_ne!(resized, 0, "resize returns a non-null buffer pointer");
        assert_eq!(getBytesLength(a), 4);

        pool()
            .map
            .get_mut(&a)
            .unwrap()
            .copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let grown = resizeBytes(a, 8);
        assert_ne!(grown, 0);
        assert_eq!(getBytesLength(a), 8);

        let buf = pool().map.get(&a).unwrap();
        assert_eq!(&buf[..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(&buf[4..], &[0, 0, 0, 0]);

        // `b` stays empty + independent.
        assert_eq!(getBytesLength(b), 0);

        #[cfg(target_arch = "wasm32")]
        {
            // Only meaningful when usize == u32: the export's u32 is the live
            // pointer, so successive reads should match the resize result.
            assert_eq!(getBytesData(a), grown);
        }
    }
}
