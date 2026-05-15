//! Vocal Limiter — Plinken WCLAP audio-effect plugin.
//!
//! Brickwall peak limiter with three params:
//!   * Threshold — output ceiling in dBFS (default −1.0)
//!   * Release   — envelope decay time in ms (default 50, log-scaled)
//!   * Stereo Link (bool) — shared envelope vs. per-channel envelope
//!
//! Mono / stereo handling is automatic: we declare a stereo port; if the
//! host gives us a 1-channel buffer (mono track) we process that single
//! channel and ignore the link toggle.

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.vocal-limiter\0",
    name: b"Vocal Limiter\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.0.1\0",
    description: b"Brickwall peak limiter tuned for vocals.\0",
    features: &[b"audio-effect\0", b"limiter\0", b"mastering\0"],
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
const THRESHOLD_MIN: f64 = -24.0;
const THRESHOLD_MAX: f64 = 0.0;
const THRESHOLD_DEFAULT: f64 = -1.0;

const RELEASE_MIN_MS: f64 = 10.0;
const RELEASE_MAX_MS: f64 = 500.0;
const RELEASE_DEFAULT_MS: f64 = 50.0;

// Output trim — post-stage gain. Below this floor we treat as mute so the
// pot's leftmost position is `-∞` (lets you confirm the audio you hear is
// actually coming from this plugin).
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
    // 20 * log10(amp). For tiny amplitudes return a "−∞" sentinel low
    // enough that the UI clamps to the meter floor.
    if amp <= 1.0e-9 {
        -120.0
    } else {
        20.0 * amp.log10()
    }
}

/// Encode `{params:{<id>:<f64>, ...}}` CBOR into a fixed-size buffer.
/// Each pair contributes 14 bytes (id: 5, value: 9).
/// Layout:
///   0xa1 0x66 "params" 0xa{n} [ 0x1a <u32 BE id> 0xfb <f64 BE value> ]×n
fn encode_params2(buf: &mut [u8], id0: u32, v0: f64, id1: u32, v1: f64) -> usize {
    let n = buf.len();
    // We require enough room for the 2-pair shape.
    if n < 1 + 1 + 6 + 1 + 14 + 14 {
        return 0;
    }
    let mut i = 0;
    buf[i] = 0xa1; i += 1;                   // map(1)
    buf[i] = 0x66; i += 1;                   // text(6)
    buf[i..i + 6].copy_from_slice(b"params"); i += 6;
    buf[i] = 0xa2; i += 1;                   // map(2)
    // pair 0
    buf[i] = 0x1a; i += 1;
    buf[i..i + 4].copy_from_slice(&id0.to_be_bytes()); i += 4;
    buf[i] = 0xfb; i += 1;
    buf[i..i + 8].copy_from_slice(&v0.to_be_bytes()); i += 8;
    // pair 1
    buf[i] = 0x1a; i += 1;
    buf[i..i + 4].copy_from_slice(&id1.to_be_bytes()); i += 4;
    buf[i] = 0xfb; i += 1;
    buf[i..i + 8].copy_from_slice(&v1.to_be_bytes()); i += 8;
    i
}

// Readonly param IDs used as the meter channels — UI side has matching
// constants in vocal-limiter/ui/index.html.
const PID_METER_PEAK: u32 = 0x1000;
const PID_METER_GR: u32 = 0x1001;

/// Compute the per-sample release decay multiplier. The envelope follows
/// `env *= release_coeff` each sample; choosing the value so that the
/// envelope drops by ~63% (one time constant) over `release_ms` gives a
/// musically familiar response.
fn release_coeff(release_ms: f32, sample_rate: f32) -> f32 {
    if release_ms <= 0.0 || sample_rate <= 0.0 {
        return 0.0;
    }
    let samples = release_ms * 0.001 * sample_rate;
    (-1.0_f32 / samples).exp()
}

struct VocalLimiter {
    // Param state (set from set_param, read in process()).
    threshold_db: f64,
    release_ms: f64,
    stereo_link: bool,
    output_trim_db: f64,

    // Derived from params — recomputed when params or sample rate change.
    threshold_lin: f32,
    release_coeff: f32,
    output_trim_lin: f32,

    sample_rate: f32,

    // Per-channel envelope (peak follower). Reset on activate.
    env_l: f32,
    env_r: f32,

    // Meter accumulators — hold the peak of input and the worst gain
    // reduction across the block, then flushed to the UI every ~33 ms
    // (frame_count >= send_interval_frames).
    meter_peak: f32,
    meter_min_gain: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl VocalLimiter {
    fn recalc(&mut self) {
        self.threshold_lin = db_to_amp(self.threshold_db as f32);
        self.release_coeff = release_coeff(self.release_ms as f32, self.sample_rate);
        // The pot's leftmost notch reads "-∞" — anything ≤ MUTE_DB is hard
        // mute, otherwise standard dB→linear.
        self.output_trim_lin = if self.output_trim_db <= OUTPUT_TRIM_MUTE_DB {
            0.0
        } else {
            db_to_amp(self.output_trim_db as f32)
        };
    }

    #[inline]
    fn limit_sample(env: &mut f32, threshold: f32, release: f32, x: f32) -> f32 {
        Self::limit_sample_with_gain(env, threshold, release, x).0
    }

    /// Like `limit_sample`, but also returns the gain factor applied. The
    /// meter path uses this so the GR meter can show the lowest gain
    /// reached across a block (= maximum reduction).
    #[inline]
    fn limit_sample_with_gain(
        env: &mut f32,
        threshold: f32,
        release: f32,
        x: f32,
    ) -> (f32, f32) {
        let a = x.abs();
        *env = if a > *env { a } else { *env * release };
        if *env > threshold {
            let g = threshold / *env;
            (x * g, g)
        } else {
            (x, 1.0)
        }
    }
}

impl Plugin for VocalLimiter {
    fn new() -> Self {
        let mut p = Self {
            threshold_db: THRESHOLD_DEFAULT,
            release_ms: RELEASE_DEFAULT_MS,
            stereo_link: true,
            output_trim_db: OUTPUT_TRIM_DEFAULT,
            threshold_lin: 0.0,
            release_coeff: 0.0,
            output_trim_lin: 1.0,
            sample_rate: 48000.0,
            env_l: 0.0,
            env_r: 0.0,
            meter_peak: 0.0,
            meter_min_gain: 1.0,
            frame_count: 0,
            send_interval_frames: 1440, // ~30 Hz @ 48 kHz; updated in activate.
        };
        p.recalc();
        p
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.env_l = 0.0;
        self.env_r = 0.0;
        // Aim for ~30 UI updates per second.
        self.send_interval_frames = (sample_rate as f32 / 30.0) as u32;
        self.recalc();
    }

    fn reset(&mut self) {
        self.env_l = 0.0;
        self.env_r = 0.0;
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
                self.recalc();
            }
            PID_RELEASE => {
                self.release_ms = value.clamp(RELEASE_MIN_MS, RELEASE_MAX_MS);
                self.recalc();
            }
            PID_STEREO_LINK => {
                self.stereo_link = value >= 0.5;
            }
            PID_OUTPUT_TRIM => {
                self.output_trim_db = value.clamp(OUTPUT_TRIM_MIN, OUTPUT_TRIM_MAX);
                self.recalc();
            }
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let threshold = self.threshold_lin;
        let release = self.release_coeff;
        let trim = self.output_trim_lin;
        let mut block_peak = self.meter_peak;
        let mut block_min_gain = self.meter_min_gain;
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
                    for f in 0..n {
                        let a = input_l[f].abs().max(input_r[f].abs());
                        if a > block_peak { block_peak = a; }
                        self.env_l = if a > self.env_l { a } else { self.env_l * release };
                        let g = if self.env_l > threshold {
                            threshold / self.env_l
                        } else {
                            1.0
                        };
                        if g < block_min_gain { block_min_gain = g; }
                        output_l[f] = input_l[f] * g * trim;
                        output_r[f] = input_r[f] * g * trim;
                    }
                    self.env_r = self.env_l;
                } else {
                    for f in 0..n {
                        let al = input_l[f].abs();
                        let ar = input_r[f].abs();
                        if al > block_peak { block_peak = al; }
                        if ar > block_peak { block_peak = ar; }
                        let (yl, gl) = Self::limit_sample_with_gain(
                            &mut self.env_l, threshold, release, input_l[f],
                        );
                        let (yr, gr_) = Self::limit_sample_with_gain(
                            &mut self.env_r, threshold, release, input_r[f],
                        );
                        let gmin = gl.min(gr_);
                        if gmin < block_min_gain { block_min_gain = gmin; }
                        output_l[f] = yl * trim;
                        output_r[f] = yr * trim;
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
                    let a = input[f].abs();
                    if a > block_peak { block_peak = a; }
                    let (y, g) = Self::limit_sample_with_gain(
                        &mut self.env_l, threshold, release, input[f],
                    );
                    if g < block_min_gain { block_min_gain = g; }
                    output[f] = y * trim;
                }
            }
        }

        self.meter_peak = block_peak;
        self.meter_min_gain = block_min_gain;
        self.frame_count += n_processed;
        if self.frame_count >= self.send_interval_frames {
            // Convert to dB for the UI; GR is reported negative.
            let peak_db = amp_to_db(self.meter_peak);
            let gr_db = if self.meter_min_gain >= 0.999_9 {
                0.0
            } else {
                amp_to_db(self.meter_min_gain)
            };
            let mut buf = [0u8; 64];
            let len = encode_params2(
                &mut buf,
                PID_METER_PEAK,
                peak_db as f64,
                PID_METER_GR,
                gr_db as f64,
            );
            if len > 0 {
                ctx.send_to_ui(&buf[..len]);
            }
            // Decay peak quickly for next window so the meter falls when
            // signal drops; reset GR.
            self.meter_peak *= 0.5;
            self.meter_min_gain = 1.0;
            self.frame_count = 0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<VocalLimiter>(&PLUGIN_DEF);
}
