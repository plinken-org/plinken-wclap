//! Parameter surface for Synome.
//!
//! Ported from the private monorepo's `plugins/Synome/src/params.rs`.
//! **Ids 0–73 are frozen** — they match the shipped `synome.json` UI and
//! saved PLST state blobs; only append.
//!
//! One static table serves both the host's `clap.params` enumeration and
//! the plugin's own clamping/defaults (wclap `ParamDef` carries
//! min/max/default as f64).

use wclap_plugin::{ParamDef, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED};

/// Total number of parameters.
pub const PARAM_COUNT: usize = 74;

/// Parameter indices — match synome.json paramId values.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Param {
    // Oscillator 1 (0-7)
    Osc1Shape = 0,
    Osc1Coarse = 1,
    Osc1Fine = 2,
    Osc1ModLfo = 3,
    Osc1ModEnv = 4,
    Osc1FmAmount = 5,
    Osc1FmEnv = 6,
    Osc1FmLfo = 7,

    // Mix (8-12)
    MixOsc1 = 8,
    MixOsc2 = 9,
    MixNoise = 10,
    MixNoiseType = 11,
    PitchModSource = 12,

    // Oscillator 2 (13-18)
    Osc2Shape = 13,
    Osc2Coarse = 14,
    Osc2Fine = 15,
    Osc2Sync = 16,
    Osc2ModLfo = 17,
    Osc2ModEnv = 18,

    // Filter (19-27)
    FilterMode = 19,
    FilterPole = 20,
    FilterRes = 21,
    FilterFreq = 22,
    FilterEnvOn = 23,
    FilterLfoOn = 24,
    FilterKeytrack = 25,
    FilterEnvAmount = 26,
    FilterLfoAmount = 27,

    // Output (28-33)
    Volume = 28,
    Mono = 29,
    Legato = 30,
    UnisonMode = 31,
    Pan = 32,
    Glide = 33,

    // Filter Envelope (34-37)
    FilterAttack = 34,
    FilterDecay = 35,
    FilterSustain = 36,
    FilterRelease = 37,

    // Amp Envelope (38-41)
    AmpAttack = 38,
    AmpDecay = 39,
    AmpSustain = 40,
    AmpRelease = 41,

    // Mod Envelope (42-45)
    ModAttack = 42,
    ModDecay = 43,
    ModSustain = 44,
    ModRelease = 45,

    // LFO (46-50)
    LfoRate = 46,
    LfoSync = 47,
    LfoShape = 48,
    LfoRetrig = 49,
    LfoDelay = 50,

    // Mod FX (51-54)
    ModFxType = 51,
    ModFxRate = 52,
    ModFxDepth = 53,
    ModFxMix = 54,

    // Space FX (55-60)
    ReverbType = 55,
    FxSync = 56,
    FxTime = 57,
    FxAmount = 58,
    FxFeedback = 59,
    FxDamping = 60,

    // Arpeggiator (61-66) — declared but inert, same as the source synth.
    ArpOn = 61,
    ArpOctave = 62,
    ArpRate = 63,
    ArpMode = 64,
    ArpGate = 65,
    ArpSync = 66,

    // Pitch (67-70)
    PitchBendRange = 67,
    PitchModAmount = 68,
    VibratoRate = 69,
    VibratoDepth = 70,

    // Master (71-73)
    MasterDrive = 71,
    VoiceCount = 72,
    MasterTune = 73,
}

macro_rules! p {
    ($id:expr, $name:literal, $min:expr, $max:expr, $def:expr) => {
        ParamDef {
            id: $id,
            flags: PARAM_IS_AUTOMATABLE,
            name: $name,
            module: b"\0",
            min: $min,
            max: $max,
            default: $def,
        }
    };
    ($id:expr, $name:literal, $min:expr, $max:expr, $def:expr, stepped) => {
        ParamDef {
            id: $id,
            flags: PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED,
            name: $name,
            module: b"\0",
            min: $min,
            max: $max,
            default: $def,
        }
    };
}

/// All parameter definitions — order matches the `Param` enum.
pub static PARAM_DEFS: [ParamDef; PARAM_COUNT] = [
    // Oscillator 1
    p!(0, b"osc1_shape\0", 0.0, 1.0, 0.0),
    p!(1, b"osc1_coarse\0", -24.0, 24.0, 0.0),
    p!(2, b"osc1_fine\0", -100.0, 100.0, 0.0),
    p!(3, b"osc1_mod_lfo\0", 0.0, 1.0, 0.0, stepped),
    p!(4, b"osc1_mod_env\0", 0.0, 1.0, 0.0, stepped),
    p!(5, b"osc1_fm_amount\0", 0.0, 1.0, 0.0),
    p!(6, b"osc1_fm_env\0", 0.0, 1.0, 0.0, stepped),
    p!(7, b"osc1_fm_lfo\0", 0.0, 1.0, 0.0, stepped),
    // Mix
    p!(8, b"mix_osc1\0", 0.0, 1.0, 0.7),
    p!(9, b"mix_osc2\0", 0.0, 1.0, 0.0),
    p!(10, b"mix_noise\0", 0.0, 1.0, 0.0),
    p!(11, b"mix_noise_type\0", 0.0, 1.0, 0.0, stepped),
    p!(12, b"pitch_mod_source\0", 0.0, 1.0, 0.0, stepped),
    // Oscillator 2
    p!(13, b"osc2_shape\0", 0.0, 1.0, 0.0),
    p!(14, b"osc2_coarse\0", -24.0, 24.0, 0.0),
    p!(15, b"osc2_fine\0", -100.0, 100.0, 0.0),
    p!(16, b"osc2_sync\0", 0.0, 1.0, 0.0, stepped),
    p!(17, b"osc2_mod_lfo\0", 0.0, 1.0, 0.0, stepped),
    p!(18, b"osc2_mod_env\0", 0.0, 1.0, 0.0, stepped),
    // Filter
    p!(19, b"filter_mode\0", 0.0, 2.0, 0.0, stepped),
    p!(20, b"filter_pole\0", 0.0, 1.0, 1.0, stepped),
    p!(21, b"filter_res\0", 0.0, 1.0, 0.0),
    p!(22, b"filter_freq\0", 20.0, 20000.0, 20000.0),
    p!(23, b"filter_env_on\0", 0.0, 1.0, 0.0, stepped),
    p!(24, b"filter_lfo_on\0", 0.0, 1.0, 0.0, stepped),
    p!(25, b"filter_keytrack\0", 0.0, 1.0, 0.0),
    p!(26, b"filter_env_amount\0", -1.0, 1.0, 0.5),
    p!(27, b"filter_lfo_amount\0", 0.0, 1.0, 0.0),
    // Output
    p!(28, b"volume\0", 0.0, 1.0, 0.7),
    p!(29, b"mono\0", 0.0, 1.0, 0.0, stepped),
    p!(30, b"legato\0", 0.0, 1.0, 0.0, stepped),
    p!(31, b"unison_mode\0", 0.0, 3.0, 0.0, stepped),
    p!(32, b"pan\0", -1.0, 1.0, 0.0),
    p!(33, b"glide\0", 0.0, 2000.0, 0.0),
    // Filter Envelope
    p!(34, b"filter_attack\0", 0.001, 8.0, 0.01),
    p!(35, b"filter_decay\0", 0.001, 8.0, 0.3),
    p!(36, b"filter_sustain\0", 0.0, 1.0, 0.3),
    p!(37, b"filter_release\0", 0.001, 8.0, 0.3),
    // Amp Envelope
    p!(38, b"amp_attack\0", 0.001, 8.0, 0.01),
    p!(39, b"amp_decay\0", 0.001, 8.0, 0.1),
    p!(40, b"amp_sustain\0", 0.0, 1.0, 0.7),
    p!(41, b"amp_release\0", 0.001, 8.0, 0.3),
    // Mod Envelope
    p!(42, b"mod_attack\0", 0.001, 8.0, 0.01),
    p!(43, b"mod_decay\0", 0.001, 8.0, 0.3),
    p!(44, b"mod_sustain\0", 0.0, 1.0, 0.5),
    p!(45, b"mod_release\0", 0.001, 8.0, 0.3),
    // LFO
    p!(46, b"lfo_rate\0", 0.01, 50.0, 1.0),
    p!(47, b"lfo_sync\0", 0.0, 1.0, 0.0, stepped),
    p!(48, b"lfo_shape\0", 0.0, 4.0, 0.0, stepped),
    p!(49, b"lfo_retrig\0", 0.0, 1.0, 0.0, stepped),
    p!(50, b"lfo_delay\0", 0.0, 2.0, 0.0),
    // Mod FX
    p!(51, b"modfx_type\0", 0.0, 3.0, 0.0, stepped),
    p!(52, b"modfx_rate\0", 0.01, 10.0, 1.0),
    p!(53, b"modfx_depth\0", 0.0, 1.0, 0.5),
    p!(54, b"modfx_mix\0", 0.0, 1.0, 0.5),
    // Space FX
    p!(55, b"reverb_type\0", 0.0, 3.0, 0.0, stepped),
    p!(56, b"fx_sync\0", 0.0, 1.0, 0.0, stepped),
    p!(57, b"fx_time\0", 0.0, 4.0, 0.5),
    p!(58, b"fx_amount\0", 0.0, 1.0, 0.3),
    p!(59, b"fx_feedback\0", 0.0, 0.95, 0.3),
    p!(60, b"fx_damping\0", 0.0, 1.0, 0.5),
    // Arpeggiator
    p!(61, b"arp_on\0", 0.0, 1.0, 0.0, stepped),
    p!(62, b"arp_octave\0", 0.0, 3.0, 0.0, stepped),
    p!(63, b"arp_rate\0", 0.5, 30.0, 4.0),
    p!(64, b"arp_mode\0", 0.0, 3.0, 0.0, stepped),
    p!(65, b"arp_gate\0", 0.1, 1.0, 0.5),
    p!(66, b"arp_sync\0", 0.0, 1.0, 0.0, stepped),
    // Pitch
    p!(67, b"pitch_bend_range\0", 0.0, 24.0, 2.0),
    p!(68, b"pitch_mod_amount\0", 0.0, 24.0, 0.0),
    p!(69, b"vibrato_rate\0", 0.1, 15.0, 5.0),
    p!(70, b"vibrato_depth\0", 0.0, 1.0, 0.0),
    // Master
    p!(71, b"master_drive\0", 0.0, 1.0, 0.0),
    p!(72, b"voice_count\0", 0.0, 3.0, 2.0),
    p!(73, b"master_tune\0", -100.0, 100.0, 0.0),
];

/// Default value for every param slot, as f32 for the DSP array.
pub fn default_values() -> [f32; PARAM_COUNT] {
    core::array::from_fn(|i| PARAM_DEFS[i].default as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sequential_and_count_matches() {
        assert_eq!(PARAM_DEFS.len(), PARAM_COUNT);
        for (i, def) in PARAM_DEFS.iter().enumerate() {
            assert_eq!(def.id as usize, i);
            assert_eq!(def.name.last(), Some(&0u8), "name must be NUL-terminated");
            assert!(def.min <= def.default && def.default <= def.max);
        }
    }
}
