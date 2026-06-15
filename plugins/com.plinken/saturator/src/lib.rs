//! Saturation — Plinken WCLAP audio-effect plugin.
//!
//! A drive-into-waveshaper saturator with three character curves, a
//! post-shaper tone roll-off, and a dry/wet blend. Hand-rolled DSP (no
//! external graph), sample in / sample out, with a DC blocker on the
//! output so the asymmetric "tube" curve doesn't push an offset into the
//! mix.
//!
//! Curves:
//!   * Tanh  — symmetric soft clip, odd harmonics, the safe default.
//!   * Tube  — asymmetric, adds a 2nd-harmonic sheen (DC removed after).
//!   * Fold  — sine wavefolder; folds back when driven past unity, the
//!             aggressive/synthy option.
//!
//! Params:
//!   * Drive  — pre-shaper gain in dB (0..36); loudness is tamed by a
//!              `drive^-0.4` makeup so turning it up changes tone, not level.
//!   * Type   — curve selector (stepped 0/1/2).
//!   * Tone   — one-pole low-pass after the shaper (500 Hz..18 kHz).
//!   * Mix    — dry/wet blend (0 = bypass tone path, 1 = full wet).
//!   * Output — final trim in dB (−24..+12).

extern crate alloc;

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.saturator\0",
    name: b"Saturation\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.0\0",
    description: b"Waveshaping saturator: tanh / tube / fold, with tone and dry/wet.\0",
    features: &[b"audio-effect\0", b"distortion\0", b"saturation\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

const PID_DRIVE: u32 = 0x0001;
const PID_TYPE: u32 = 0x0002;
const PID_TONE: u32 = 0x0003;
const PID_MIX: u32 = 0x0004;
const PID_OUTPUT: u32 = 0x0005;

const PID_METER_PEAK: u32 = 0x1000;

const DRIVE_MIN: f64 = 0.0;
const DRIVE_MAX: f64 = 36.0;
const DRIVE_DEFAULT: f64 = 6.0;

const TYPE_MIN: f64 = 0.0;
const TYPE_MAX: f64 = 2.0;
const TYPE_DEFAULT: f64 = 0.0;

const TONE_MIN: f64 = 500.0;
const TONE_MAX: f64 = 18000.0;
const TONE_DEFAULT: f64 = 12000.0;

const MIX_MIN: f64 = 0.0;
const MIX_MAX: f64 = 1.0;
const MIX_DEFAULT: f64 = 1.0;

const OUTPUT_MIN: f64 = -24.0;
const OUTPUT_MAX: f64 = 12.0;
const OUTPUT_DEFAULT: f64 = 0.0;

/// Knob-smoothing time constant (de-zipper), seconds.
const SMOOTH_SEC: f32 = 0.020;

static PARAMS: &[ParamDef] = &[
    ParamDef {
        id: PID_DRIVE,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Drive\0",
        module: b"\0",
        min: DRIVE_MIN,
        max: DRIVE_MAX,
        default: DRIVE_DEFAULT,
    },
    ParamDef {
        id: PID_TYPE,
        flags: PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED,
        name: b"Type\0",
        module: b"\0",
        min: TYPE_MIN,
        max: TYPE_MAX,
        default: TYPE_DEFAULT,
    },
    ParamDef {
        id: PID_TONE,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Tone\0",
        module: b"\0",
        min: TONE_MIN,
        max: TONE_MAX,
        default: TONE_DEFAULT,
    },
    ParamDef {
        id: PID_MIX,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Mix\0",
        module: b"\0",
        min: MIX_MIN,
        max: MIX_MAX,
        default: MIX_DEFAULT,
    },
    ParamDef {
        id: PID_OUTPUT,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Output\0",
        module: b"\0",
        min: OUTPUT_MIN,
        max: OUTPUT_MAX,
        default: OUTPUT_DEFAULT,
    },
];

fn db_to_amp(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn amp_to_db(amp: f32) -> f32 {
    if amp <= 1.0e-9 {
        -120.0
    } else {
        20.0 * amp.log10()
    }
}

/// One-pole smoothing coefficient reaching 1/e in `sec`.
fn smooth_coeff(sec: f32, sample_rate: f32) -> f32 {
    if sec <= 0.0 || sample_rate <= 0.0 {
        return 1.0;
    }
    1.0 - (-1.0_f32 / (sec * sample_rate)).exp()
}

/// One-pole low-pass coefficient for cutoff `fc` Hz.
fn lp_coeff(fc: f32, sample_rate: f32) -> f32 {
    if sample_rate <= 0.0 {
        return 1.0;
    }
    let c = 1.0 - (-2.0 * core::f32::consts::PI * fc / sample_rate).exp();
    c.clamp(0.0, 1.0)
}

/// Map a continuous knob value to one of the three curve indices.
fn shape(kind: u8, x: f32) -> f32 {
    match kind {
        // Tanh — symmetric soft clip.
        0 => x.tanh(),
        // Tube — asymmetric; the squared term adds a 2nd harmonic. A DC
        // blocker downstream removes the offset it introduces.
        1 => {
            let t = (x * 1.3).tanh();
            t + 0.15 * t * t
        }
        // Fold — sine wavefolder. |x| > 1 folds back on itself.
        _ => (x * core::f32::consts::FRAC_PI_2).sin(),
    }
}

/// `{params:{<id>:<f64>, …}}` CBOR into a fixed buffer (shared meter wire shape).
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

/// Per-channel post-shaper state: tone low-pass + DC blocker.
#[derive(Clone, Copy, Default)]
struct ChannelState {
    lp: f32,
    dc_x1: f32,
    dc_y1: f32,
}

impl ChannelState {
    #[inline]
    fn process(&mut self, wet_raw: f32, lp_a: f32) -> f32 {
        // Tone roll-off.
        self.lp += (wet_raw - self.lp) * lp_a;
        let lp = self.lp;
        // DC blocker: y[n] = x[n] - x[n-1] + 0.995*y[n-1].
        let y = lp - self.dc_x1 + 0.995 * self.dc_y1;
        self.dc_x1 = lp;
        self.dc_y1 = y;
        y
    }

    fn reset(&mut self) {
        self.lp = 0.0;
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
    }
}

struct Saturator {
    drive_db: f64,
    kind: u8,
    tone_hz: f64,
    mix: f64,
    output_db: f64,

    // Smoothed linear coefficients.
    drive_lin_cur: f32,
    drive_lin_tgt: f32,
    makeup_cur: f32,
    makeup_tgt: f32,
    output_lin_cur: f32,
    output_lin_tgt: f32,
    mix_cur: f32,
    mix_tgt: f32,

    smooth_coeff: f32,
    lp_a: f32,
    sample_rate: f32,

    ch_l: ChannelState,
    ch_r: ChannelState,

    meter_peak: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl Saturator {
    fn recalc(&mut self) {
        let d = db_to_amp(self.drive_db as f32).max(1.0);
        self.drive_lin_tgt = d;
        // Tame loudness as drive rises so it changes tone, not level.
        self.makeup_tgt = d.powf(-0.4);
        self.output_lin_tgt = db_to_amp(self.output_db as f32);
        self.mix_tgt = self.mix as f32;
    }
}

impl Plugin for Saturator {
    fn new() -> Self {
        let sr = 48000.0_f32;
        let mut p = Self {
            drive_db: DRIVE_DEFAULT,
            kind: 0,
            tone_hz: TONE_DEFAULT,
            mix: MIX_DEFAULT,
            output_db: OUTPUT_DEFAULT,
            drive_lin_cur: 1.0,
            drive_lin_tgt: 1.0,
            makeup_cur: 1.0,
            makeup_tgt: 1.0,
            output_lin_cur: 1.0,
            output_lin_tgt: 1.0,
            mix_cur: 1.0,
            mix_tgt: 1.0,
            smooth_coeff: smooth_coeff(SMOOTH_SEC, sr),
            lp_a: lp_coeff(TONE_DEFAULT as f32, sr),
            sample_rate: sr,
            ch_l: ChannelState::default(),
            ch_r: ChannelState::default(),
            meter_peak: 0.0,
            frame_count: 0,
            send_interval_frames: 1600,
        };
        p.recalc();
        p.drive_lin_cur = p.drive_lin_tgt;
        p.makeup_cur = p.makeup_tgt;
        p.output_lin_cur = p.output_lin_tgt;
        p.mix_cur = p.mix_tgt;
        p
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.smooth_coeff = smooth_coeff(SMOOTH_SEC, self.sample_rate);
        self.lp_a = lp_coeff(self.tone_hz as f32, self.sample_rate);
        self.send_interval_frames = (self.sample_rate / 30.0) as u32;
        self.recalc();
        self.drive_lin_cur = self.drive_lin_tgt;
        self.makeup_cur = self.makeup_tgt;
        self.output_lin_cur = self.output_lin_tgt;
        self.mix_cur = self.mix_tgt;
    }

    fn reset(&mut self) {
        self.ch_l.reset();
        self.ch_r.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_DRIVE => self.drive_db,
            PID_TYPE => self.kind as f64,
            PID_TONE => self.tone_hz,
            PID_MIX => self.mix,
            PID_OUTPUT => self.output_db,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_DRIVE => {
                self.drive_db = value.clamp(DRIVE_MIN, DRIVE_MAX);
                self.recalc();
            }
            PID_TYPE => {
                let k = value.clamp(TYPE_MIN, TYPE_MAX).round() as u8;
                self.kind = k;
            }
            PID_TONE => {
                self.tone_hz = value.clamp(TONE_MIN, TONE_MAX);
                self.lp_a = lp_coeff(self.tone_hz as f32, self.sample_rate);
            }
            PID_MIX => {
                self.mix = value.clamp(MIX_MIN, MIX_MAX);
                self.recalc();
            }
            PID_OUTPUT => {
                self.output_db = value.clamp(OUTPUT_MIN, OUTPUT_MAX);
                self.recalc();
            }
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let coeff = self.smooth_coeff;
        let lp_a = self.lp_a;
        let kind = self.kind;
        let drive_tgt = self.drive_lin_tgt;
        let makeup_tgt = self.makeup_tgt;
        let out_tgt = self.output_lin_tgt;
        let mix_tgt = self.mix_tgt;
        let mut drive = self.drive_lin_cur;
        let mut makeup = self.makeup_cur;
        let mut out_g = self.output_lin_cur;
        let mut mix = self.mix_cur;
        let mut peak = self.meter_peak;
        let mut n_processed: u32 = 0;

        if ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2 {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo {
                    input_l,
                    input_r,
                    output_l,
                    output_r,
                } = io;
                let n = input_l.len();
                n_processed = n as u32;
                for f in 0..n {
                    drive += (drive_tgt - drive) * coeff;
                    makeup += (makeup_tgt - makeup) * coeff;
                    out_g += (out_tgt - out_g) * coeff;
                    mix += (mix_tgt - mix) * coeff;

                    let dry_l = input_l[f];
                    let dry_r = input_r[f];
                    let wl = self.ch_l.process(shape(kind, dry_l * drive) * makeup, lp_a);
                    let wr = self.ch_r.process(shape(kind, dry_r * drive) * makeup, lp_a);
                    let yl = (dry_l * (1.0 - mix) + wl * mix) * out_g;
                    let yr = (dry_r * (1.0 - mix) + wr * mix) * out_g;
                    output_l[f] = yl;
                    output_r[f] = yr;
                    let m = yl.abs().max(yr.abs());
                    if m > peak {
                        peak = m;
                    }
                }
            }
        }

        if n_processed == 0 {
            if let Some(io) = ctx.mono_io() {
                let wclap_plugin::MonoIo { input, output } = io;
                n_processed = input.len() as u32;
                for f in 0..input.len() {
                    drive += (drive_tgt - drive) * coeff;
                    makeup += (makeup_tgt - makeup) * coeff;
                    out_g += (out_tgt - out_g) * coeff;
                    mix += (mix_tgt - mix) * coeff;
                    let dry = input[f];
                    let w = self.ch_l.process(shape(kind, dry * drive) * makeup, lp_a);
                    let y = (dry * (1.0 - mix) + w * mix) * out_g;
                    output[f] = y;
                    if y.abs() > peak {
                        peak = y.abs();
                    }
                }
            }
        }

        self.drive_lin_cur = drive;
        self.makeup_cur = makeup;
        self.output_lin_cur = out_g;
        self.mix_cur = mix;
        self.meter_peak = peak;
        self.frame_count += n_processed;
        if self.frame_count >= self.send_interval_frames {
            let mut buf = [0u8; 32];
            let pairs = [(PID_METER_PEAK, amp_to_db(self.meter_peak) as f64)];
            let len = encode_params(&mut buf, &pairs);
            if len > 0 {
                ctx.send_to_ui(&buf[..len]);
            }
            self.meter_peak *= 0.5;
            self.frame_count = 0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<Saturator>(&PLUGIN_DEF);
}
