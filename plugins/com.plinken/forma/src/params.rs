//! Parameter surface for FORMA.
//!
//! Ids are sparse on purpose: globals start at 0, slot A at 100, slot B at
//! 200, each slot using the same offsets. **Ids are frozen once shipped** —
//! they are the keys of saved PLST state and host automation; only append
//! (new globals after the last global, new slot offsets after the last
//! offset).

use wclap_plugin::{ParamDef, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED};

// ---- globals --------------------------------------------------------------
pub const VOLUME: u32 = 0;
/// 0 = slot A only, 1 = slot B only.
pub const FADE: u32 = 1;
/// 0 = crossfade, 1 = ring modulation at the centre of the fade.
pub const MIX_MODE: u32 = 2;
pub const F_CUTOFF: u32 = 3;
pub const F_RES: u32 = 4;
/// 0 = low-pass, 1 = band-pass, 2 = high-pass.
pub const F_MODE: u32 = 5;
pub const ENV_A: u32 = 6;
pub const ENV_D: u32 = 7;
pub const ENV_S: u32 = 8;
pub const ENV_R: u32 = 9;
/// How much note velocity scales level (0 = none, 1 = full).
pub const VEL: u32 = 10;
pub const GLOBAL_COUNT: usize = 11;

// ---- per-slot offsets -----------------------------------------------------
pub const SLOT_A: u32 = 100;
pub const SLOT_B: u32 = 200;

/// 0 = Grain, 1 = Spectral.
pub const ENGINE: u32 = 0;
pub const LEVEL: u32 = 1;
pub const TUNE: u32 = 2;
pub const POSITION: u32 = 3;
/// Read-head speed through the source, in multiples of real time.
pub const SCAN: u32 = 4;
/// Mid/side width: 0 = mono, 1 = as generated, 2 = side doubled.
pub const WIDTH: u32 = 5;
pub const BALANCE: u32 = 6;
/// How much stereo the engine generates (per-grain pan / left-right detune).
pub const SPREAD: u32 = 7;
/// Grain rate in octaves relative to the played note (or C3 with Track off).
pub const G_RATE: u32 = 8;
pub const G_TRACK: u32 = 9;
/// Grain length as a multiple of the grain period.
pub const G_SIZE: u32 = 10;
pub const G_SHAPE: u32 = 11;
pub const G_PITCH: u32 = 12;
pub const G_SCATTER: u32 = 13;
pub const G_JITTER: u32 = 14;
pub const S_PURE: u32 = 15;
pub const S_WARP: u32 = 16;
pub const S_FM: u32 = 17;
pub const S_RATIO: u32 = 18;
pub const S_SMOOTH: u32 = 19;
pub const S_TILT: u32 = 20;
/// Grain playback speed follows the played note relative to the detected root.
pub const G_KEYPITCH: u32 = 21;
pub const SLOT_COUNT: usize = 22;

pub const PARAM_COUNT: usize = GLOBAL_COUNT + 2 * SLOT_COUNT;

const A: u32 = PARAM_IS_AUTOMATABLE;
const S: u32 = PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED;

const fn p(id: u32, flags: u32, name: &'static str, module: &'static str, min: f64, max: f64, default: f64) -> ParamDef {
    ParamDef { id, flags, name: name.as_bytes(), module: module.as_bytes(), min, max, default }
}

macro_rules! slot_params {
    ($base:expr, $tag:literal, $module:literal, $engine:expr, $width:expr) => {
        [
            p($base + ENGINE, S, concat!($tag, " Engine\0"), $module, 0.0, 1.0, $engine),
            p($base + LEVEL, A, concat!($tag, " Level\0"), $module, 0.0, 1.0, 0.8),
            p($base + TUNE, A, concat!($tag, " Tune\0"), $module, -24.0, 24.0, 0.0),
            p($base + POSITION, A, concat!($tag, " Position\0"), $module, 0.0, 1.0, 0.1),
            p($base + SCAN, A, concat!($tag, " Scan\0"), $module, -2.0, 2.0, 0.0),
            p($base + WIDTH, A, concat!($tag, " Width\0"), $module, 0.0, 2.0, $width),
            p($base + BALANCE, A, concat!($tag, " Balance\0"), $module, -1.0, 1.0, 0.0),
            p($base + SPREAD, A, concat!($tag, " Spread\0"), $module, 0.0, 1.0, 0.5),
            p($base + G_RATE, A, concat!($tag, " Grain Rate\0"), $module, -8.0, 2.0, 0.0),
            p($base + G_TRACK, S, concat!($tag, " Grain Key Track\0"), $module, 0.0, 1.0, 1.0),
            p($base + G_SIZE, A, concat!($tag, " Grain Size\0"), $module, 0.25, 16.0, 2.0),
            p($base + G_SHAPE, A, concat!($tag, " Grain Shape\0"), $module, 0.0, 1.0, 0.5),
            p($base + G_PITCH, A, concat!($tag, " Grain Pitch\0"), $module, -24.0, 24.0, 0.0),
            p($base + G_SCATTER, A, concat!($tag, " Grain Scatter\0"), $module, 0.0, 1.0, 0.0),
            p($base + G_JITTER, A, concat!($tag, " Grain Jitter\0"), $module, 0.0, 1.0, 0.0),
            p($base + S_PURE, A, concat!($tag, " Pure\0"), $module, 0.0, 1.0, 0.0),
            p($base + S_WARP, A, concat!($tag, " Warp\0"), $module, 0.0, 1.0, 0.0),
            p($base + S_FM, A, concat!($tag, " FM\0"), $module, 0.0, 1.0, 0.0),
            p($base + S_RATIO, A, concat!($tag, " FM Ratio\0"), $module, 0.5, 8.0, 2.0),
            p($base + S_SMOOTH, A, concat!($tag, " Smooth\0"), $module, 0.0, 1.0, 0.0),
            p($base + S_TILT, A, concat!($tag, " Tilt\0"), $module, -1.0, 1.0, 0.0),
            p($base + G_KEYPITCH, S, concat!($tag, " Grain Key Pitch\0"), $module, 0.0, 1.0, 0.0),
        ]
    };
}

const GLOBALS: [ParamDef; GLOBAL_COUNT] = [
    p(VOLUME, A, "Volume\0", "\0", 0.0, 1.0, 0.7),
    p(FADE, A, "Fade\0", "\0", 0.0, 1.0, 0.5),
    p(MIX_MODE, S, "Mix Mode\0", "\0", 0.0, 1.0, 0.0),
    p(F_CUTOFF, A, "Filter Cutoff\0", "Filter\0", 20.0, 20000.0, 20000.0),
    p(F_RES, A, "Filter Resonance\0", "Filter\0", 0.0, 1.0, 0.0),
    p(F_MODE, S, "Filter Mode\0", "Filter\0", 0.0, 2.0, 0.0),
    p(ENV_A, A, "Attack\0", "Envelope\0", 0.001, 8.0, 0.01),
    p(ENV_D, A, "Decay\0", "Envelope\0", 0.001, 8.0, 0.3),
    p(ENV_S, A, "Sustain\0", "Envelope\0", 0.0, 1.0, 0.8),
    p(ENV_R, A, "Release\0", "Envelope\0", 0.001, 8.0, 0.4),
    p(VEL, A, "Velocity\0", "\0", 0.0, 1.0, 0.5),
];
// Slot A defaults to a centred grain engine, slot B to a wide spectral one.
const SLOT_A_DEFS: [ParamDef; SLOT_COUNT] = slot_params!(SLOT_A, "A", "Slot A\0", 0.0, 0.25);
const SLOT_B_DEFS: [ParamDef; SLOT_COUNT] = slot_params!(SLOT_B, "B", "Slot B\0", 1.0, 1.5);

pub static PARAM_DEFS: [ParamDef; PARAM_COUNT] = {
    let mut out = [const { p(0, 0, "\0", "\0", 0.0, 0.0, 0.0) }; PARAM_COUNT];
    let mut i = 0;
    while i < GLOBAL_COUNT {
        out[i] = copy(&GLOBALS[i]);
        i += 1;
    }
    let mut j = 0;
    while j < SLOT_COUNT {
        out[GLOBAL_COUNT + j] = copy(&SLOT_A_DEFS[j]);
        out[GLOBAL_COUNT + SLOT_COUNT + j] = copy(&SLOT_B_DEFS[j]);
        j += 1;
    }
    out
};

const fn copy(d: &ParamDef) -> ParamDef {
    ParamDef { id: d.id, flags: d.flags, name: d.name, module: d.module, min: d.min, max: d.max, default: d.default }
}

/// Position of param `id` in [`PARAM_DEFS`] (and in the value array).
pub fn index_of(id: u32) -> Option<usize> {
    let (base, off, start) = match id {
        0..=99 => return ((id as usize) < GLOBAL_COUNT).then_some(id as usize),
        100..=199 => (SLOT_A, id - SLOT_A, GLOBAL_COUNT),
        200..=299 => (SLOT_B, id - SLOT_B, GLOBAL_COUNT + SLOT_COUNT),
        _ => return None,
    };
    let _ = base;
    ((off as usize) < SLOT_COUNT).then_some(start + off as usize)
}

/// Index of slot param `offset` for slot 0 (A) or 1 (B).
#[inline]
pub const fn slot_index(slot: usize, offset: u32) -> usize {
    GLOBAL_COUNT + slot * SLOT_COUNT + offset as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_index_round_trips() {
        for (i, d) in PARAM_DEFS.iter().enumerate() {
            assert_eq!(index_of(d.id), Some(i), "id {}", d.id);
            assert!(d.min <= d.default && d.default <= d.max, "id {}", d.id);
            assert_eq!(d.name.last(), Some(&0), "id {} name not NUL-terminated", d.id);
        }
        assert_eq!(index_of(11), None);
        assert_eq!(index_of(122), None);
        assert_eq!(index_of(300), None);
    }
}
