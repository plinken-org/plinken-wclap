#![allow(dead_code)] // helpers land in use at step 4b (`createPlugin`).

//! `_wclapInstance.call32` argument-packing helpers.
//!
//! Each argument is a 16-byte slot the JS bridge reads from `argsPtr`:
//!   byte 0    : type tag (0=u32, 1=u64, 2=f32, 3=f64)
//!   bytes 1..8: reserved (ignored by the bridge)
//!   bytes 8.. : little-endian value (4 or 8 bytes wide)
//! The result slot at `resultPtr` is written back in the same layout.
//!
//! Source: `vendor/wclap-host-js/es6/wclap.mjs` `call32:` handler.
//!
//! Args buffers are flat: N args = N * 16 bytes. Result buffer is always 16
//! bytes regardless of return width.

pub const SLOT_SIZE: usize = 16;
pub const TAG_U32: u8 = 0;
pub const TAG_U64: u8 = 1;
pub const TAG_F32: u8 = 2;
pub const TAG_F64: u8 = 3;

pub fn write_arg_u32(slot: &mut [u8; SLOT_SIZE], v: u32) {
    *slot = [0; SLOT_SIZE];
    slot[0] = TAG_U32;
    slot[8..12].copy_from_slice(&v.to_le_bytes());
}

pub fn write_arg_u64(slot: &mut [u8; SLOT_SIZE], v: u64) {
    *slot = [0; SLOT_SIZE];
    slot[0] = TAG_U64;
    slot[8..16].copy_from_slice(&v.to_le_bytes());
}

pub fn write_arg_f32(slot: &mut [u8; SLOT_SIZE], v: f32) {
    *slot = [0; SLOT_SIZE];
    slot[0] = TAG_F32;
    slot[8..12].copy_from_slice(&v.to_le_bytes());
}

pub fn write_arg_f64(slot: &mut [u8; SLOT_SIZE], v: f64) {
    *slot = [0; SLOT_SIZE];
    slot[0] = TAG_F64;
    slot[8..16].copy_from_slice(&v.to_le_bytes());
}

pub fn read_result_u32(slot: &[u8; SLOT_SIZE]) -> u32 {
    debug_assert_eq!(slot[0], TAG_U32, "result is not a u32");
    u32::from_le_bytes(slot[8..12].try_into().unwrap())
}

pub fn read_result_u64(slot: &[u8; SLOT_SIZE]) -> u64 {
    debug_assert_eq!(slot[0], TAG_U64, "result is not a u64");
    u64::from_le_bytes(slot[8..16].try_into().unwrap())
}

pub fn read_result_f32(slot: &[u8; SLOT_SIZE]) -> f32 {
    debug_assert_eq!(slot[0], TAG_F32, "result is not an f32");
    f32::from_le_bytes(slot[8..12].try_into().unwrap())
}

pub fn read_result_f64(slot: &[u8; SLOT_SIZE]) -> f64 {
    debug_assert_eq!(slot[0], TAG_F64, "result is not an f64");
    f64::from_le_bytes(slot[8..16].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_pack_layout() {
        let mut slot = [0u8; SLOT_SIZE];
        write_arg_u32(&mut slot, 0xDEADBEEF);
        assert_eq!(slot[0], TAG_U32);
        assert_eq!(&slot[1..8], &[0; 7], "reserved bytes must be zero");
        assert_eq!(&slot[8..12], &[0xEF, 0xBE, 0xAD, 0xDE]);
        assert_eq!(&slot[12..16], &[0; 4]);
        assert_eq!(read_result_u32(&slot), 0xDEADBEEF);
    }

    #[test]
    fn u64_pack_layout() {
        let mut slot = [0u8; SLOT_SIZE];
        write_arg_u64(&mut slot, 0x0102030405060708);
        assert_eq!(slot[0], TAG_U64);
        assert_eq!(&slot[8..16], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(read_result_u64(&slot), 0x0102030405060708);
    }

    #[test]
    fn f32_pack_layout() {
        let mut slot = [0u8; SLOT_SIZE];
        write_arg_f32(&mut slot, 1.0f32);
        assert_eq!(slot[0], TAG_F32);
        assert_eq!(&slot[8..12], &1.0f32.to_le_bytes());
        assert_eq!(read_result_f32(&slot), 1.0);
    }

    #[test]
    fn f64_pack_layout() {
        let mut slot = [0u8; SLOT_SIZE];
        write_arg_f64(&mut slot, 44100.0f64);
        assert_eq!(slot[0], TAG_F64);
        assert_eq!(&slot[8..16], &44100.0f64.to_le_bytes());
        assert_eq!(read_result_f64(&slot), 44100.0);
    }

    #[test]
    fn rewriting_a_slot_clears_prior_bytes() {
        let mut slot = [0xAAu8; SLOT_SIZE];
        write_arg_u32(&mut slot, 0);
        assert_eq!(slot, [0u8; SLOT_SIZE]);
    }
}
