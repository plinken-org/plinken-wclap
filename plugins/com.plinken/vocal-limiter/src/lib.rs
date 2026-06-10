//! Vocal Limiter — Plinken WCLAP audio-effect plugin.
//!
//! ZERO-LATENCY feed-back peak limiter for tracking and live monitoring.
//! No lookahead, no delay line — the signal path is gain × input, sample
//! in / sample out, so it is safe in a recording chain (a lookahead
//! limiter delays the signal; for that use `com.plinken.brick-limiter`).
//!
//! Topology: feed-back. The gain computer reads the *output* of the gain
//! stage — when the limited signal still exceeds the ceiling, gain is
//! pulled down with a fast attack one-pole; when it falls below, gain
//! releases toward unity. Because attack is a smoother (not a brickwall),
//! the first half-millisecond of a hard transient can overshoot — a final
//! hard clamp at the ceiling catches exactly those samples. That clamp is
//! what makes the limiter "safe" without lookahead; under normal vocal
//! material it never engages because the envelope gets there first.
//!
//! Four params (same surface as the Brick Limiter, so the shared UI
//! works unchanged):
//!   * Threshold — output ceiling in dBFS (default −1.0)
//!   * Release   — envelope decay time in ms (default 50)
//!   * Stereo Link (bool) — shared envelope across L/R vs. independent
//!   * Output    — post-stage gain in dB (mute floor at −60)

extern crate alloc;

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

/// Attack time of the feed-back gain computer. 0.5 ms reaches ~1/e of the
/// required reduction in half a millisecond — fast enough that the safety
/// clamp only ever touches the first few samples of a hard consonant,
/// slow enough that program material doesn't distort.
const ATTACK_MS: f32 = 0.5;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.vocal-limiter\0",
    name: b"Vocal Limiter\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.2\0",
    description: b"Zero-latency feed-back peak limiter for tracking and vocals.\0",
    features: &[b"audio-effect\0", b"limiter\0", b"vocal\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

// Param IDs — kept stable; saved automation lookups depend on them.
const PID_THRESHOLD: u32 = 0x0001;
const PID_RELEASE: u32 = 0x0002;
const PID_STEREO_LINK: u32 = 0x0003;
const PID_OUTPUT_TRIM: u32 = 0x0004;

// Param ranges + defaults.
const THRESHOLD_MIN: f64 = -40.0;
const THRESHOLD_MAX: f64 = 0.0;
const THRESHOLD_DEFAULT: f64 = -1.0;

/// Gain-coefficient smoothing time constant, in seconds (knob smoothing,
/// not the limiter envelope). Same constant as the Brick Limiter.
const GAIN_SMOOTH_SEC: f32 = 0.030;

const RELEASE_MIN_MS: f64 = 10.0;
const RELEASE_MAX_MS: f64 = 500.0;
const RELEASE_DEFAULT_MS: f64 = 50.0;

const OUTPUT_TRIM_MIN: f64 = -60.0;
const OUTPUT_TRIM_MAX: f64 = 12.0;
const OUTPUT_TRIM_DEFAULT: f64 = 0.0;
const OUTPUT_TRIM_MUTE_DB: f64 = -59.99;

static PARAMS: &[ParamDef] = &[
    ParamDef {
        id: PID_THRESHOLD,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Threshold\0",
        module: b"\0",
        min: THRESHOLD_MIN,
        max: THRESHOLD_MAX,
        default: THRESHOLD_DEFAULT,
    },
    ParamDef {
        id: PID_RELEASE,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Release\0",
        module: b"\0",
        min: RELEASE_MIN_MS,
        max: RELEASE_MAX_MS,
        default: RELEASE_DEFAULT_MS,
    },
    ParamDef {
        id: PID_STEREO_LINK,
        flags: PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED,
        name: b"Stereo Link\0",
        module: b"\0",
        min: 0.0,
        max: 1.0,
        default: 1.0,
    },
    ParamDef {
        id: PID_OUTPUT_TRIM,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Output\0",
        module: b"\0",
        min: OUTPUT_TRIM_MIN,
        max: OUTPUT_TRIM_MAX,
        default: OUTPUT_TRIM_DEFAULT,
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

/// Encode `{params:{<id>:<f64>, ...}}` CBOR into a fixed-size buffer.
/// Same wire shape as the Brick Limiter / shared meter UI.
fn encode_params(buf: &mut [u8], pairs: &[(u32, f64)]) -> usize {
    if pairs.len() > 23 {
        return 0;
    }
    let needed = 1 + 1 + 6 + 1 + pairs.len() * 14;
    if buf.len() < needed {
        return 0;
    }
    let mut i = 0;
    buf[i] = 0xa1; i += 1;                   // map(1)
    buf[i] = 0x66; i += 1;                   // text(6)
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

// Readonly param IDs used as the meter channels — UI side has matching
// constants in vocal-limiter/ui/index.html.
const PID_METER_PEAK: u32 = 0x1000;
const PID_METER_GR: u32 = 0x1001;

/// Feed-back gain computer for one channel group. `gain` is the current
/// reduction factor (≤ 1.0, 1.0 = no reduction) applied to the NEXT
/// sample; the detector reads the gain stage's own output. No buffers,
/// no delay — state is two coefficients and one float.
#[derive(Clone, Copy)]
struct FeedbackEnv {
    gain: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

impl FeedbackEnv {
    fn new(sample_rate: f32, release_ms: f32) -> Self {
        Self {
            gain: 1.0,
            attack_coeff: Self::coeff(ATTACK_MS, sample_rate),
            release_coeff: Self::coeff(release_ms, sample_rate),
        }
    }

    /// One-pole smoothing coefficient reaching 1/e in `ms`.
    fn coeff(ms: f32, sample_rate: f32) -> f32 {
        if ms <= 0.0 || sample_rate <= 0.0 {
            return 1.0;
        }
        1.0 - (-1.0_f32 / (ms * 0.001 * sample_rate)).exp()
    }

    fn set_release(&mut self, release_ms: f32, sample_rate: f32) {
        self.release_coeff = Self::coeff(release_ms, sample_rate);
    }

    fn set_sample_rate(&mut self, sample_rate: f32, release_ms: f32) {
        self.attack_coeff = Self::coeff(ATTACK_MS, sample_rate);
        self.release_coeff = Self::coeff(release_ms, sample_rate);
    }

    fn reset(&mut self) {
        self.gain = 1.0;
    }

    /// Process one frame in the normalized domain (ceiling == 1.0).
    /// `amp` is the peak |input| across the linked channels. Returns the
    /// gain to apply to THIS frame. Feed-back: the decision uses the gain
    /// stage's own output (`amp * gain`).
    #[inline]
    fn tick(&mut self, amp: f32) -> f32 {
        let out = amp * self.gain;
        if out > 1.0 {
            // Output still over ceiling → required gain for ceiling-exact
            // output is gain/out. Fast one-pole toward it (attack).
            let target = self.gain / out;
            self.gain += (target - self.gain) * self.attack_coeff;
        } else {
            // Below ceiling → release toward unity.
            self.gain += (1.0 - self.gain) * self.release_coeff;
            if self.gain > 1.0 {
                self.gain = 1.0;
            }
        }
        self.gain
    }
}

struct VocalLimiter {
    // Param state (set from set_param, read in process()).
    threshold_db: f64,
    release_ms: f64,
    stereo_link: bool,
    output_trim_db: f64,

    // Target gain coefficients — recomputed when params change; the hot
    // loop smooths the *_cur fields toward these (knob de-zipper).
    threshold_lin_tgt: f32,
    inv_threshold_lin_tgt: f32,
    output_trim_lin_tgt: f32,

    threshold_lin_cur: f32,
    inv_threshold_lin_cur: f32,
    output_trim_lin_cur: f32,

    gain_smooth_coeff: f32,
    sample_rate: f32,

    // Feed-back envelopes: one linked (stereo) + two independent (mono /
    // unlinked stereo). Cheap enough to keep all three warm.
    env_linked: FeedbackEnv,
    env_l: FeedbackEnv,
    env_r: FeedbackEnv,

    // Meter accumulators — peak in / peak out across the send window.
    // Sample-aligned here (no lookahead delay), so the GR ratio is exact.
    meter_peak_in: f32,
    meter_peak_out: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl VocalLimiter {
    fn recalc_gains(&mut self) {
        let t = db_to_amp(self.threshold_db as f32);
        self.threshold_lin_tgt = t;
        self.inv_threshold_lin_tgt = if t > 1.0e-6 { 1.0 / t } else { 0.0 };
        self.output_trim_lin_tgt = if self.output_trim_db <= OUTPUT_TRIM_MUTE_DB {
            0.0
        } else {
            db_to_amp(self.output_trim_db as f32)
        };
    }
}

impl Plugin for VocalLimiter {
    fn new() -> Self {
        let sr = 48000.0_f32;
        let r = RELEASE_DEFAULT_MS as f32;
        let t_default = db_to_amp(THRESHOLD_DEFAULT as f32);
        let inv_t_default = if t_default > 1.0e-6 { 1.0 / t_default } else { 0.0 };
        let trim_default = db_to_amp(OUTPUT_TRIM_DEFAULT as f32);
        let mut p = Self {
            threshold_db: THRESHOLD_DEFAULT,
            release_ms: RELEASE_DEFAULT_MS,
            stereo_link: true,
            output_trim_db: OUTPUT_TRIM_DEFAULT,
            threshold_lin_tgt: t_default,
            inv_threshold_lin_tgt: inv_t_default,
            output_trim_lin_tgt: trim_default,
            threshold_lin_cur: t_default,
            inv_threshold_lin_cur: inv_t_default,
            output_trim_lin_cur: trim_default,
            gain_smooth_coeff: 1.0 - (-1.0_f32 / (GAIN_SMOOTH_SEC * sr)).exp(),
            sample_rate: sr,
            env_linked: FeedbackEnv::new(sr, r),
            env_l: FeedbackEnv::new(sr, r),
            env_r: FeedbackEnv::new(sr, r),
            meter_peak_in: 0.0,
            meter_peak_out: 0.0,
            frame_count: 0,
            send_interval_frames: 1440,
        };
        p.recalc_gains();
        p
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.send_interval_frames = (sample_rate as f32 / 30.0) as u32;
        self.gain_smooth_coeff =
            1.0 - (-1.0_f32 / (GAIN_SMOOTH_SEC * (sample_rate as f32))).exp();
        self.recalc_gains();
        self.threshold_lin_cur = self.threshold_lin_tgt;
        self.inv_threshold_lin_cur = self.inv_threshold_lin_tgt;
        self.output_trim_lin_cur = self.output_trim_lin_tgt;
        let sr = self.sample_rate;
        let r = self.release_ms as f32;
        self.env_linked.set_sample_rate(sr, r);
        self.env_l.set_sample_rate(sr, r);
        self.env_r.set_sample_rate(sr, r);
    }

    fn reset(&mut self) {
        self.env_linked.reset();
        self.env_l.reset();
        self.env_r.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_THRESHOLD => self.threshold_db,
            PID_RELEASE => self.release_ms,
            PID_STEREO_LINK => {
                if self.stereo_link {
                    1.0
                } else {
                    0.0
                }
            }
            PID_OUTPUT_TRIM => self.output_trim_db,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_THRESHOLD => {
                self.threshold_db = value.clamp(THRESHOLD_MIN, THRESHOLD_MAX);
                self.recalc_gains();
            }
            PID_RELEASE => {
                self.release_ms = value.clamp(RELEASE_MIN_MS, RELEASE_MAX_MS);
                let sr = self.sample_rate;
                let r = self.release_ms as f32;
                self.env_linked.set_release(r, sr);
                self.env_l.set_release(r, sr);
                self.env_r.set_release(r, sr);
            }
            PID_STEREO_LINK => {
                self.stereo_link = value >= 0.5;
            }
            PID_OUTPUT_TRIM => {
                self.output_trim_db = value.clamp(OUTPUT_TRIM_MIN, OUTPUT_TRIM_MAX);
                self.recalc_gains();
            }
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let coeff = self.gain_smooth_coeff;
        let t_tgt = self.threshold_lin_tgt;
        let inv_tgt = self.inv_threshold_lin_tgt;
        let trim_tgt = self.output_trim_lin_tgt;
        let mut threshold = self.threshold_lin_cur;
        let mut inv_threshold = self.inv_threshold_lin_cur;
        let mut trim = self.output_trim_lin_cur;
        let mut peak_in = self.meter_peak_in;
        let mut peak_out = self.meter_peak_out;
        let mut n_processed: u32 = 0;

        // Stereo path — host gave us 2 input channels.
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
                if self.stereo_link {
                    // Linked: one envelope driven by max(|L|,|R|) — both
                    // channels duck together, image stays put.
                    for f in 0..n {
                        threshold += (t_tgt - threshold) * coeff;
                        inv_threshold += (inv_tgt - inv_threshold) * coeff;
                        trim += (trim_tgt - trim) * coeff;
                        let ul = input_l[f] * inv_threshold;
                        let ur = input_r[f] * inv_threshold;
                        let amp = ul.abs().max(ur.abs());
                        let g = self.env_linked.tick(amp);
                        // Safety clamp: zero-lookahead attack can overshoot
                        // for the first samples of a hard transient.
                        let yl = (ul * g).clamp(-1.0, 1.0) * threshold;
                        let yr = (ur * g).clamp(-1.0, 1.0) * threshold;
                        output_l[f] = yl * trim;
                        output_r[f] = yr * trim;
                        let xi = input_l[f].abs().max(input_r[f].abs());
                        let yi = yl.abs().max(yr.abs());
                        if xi > peak_in { peak_in = xi; }
                        if yi > peak_out { peak_out = yi; }
                    }
                } else {
                    for f in 0..n {
                        threshold += (t_tgt - threshold) * coeff;
                        inv_threshold += (inv_tgt - inv_threshold) * coeff;
                        trim += (trim_tgt - trim) * coeff;
                        let ul = input_l[f] * inv_threshold;
                        let ur = input_r[f] * inv_threshold;
                        let gl = self.env_l.tick(ul.abs());
                        let gr = self.env_r.tick(ur.abs());
                        let yl = (ul * gl).clamp(-1.0, 1.0) * threshold;
                        let yr = (ur * gr).clamp(-1.0, 1.0) * threshold;
                        output_l[f] = yl * trim;
                        output_r[f] = yr * trim;
                        let xi = input_l[f].abs().max(input_r[f].abs());
                        let yi = yl.abs().max(yr.abs());
                        if xi > peak_in { peak_in = xi; }
                        if yi > peak_out { peak_out = yi; }
                    }
                }
            }
        }

        // Mono fallback — single input/output channel.
        if n_processed == 0 {
            if let Some(io) = ctx.mono_io() {
                let wclap_plugin::MonoIo { input, output } = io;
                n_processed = input.len() as u32;
                for f in 0..input.len() {
                    threshold += (t_tgt - threshold) * coeff;
                    inv_threshold += (inv_tgt - inv_threshold) * coeff;
                    trim += (trim_tgt - trim) * coeff;
                    let u = input[f] * inv_threshold;
                    let g = self.env_l.tick(u.abs());
                    let y = (u * g).clamp(-1.0, 1.0) * threshold;
                    output[f] = y * trim;
                    let xi = input[f].abs();
                    let yi = y.abs();
                    if xi > peak_in { peak_in = xi; }
                    if yi > peak_out { peak_out = yi; }
                }
            }
        }

        self.threshold_lin_cur = threshold;
        self.inv_threshold_lin_cur = inv_threshold;
        self.output_trim_lin_cur = trim;

        self.meter_peak_in = peak_in;
        self.meter_peak_out = peak_out;
        self.frame_count += n_processed;
        if self.frame_count >= self.send_interval_frames {
            let peak_db = amp_to_db(self.meter_peak_in);
            let gr_db = if self.meter_peak_in > 1.0e-6 {
                let ratio = (self.meter_peak_out / self.meter_peak_in).min(1.0);
                if ratio >= 0.999_9 {
                    0.0
                } else {
                    amp_to_db(ratio)
                }
            } else {
                0.0
            };
            let mut buf = [0u8; 64];
            let pairs = [
                (PID_METER_PEAK, peak_db as f64),
                (PID_METER_GR, gr_db as f64),
            ];
            let len = encode_params(&mut buf, &pairs);
            if len > 0 {
                ctx.send_to_ui(&buf[..len]);
            }
            self.meter_peak_in *= 0.5;
            self.meter_peak_out *= 0.5;
            self.frame_count = 0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<VocalLimiter>(&PLUGIN_DEF);
}
