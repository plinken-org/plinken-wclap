//! Brick Limiter — Plinken WCLAP audio-effect plugin.
//!
//! Brickwall lookahead peak limiter built on `fundsp::dynamics::Limiter`.
//! fundsp's limiter clamps at unity (±1.0) and has no threshold input, so
//! we scale around it: `input × (1/T) → limiter → output × T`. The
//! lookahead window is `LOOKAHEAD_MS` (5 ms) and release follows the
//! `Release` parameter.
//!
//! Three params:
//!   * Threshold — output ceiling in dBFS (default −1.0)
//!   * Release   — envelope decay time in ms (default 50, log-scaled)
//!   * Stereo Link (bool) — `Limiter<U2>` (linked) vs. two `Limiter<U1>`
//!   * Output    — post-stage gain in dB (mute floor at −60)
//!
//! Mono / stereo handling is automatic: if the host gives us 1-channel
//! buffers we use the mono limiter and the link toggle is ignored.

use fundsp::dynamics::{Maximum, ReduceBuffer};

extern crate alloc;

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

/// Lookahead window for fundsp's Limiter (also the attack time of its
/// internal AFollow envelope). 2 ms suits vocal duty — long enough for
/// the lookahead reducer to catch transients before they pass, which is
/// exactly the mastering-brick trade: true peak ceiling in exchange for
/// 2 ms of latency. Do NOT use on live-monitored tracking chains.
const LOOKAHEAD_MS: f32 = 2.0;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.brick-limiter\0",
    name: b"Brick Limiter\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.0.4\0",
    description: b"Lookahead brickwall peak limiter for buses and master (2 ms lookahead latency).\0",
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
const THRESHOLD_MIN: f64 = -40.0;
const THRESHOLD_MAX: f64 = 0.0;
const THRESHOLD_DEFAULT: f64 = -1.0;

/// Gain-coefficient smoothing time constant, in seconds. ~30 ms is fast
/// enough that knob movements feel responsive and slow enough that they
/// don't introduce audible zipper. The per-sample one-pole coefficient
/// is `1 - exp(-1 / (TIME * sample_rate))`.
const GAIN_SMOOTH_SEC: f32 = 0.030;

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
/// Each pair contributes 14 bytes (id: 5, value: 9). Up to 23 pairs
/// (short-form map header). Returns 0 if the buffer can't hold them.
/// Layout:
///   0xa1 0x66 "params" 0xa{n} [ 0x1a <u32 BE id> 0xfb <f64 BE value> ]×n
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

struct BrickLimiter {
    // Param state (set from set_param, read in process()).
    threshold_db: f64,
    release_ms: f64,
    stereo_link: bool,
    output_trim_db: f64,

    // Target gain coefficients — recomputed when params change. The hot
    // loop smooths the *_cur fields toward these so live knob movements
    // don't produce zipper noise.
    threshold_lin_tgt: f32,
    inv_threshold_lin_tgt: f32,
    output_trim_lin_tgt: f32,

    // Current (smoothed) gain coefficients — updated per sample.
    threshold_lin_cur: f32,
    inv_threshold_lin_cur: f32,
    output_trim_lin_cur: f32,

    // Per-sample one-pole smoothing coefficient, derived from sample rate
    // and GAIN_SMOOTH_SEC in activate(). `cur += (tgt - cur) * coeff`.
    gain_smooth_coeff: f32,

    sample_rate: f32,

    // Lookahead limiter chains. Three instances so we can route by
    // channel-layout / link mode without recompiling on the fly:
    //   * `lim_stereo` — used when stereo + link (single envelope shared
    //     across L/R; reducer sees max(|L|,|R|))
    //   * `lim_mono_l` / `lim_mono_r` — used for mono input *and* for
    //     stereo + unlinked (each channel limits independently)
    // Owning our own implementation lets us live-tune release via a
    // one-pole coefficient instead of rebuilding the lookahead buffer.
    lim_stereo: LookaheadLimiter,
    lim_mono_l: LookaheadLimiter,
    lim_mono_r: LookaheadLimiter,

    // Meter accumulators — running peak of input and of limiter output
    // (pre-trim) across the send window. We derive GR from the ratio of
    // these two peaks rather than per-sample, because fundsp's lookahead
    // delays output by ~5 ms relative to input, so sample-aligned ratios
    // collapse to ~0 during the lookahead fill and pin the GR meter to
    // its floor. Flushed every ~33 ms (frame_count ≥ send_interval).
    meter_peak_in: f32,
    meter_peak_out: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

/// Compact in-house lookahead brickwall limiter. Roughly the same shape
/// as `fundsp::dynamics::Limiter`, but with:
///   * `set_release()` so the release time can be live-tuned without
///     rebuilding the lookahead buffer (the source of the click/zipper
///     when twisting the Release knob),
///   * channel count chosen at construction so a single concrete type
///     handles both mono and stereo without `Box<dyn AudioUnit>` (which
///     LTO breaks — see CLAUDE.md "No Box<dyn AudioUnit>").
struct LookaheadLimiter {
    /// Per-frame delay line, interleaved `[c0, c1, c0, c1, ...]`. Length
    /// is `lookahead_samples * channels`.
    buffer: alloc::vec::Vec<f32>,
    channels: usize,
    lookahead_samples: usize,
    /// Sliding `max(|amp|)` over the lookahead window (fundsp's segment
    /// tree). Same length as the delay buffer in frames.
    reducer: ReduceBuffer<f32, Maximum<f32>>,
    /// Envelope follower (one-pole) over the reducer's output. Instant
    /// attack within the lookahead window, exponential release.
    env: f32,
    release_coeff: f32,
    /// Ring index into `buffer` and `reducer`.
    idx: usize,
    /// Number of frames written so far during initial fill. Once `>=
    /// lookahead_samples` we start producing real output.
    fill: usize,
}

impl LookaheadLimiter {
    fn new(sample_rate: f32, lookahead_ms: f32, release_ms: f32, channels: usize) -> Self {
        let n = ((lookahead_ms * 0.001 * sample_rate).max(1.0)) as usize;
        Self {
            buffer: alloc::vec![0.0; n * channels],
            channels,
            lookahead_samples: n,
            reducer: ReduceBuffer::new(n, Maximum::new()),
            env: 1.0,
            release_coeff: Self::calc_release_coeff(release_ms, sample_rate),
            idx: 0,
            fill: 0,
        }
    }

    /// One-pole release coefficient. `env = env*c + target*(1-c)` reaches
    /// 1/e of the way to `target` after `release_ms` of decay.
    fn calc_release_coeff(release_ms: f32, sample_rate: f32) -> f32 {
        if release_ms <= 0.0 || sample_rate <= 0.0 {
            return 0.0;
        }
        let samples = release_ms * 0.001 * sample_rate;
        (-1.0_f32 / samples).exp()
    }

    fn set_release(&mut self, release_ms: f32, sample_rate: f32) {
        self.release_coeff = Self::calc_release_coeff(release_ms, sample_rate);
    }

    fn reset(&mut self) {
        for x in self.buffer.iter_mut() {
            *x = 0.0;
        }
        self.reducer.clear();
        self.env = 1.0;
        self.idx = 0;
        self.fill = 0;
    }

    /// Push one frame of input, write the corresponding (delayed) limited
    /// output frame. `input.len()` and `output.len()` must equal
    /// `self.channels`.
    #[inline]
    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        // Current frame's amplitude (max across channels — peak-coupled).
        let mut amp = 0.0_f32;
        for &x in input {
            let a = x.abs();
            if a > amp { amp = a; }
        }
        self.reducer.set(self.idx, amp);

        // Read delayed sample then overwrite slot with new input.
        let base = self.idx * self.channels;
        for c in 0..self.channels {
            output[c] = self.buffer[base + c];
            self.buffer[base + c] = input[c];
        }

        // 10 % headroom — fundsp uses this; it slightly pre-attenuates so
        // the brickwall doesn't bump against the ceiling under sustained
        // peaks.
        let target = (self.reducer.total() * 1.10).max(1.0);
        if target > self.env {
            // Instant attack — the lookahead has already given us the
            // peak, so we slam to it. The release filter handles smoothness
            // on the way down.
            self.env = target;
        } else {
            self.env = self.env * self.release_coeff
                + target * (1.0 - self.release_coeff);
        }

        // Gain reduction: divide delayed output by smoothed envelope.
        // During initial fill, output is zero anyway (buffer is silent),
        // so this is safe; we just bump the fill counter.
        let gain = 1.0 / self.env.max(1.0e-6);
        for c in 0..self.channels {
            output[c] *= gain;
        }

        self.idx = (self.idx + 1) % self.lookahead_samples;
        if self.fill < self.lookahead_samples {
            self.fill += 1;
        }
    }
}

fn make_limiter(sr: f32, release_ms: f32, channels: usize) -> LookaheadLimiter {
    LookaheadLimiter::new(sr, LOOKAHEAD_MS, release_ms, channels)
}

impl BrickLimiter {
    fn recalc_gains(&mut self) {
        let t = db_to_amp(self.threshold_db as f32);
        self.threshold_lin_tgt = t;
        self.inv_threshold_lin_tgt = if t > 1.0e-6 { 1.0 / t } else { 0.0 };
        // The pot's leftmost notch reads "-∞" — anything ≤ MUTE_DB is hard
        // mute, otherwise standard dB→linear.
        self.output_trim_lin_tgt = if self.output_trim_db <= OUTPUT_TRIM_MUTE_DB {
            0.0
        } else {
            db_to_amp(self.output_trim_db as f32)
        };
    }

    /// Rebuild all three limiter chains. Called from activate (sample-rate
    /// change) — lookahead buffer length depends on sample rate, so this
    /// reset is unavoidable there. NOT called on Release knob movement;
    /// that path uses `set_release` instead so the buffer keeps running.
    fn rebuild_chains(&mut self) {
        let sr = self.sample_rate;
        let r = self.release_ms as f32;
        self.lim_stereo = make_limiter(sr, r, 2);
        self.lim_mono_l = make_limiter(sr, r, 1);
        self.lim_mono_r = make_limiter(sr, r, 1);
    }
}

impl Plugin for BrickLimiter {
    fn new() -> Self {
        let sr_f32 = 48000.0_f32;
        let r_default = RELEASE_DEFAULT_MS as f32;
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
            gain_smooth_coeff: 1.0 - (-1.0_f32 / (GAIN_SMOOTH_SEC * sr_f32)).exp(),
            sample_rate: sr_f32,
            lim_stereo: make_limiter(sr_f32, r_default, 2),
            lim_mono_l: make_limiter(sr_f32, r_default, 1),
            lim_mono_r: make_limiter(sr_f32, r_default, 1),
            meter_peak_in: 0.0,
            meter_peak_out: 0.0,
            frame_count: 0,
            send_interval_frames: 1440, // ~30 Hz @ 48 kHz; updated in activate.
        };
        p.recalc_gains();
        p
    }

    /// The lookahead delay is real latency the host must compensate.
    /// EXACTLY the buffer length `Lookahead::new` allocates, so the
    /// reported value and the actual delay can't drift apart.
    fn latency_samples(&self) -> u32 {
        ((LOOKAHEAD_MS * 0.001 * self.sample_rate).max(1.0)) as u32
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        // Aim for ~30 UI updates per second.
        self.send_interval_frames = (sample_rate as f32 / 30.0) as u32;
        // Re-derive the smoothing coefficient against the host's sample rate.
        self.gain_smooth_coeff =
            1.0 - (-1.0_f32 / (GAIN_SMOOTH_SEC * (sample_rate as f32))).exp();
        self.recalc_gains();
        // Snap the current coefficients to their targets on activate so the
        // first audio block doesn't ramp up from stale values.
        self.threshold_lin_cur = self.threshold_lin_tgt;
        self.inv_threshold_lin_cur = self.inv_threshold_lin_tgt;
        self.output_trim_lin_cur = self.output_trim_lin_tgt;
        self.rebuild_chains();
    }

    fn reset(&mut self) {
        self.lim_stereo.reset();
        self.lim_mono_l.reset();
        self.lim_mono_r.reset();
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
                // Live-tune the release coefficient on all three limiters
                // without touching their lookahead buffers — that's what
                // used to cause the click/zipper on Release-knob movement.
                let sr = self.sample_rate;
                let r = self.release_ms as f32;
                self.lim_stereo.set_release(r, sr);
                self.lim_mono_l.set_release(r, sr);
                self.lim_mono_r.set_release(r, sr);
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
                let mut frame_in = [0.0_f32; 2];
                let mut frame_out = [0.0_f32; 2];
                if self.stereo_link {
                    // Shared stereo limiter — reducer sees max(|L|,|R|) so
                    // both channels duck together.
                    for f in 0..n {
                        threshold += (t_tgt - threshold) * coeff;
                        inv_threshold += (inv_tgt - inv_threshold) * coeff;
                        trim += (trim_tgt - trim) * coeff;
                        frame_in[0] = input_l[f] * inv_threshold;
                        frame_in[1] = input_r[f] * inv_threshold;
                        self.lim_stereo.tick(&frame_in, &mut frame_out);
                        let yl = frame_out[0] * threshold;
                        let yr = frame_out[1] * threshold;
                        output_l[f] = yl * trim;
                        output_r[f] = yr * trim;
                        let xi = input_l[f].abs().max(input_r[f].abs());
                        let yi = yl.abs().max(yr.abs());
                        if xi > peak_in { peak_in = xi; }
                        if yi > peak_out { peak_out = yi; }
                    }
                } else {
                    // Two independent mono limiters — each channel sees
                    // only its own envelope.
                    let mut in1 = [0.0_f32; 1];
                    let mut out1 = [0.0_f32; 1];
                    for f in 0..n {
                        threshold += (t_tgt - threshold) * coeff;
                        inv_threshold += (inv_tgt - inv_threshold) * coeff;
                        trim += (trim_tgt - trim) * coeff;
                        in1[0] = input_l[f] * inv_threshold;
                        self.lim_mono_l.tick(&in1, &mut out1);
                        let yl = out1[0] * threshold;
                        in1[0] = input_r[f] * inv_threshold;
                        self.lim_mono_r.tick(&in1, &mut out1);
                        let yr = out1[0] * threshold;
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
                let mut in1 = [0.0_f32; 1];
                let mut out1 = [0.0_f32; 1];
                for f in 0..input.len() {
                    threshold += (t_tgt - threshold) * coeff;
                    inv_threshold += (inv_tgt - inv_threshold) * coeff;
                    trim += (trim_tgt - trim) * coeff;
                    in1[0] = input[f] * inv_threshold;
                    self.lim_mono_l.tick(&in1, &mut out1);
                    let y = out1[0] * threshold;
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
            // GR = how much the *limiter* pulled down. Ratio of output peak
            // to input peak, clamped to ≤1 (positive numerator only — never
            // report "gain increase"). 0 dB when input is silent or signal
            // never reaches threshold.
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
            // Decay peaks between sends so meters fall when signal drops.
            self.meter_peak_in *= 0.5;
            self.meter_peak_out *= 0.5;
            self.frame_count = 0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<BrickLimiter>(&PLUGIN_DEF);
}
