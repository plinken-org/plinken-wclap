//! Compressor — Plinken WCLAP audio-effect plugin.
//!
//! A clean feed-forward stereo dynamics compressor with a soft knee and a
//! log-domain peak detector. Both channels share one envelope (stereo
//! linked) so the image stays put under gain reduction. Hand-rolled DSP,
//! no lookahead — `latency_samples()` is 0.
//!
//! Signal path per frame:
//!   1. detector = max(|L|, |R|) → level in dB
//!   2. static gain reduction from threshold / ratio / soft-knee curve
//!   3. smooth the reduction with attack / release one-poles (in dB)
//!   4. gain = 10^((makeup − reduction)/20), applied to both channels
//!
//! Params: Threshold, Ratio, Attack, Release, Makeup, Knee.

extern crate alloc;

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, PARAM_IS_AUTOMATABLE,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.compressor\0",
    name: b"Compressor\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.0\0",
    description: b"Feed-forward stereo compressor with soft knee and peak detector.\0",
    features: &[b"audio-effect\0", b"compressor\0", b"dynamics\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

const PID_THRESHOLD: u32 = 0x0001;
const PID_RATIO: u32 = 0x0002;
const PID_ATTACK: u32 = 0x0003;
const PID_RELEASE: u32 = 0x0004;
const PID_MAKEUP: u32 = 0x0005;
const PID_KNEE: u32 = 0x0006;

const PID_METER_PEAK: u32 = 0x1000;
const PID_METER_GR: u32 = 0x1001;

const THRESHOLD_MIN: f64 = -48.0;
const THRESHOLD_MAX: f64 = 0.0;
const THRESHOLD_DEFAULT: f64 = -18.0;

const RATIO_MIN: f64 = 1.0;
const RATIO_MAX: f64 = 20.0;
const RATIO_DEFAULT: f64 = 4.0;

const ATTACK_MIN: f64 = 0.1;
const ATTACK_MAX: f64 = 100.0;
const ATTACK_DEFAULT: f64 = 10.0;

const RELEASE_MIN: f64 = 10.0;
const RELEASE_MAX: f64 = 1000.0;
const RELEASE_DEFAULT: f64 = 120.0;

const MAKEUP_MIN: f64 = 0.0;
const MAKEUP_MAX: f64 = 24.0;
const MAKEUP_DEFAULT: f64 = 0.0;

const KNEE_MIN: f64 = 0.0;
const KNEE_MAX: f64 = 24.0;
const KNEE_DEFAULT: f64 = 6.0;

static PARAMS: &[ParamDef] = &[
    ParamDef { id: PID_THRESHOLD, flags: PARAM_IS_AUTOMATABLE, name: b"Threshold\0", module: b"\0", min: THRESHOLD_MIN, max: THRESHOLD_MAX, default: THRESHOLD_DEFAULT },
    ParamDef { id: PID_RATIO, flags: PARAM_IS_AUTOMATABLE, name: b"Ratio\0", module: b"\0", min: RATIO_MIN, max: RATIO_MAX, default: RATIO_DEFAULT },
    ParamDef { id: PID_ATTACK, flags: PARAM_IS_AUTOMATABLE, name: b"Attack\0", module: b"\0", min: ATTACK_MIN, max: ATTACK_MAX, default: ATTACK_DEFAULT },
    ParamDef { id: PID_RELEASE, flags: PARAM_IS_AUTOMATABLE, name: b"Release\0", module: b"\0", min: RELEASE_MIN, max: RELEASE_MAX, default: RELEASE_DEFAULT },
    ParamDef { id: PID_MAKEUP, flags: PARAM_IS_AUTOMATABLE, name: b"Makeup\0", module: b"\0", min: MAKEUP_MIN, max: MAKEUP_MAX, default: MAKEUP_DEFAULT },
    ParamDef { id: PID_KNEE, flags: PARAM_IS_AUTOMATABLE, name: b"Knee\0", module: b"\0", min: KNEE_MIN, max: KNEE_MAX, default: KNEE_DEFAULT },
];

fn amp_to_db(amp: f32) -> f32 {
    if amp <= 1.0e-9 {
        -120.0
    } else {
        20.0 * amp.log10()
    }
}

fn db_to_amp(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Envelope coefficient reaching 1/e in `ms`.
fn env_coeff(ms: f32, sample_rate: f32) -> f32 {
    if ms <= 0.0 || sample_rate <= 0.0 {
        return 0.0;
    }
    (-1.0_f32 / (ms * 0.001 * sample_rate)).exp()
}

fn encode_params(buf: &mut [u8], pairs: &[(u32, f64)]) -> usize {
    if pairs.len() > 23 {
        return 0;
    }
    let needed = 1 + 1 + 6 + 1 + pairs.len() * 14;
    if buf.len() < needed {
        return 0;
    }
    let mut i = 0;
    buf[i] = 0xa1; i += 1;
    buf[i] = 0x66; i += 1;
    buf[i..i + 6].copy_from_slice(b"params"); i += 6;
    buf[i] = 0xa0 | (pairs.len() as u8); i += 1;
    for (id, v) in pairs {
        buf[i] = 0x1a; i += 1;
        buf[i..i + 4].copy_from_slice(&id.to_be_bytes()); i += 4;
        buf[i] = 0xfb; i += 1;
        buf[i..i + 8].copy_from_slice(&v.to_be_bytes()); i += 8;
    }
    i
}

struct Compressor {
    threshold_db: f64,
    ratio: f64,
    attack_ms: f64,
    release_ms: f64,
    makeup_db: f64,
    knee_db: f64,

    attack_coeff: f32,
    release_coeff: f32,
    makeup_db_f32: f32,
    sample_rate: f32,

    // Smoothed gain reduction, in dB (>= 0 = reduction).
    gr_env_db: f32,

    meter_peak: f32,
    meter_gr: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl Compressor {
    /// Static gain reduction (dB, >= 0) for an input `level_db` under the
    /// current threshold/ratio/knee. Standard soft-knee curve.
    #[inline]
    fn static_gr(&self, level_db: f32) -> f32 {
        let t = self.threshold_db as f32;
        let r = self.ratio as f32;
        let w = self.knee_db as f32;
        let over = level_db - t;
        let slope = 1.0 - 1.0 / r;
        if w > 0.0 && over.abs() <= w * 0.5 {
            let x = over + w * 0.5;
            slope * x * x / (2.0 * w)
        } else if over > 0.0 {
            slope * over
        } else {
            0.0
        }
    }

    fn recalc(&mut self) {
        self.attack_coeff = env_coeff(self.attack_ms as f32, self.sample_rate);
        self.release_coeff = env_coeff(self.release_ms as f32, self.sample_rate);
        self.makeup_db_f32 = self.makeup_db as f32;
    }
}

impl Plugin for Compressor {
    fn new() -> Self {
        let sr = 48000.0_f32;
        let mut p = Self {
            threshold_db: THRESHOLD_DEFAULT,
            ratio: RATIO_DEFAULT,
            attack_ms: ATTACK_DEFAULT,
            release_ms: RELEASE_DEFAULT,
            makeup_db: MAKEUP_DEFAULT,
            knee_db: KNEE_DEFAULT,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            makeup_db_f32: 0.0,
            sample_rate: sr,
            gr_env_db: 0.0,
            meter_peak: 0.0,
            meter_gr: 0.0,
            frame_count: 0,
            send_interval_frames: 1600,
        };
        p.recalc();
        p
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.send_interval_frames = (self.sample_rate / 30.0) as u32;
        self.recalc();
    }

    fn reset(&mut self) {
        self.gr_env_db = 0.0;
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_THRESHOLD => self.threshold_db,
            PID_RATIO => self.ratio,
            PID_ATTACK => self.attack_ms,
            PID_RELEASE => self.release_ms,
            PID_MAKEUP => self.makeup_db,
            PID_KNEE => self.knee_db,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_THRESHOLD => self.threshold_db = value.clamp(THRESHOLD_MIN, THRESHOLD_MAX),
            PID_RATIO => self.ratio = value.clamp(RATIO_MIN, RATIO_MAX),
            PID_ATTACK => {
                self.attack_ms = value.clamp(ATTACK_MIN, ATTACK_MAX);
                self.attack_coeff = env_coeff(self.attack_ms as f32, self.sample_rate);
            }
            PID_RELEASE => {
                self.release_ms = value.clamp(RELEASE_MIN, RELEASE_MAX);
                self.release_coeff = env_coeff(self.release_ms as f32, self.sample_rate);
            }
            PID_MAKEUP => {
                self.makeup_db = value.clamp(MAKEUP_MIN, MAKEUP_MAX);
                self.makeup_db_f32 = self.makeup_db as f32;
            }
            PID_KNEE => self.knee_db = value.clamp(KNEE_MIN, KNEE_MAX),
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let atk = self.attack_coeff;
        let rel = self.release_coeff;
        let makeup = self.makeup_db_f32;
        let mut env = self.gr_env_db;
        let mut peak = self.meter_peak;
        let mut gr_meter = self.meter_gr;
        let mut n_processed: u32 = 0;

        // Stereo-linked path.
        if ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2 {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo { input_l, input_r, output_l, output_r } = io;
                let n = input_l.len();
                n_processed = n as u32;
                for f in 0..n {
                    let det = input_l[f].abs().max(input_r[f].abs());
                    let level_db = amp_to_db(det);
                    let target = self.static_gr(level_db);
                    // Attack when reduction must grow, release when it shrinks.
                    let coeff = if target > env { atk } else { rel };
                    env = target + (env - target) * coeff;
                    let gain = db_to_amp(makeup - env);
                    let yl = input_l[f] * gain;
                    let yr = input_r[f] * gain;
                    output_l[f] = yl;
                    output_r[f] = yr;
                    let m = yl.abs().max(yr.abs());
                    if m > peak { peak = m; }
                    if env > gr_meter { gr_meter = env; }
                }
            }
        }

        // Mono fallback.
        if n_processed == 0 {
            if let Some(io) = ctx.mono_io() {
                let wclap_plugin::MonoIo { input, output } = io;
                n_processed = input.len() as u32;
                for f in 0..input.len() {
                    let level_db = amp_to_db(input[f].abs());
                    let target = self.static_gr(level_db);
                    let coeff = if target > env { atk } else { rel };
                    env = target + (env - target) * coeff;
                    let gain = db_to_amp(makeup - env);
                    let y = input[f] * gain;
                    output[f] = y;
                    if y.abs() > peak { peak = y.abs(); }
                    if env > gr_meter { gr_meter = env; }
                }
            }
        }

        self.gr_env_db = env;
        self.meter_peak = peak;
        self.meter_gr = gr_meter;
        self.frame_count += n_processed;
        if self.frame_count >= self.send_interval_frames {
            let mut buf = [0u8; 48];
            let pairs = [
                (PID_METER_PEAK, amp_to_db(self.meter_peak) as f64),
                // GR meter is negative dB (gain change), matching the shared UI.
                (PID_METER_GR, (-self.meter_gr) as f64),
            ];
            let len = encode_params(&mut buf, &pairs);
            if len > 0 {
                ctx.send_to_ui(&buf[..len]);
            }
            self.meter_peak *= 0.5;
            self.meter_gr *= 0.5;
            self.frame_count = 0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<Compressor>(&PLUGIN_DEF);
}
