//! Pad model + param-id scheme.
//!
//! The generated tables (`PARAM_DEFS`, `PAD_NOTES`, `NOTE_TO_PAD`) come
//! from build.rs — see the id layout there. Ids are frozen once shipped:
//!
//! - globals: `0` MasterLevel, `1` PadCount (hidden)
//! - per-pad: `0x0100 + pad * 0x10 + offset`, pad ∈ 0..64
//!
//! Param *values* live in the plugin's flat `values` array (indexed by
//! table position, not id) — the single source of truth that state
//! save/load and the UI snapshot read. This module maps between ids,
//! table indices, and typed per-pad reads.

include!(concat!(env!("OUT_DIR"), "/params_gen.rs"));

pub const PADS: usize = 64;
pub const PAD_PARAMS: usize = 12;
pub const PAD_ID_BASE: u32 = 0x0100;
pub const PAD_ID_STRIDE: u32 = 0x10;

/// Per-pad param offsets within a pad's id block.
#[repr(u32)]
#[derive(Clone, Copy)]
pub enum PadParam {
    Level = 0,
    Tune = 1,
    FineTune = 2,
    Pan = 3,
    Attack = 4,
    Decay = 5,
    FilterType = 6,
    Cutoff = 7,
    Resonance = 8,
    MuteGroup = 9,
    OneShot = 10,
    RootKey = 11,
}

/// CLAP param id → index into `PARAM_DEFS` / the values array.
pub fn param_index(id: u32) -> Option<usize> {
    match id {
        0 | 1 => Some(id as usize),
        _ => {
            let rel = id.checked_sub(PAD_ID_BASE)?;
            let pad = (rel / PAD_ID_STRIDE) as usize;
            let off = (rel % PAD_ID_STRIDE) as usize;
            if pad < PADS && off < PAD_PARAMS {
                Some(2 + pad * PAD_PARAMS + off)
            } else {
                None
            }
        }
    }
}

/// Values-array index of one pad param.
#[inline]
pub fn pad_value_index(pad: usize, p: PadParam) -> usize {
    2 + pad * PAD_PARAMS + p as usize
}

/// Pad index for a MIDI key, honouring the (persisted) pad count so notes
/// mapped to not-yet-added pads stay silent.
pub fn pad_for_note(key: i16, pad_count: usize) -> Option<usize> {
    if !(0..=127).contains(&key) {
        return None;
    }
    let pad = NOTE_TO_PAD[key as usize];
    if pad == 0xFF || pad as usize >= pad_count.min(PADS) {
        return None;
    }
    Some(pad as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_table_shape() {
        assert_eq!(PARAM_DEFS.len(), 2 + PADS * PAD_PARAMS);
        // Ids are unique and map back to their own table index.
        for (i, def) in PARAM_DEFS.iter().enumerate() {
            assert_eq!(param_index(def.id), Some(i), "id {:#x}", def.id);
            assert_eq!(def.name.last(), Some(&0u8));
            assert!(def.min <= def.default && def.default <= def.max);
        }
        // Out-of-scheme ids don't resolve.
        assert_eq!(param_index(2), None);
        assert_eq!(param_index(PAD_ID_BASE + 63 * PAD_ID_STRIDE + 12), None);
        assert_eq!(param_index(PAD_ID_BASE + 64 * PAD_ID_STRIDE), None);
    }

    #[test]
    fn bank_a_is_the_classic_mpc_layout() {
        let expect: [u8; 16] = [37, 36, 42, 82, 40, 38, 46, 44, 48, 47, 45, 43, 49, 55, 51, 53];
        assert_eq!(&PAD_NOTES[..16], &expect);
        // Note 36 (kick) → pad A02 (index 1); note 38 (snare) → A06 (5).
        assert_eq!(pad_for_note(36, 16), Some(1));
        assert_eq!(pad_for_note(38, 16), Some(5));
        assert_eq!(pad_for_note(42, 16), Some(2));
        assert_eq!(pad_for_note(46, 16), Some(6));
    }

    #[test]
    fn pad_count_gates_higher_banks() {
        let bank_b_note = PAD_NOTES[16];
        assert_eq!(pad_for_note(bank_b_note as i16, 16), None);
        assert_eq!(pad_for_note(bank_b_note as i16, 32), Some(16));
    }

    #[test]
    fn root_key_defaults_to_trigger_note() {
        for pad in 0..PADS {
            let def = &PARAM_DEFS[pad_value_index(pad, PadParam::RootKey)];
            assert_eq!(def.default as u8, PAD_NOTES[pad]);
        }
    }

    #[test]
    fn no_duplicate_pad_notes() {
        let mut seen = [false; 128];
        for &n in PAD_NOTES.iter() {
            assert!(!seen[n as usize], "duplicate note {n}");
            seen[n as usize] = true;
        }
    }
}
