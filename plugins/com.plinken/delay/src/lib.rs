//! Delay — Plinken WCLAP audio-effect plugin.
//!
//! A stereo echo with feedback, a tone (low-pass) roll-off inside the
//! feedback loop, dry/wet mix, and an optional ping-pong cross-feed.
//! Fractional delay is read with linear interpolation, and the delay time
//! is smoothed so moving the knob glides (tape-style) instead of clicking.
//!
//! Params: Time, Feedback, Tone, Mix, Ping-Pong.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, PARAM_IS_AUTOMATABLE,
    PARAM_IS_STEPPED,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.delay\0",
    name: b"Delay\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.0\0",
    description: b"Stereo delay with feedback, feedback-path tone roll-off, and ping-pong.\0",
    features: &[b"audio-effect\0", b"delay\0", b"echo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

const PID_TIME: u32 = 0x0001;
const PID_FEEDBACK: u32 = 0x0002;
const PID_TONE: u32 = 0x0003;
const PID_MIX: u32 = 0x0004;
const PID_PINGPONG: u32 = 0x0005;

const PID_METER_PEAK: u32 = 0x1000;

const TIME_MIN: f64 = 1.0;
const TIME_MAX: f64 = 2000.0;
const TIME_DEFAULT: f64 = 350.0;

const FEEDBACK_MIN: f64 = 0.0;
const FEEDBACK_MAX: f64 = 0.95;
const FEEDBACK_DEFAULT: f64 = 0.4;

const TONE_MIN: f64 = 200.0;
const TONE_MAX: f64 = 18000.0;
const TONE_DEFAULT: f64 = 6000.0;

const MIX_MIN: f64 = 0.0;
const MIX_MAX: f64 = 1.0;
const MIX_DEFAULT: f64 = 0.3;

static PARAMS: &[ParamDef] = &[
    ParamDef { id: PID_TIME, flags: PARAM_IS_AUTOMATABLE, name: b"Time\0", module: b"\0", min: TIME_MIN, max: TIME_MAX, default: TIME_DEFAULT },
    ParamDef { id: PID_FEEDBACK, flags: PARAM_IS_AUTOMATABLE, name: b"Feedback\0", module: b"\0", min: FEEDBACK_MIN, max: FEEDBACK_MAX, default: FEEDBACK_DEFAULT },
    ParamDef { id: PID_TONE, flags: PARAM_IS_AUTOMATABLE, name: b"Tone\0", module: b"\0", min: TONE_MIN, max: TONE_MAX, default: TONE_DEFAULT },
    ParamDef { id: PID_MIX, flags: PARAM_IS_AUTOMATABLE, name: b"Mix\0", module: b"\0", min: MIX_MIN, max: MIX_MAX, default: MIX_DEFAULT },
    ParamDef { id: PID_PINGPONG, flags: PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED, name: b"Ping-Pong\0", module: b"\0", min: 0.0, max: 1.0, default: 0.0 },
];

fn amp_to_db(amp: f32) -> f32 {
    if amp <= 1.0e-9 { -120.0 } else { 20.0 * amp.log10() }
}

fn lp_coeff(fc: f32, sample_rate: f32) -> f32 {
    if sample_rate <= 0.0 { return 1.0; }
    (1.0 - (-2.0 * core::f32::consts::PI * fc / sample_rate).exp()).clamp(0.0, 1.0)
}

fn encode_params(buf: &mut [u8], pairs: &[(u32, f64)]) -> usize {
    if pairs.len() > 23 { return 0; }
    let needed = 1 + 1 + 6 + 1 + pairs.len() * 14;
    if buf.len() < needed { return 0; }
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

/// Read a fractional-sample-delayed value with linear interpolation.
#[inline]
fn read_interp(buf: &[f32], write_idx: usize, delay: f32) -> f32 {
    let len = buf.len();
    if len == 0 {
        return 0.0;
    }
    let rp = write_idx as f32 - delay;
    // Wrap into [0, len).
    let mut rp = rp % len as f32;
    if rp < 0.0 {
        rp += len as f32;
    }
    let i0 = rp.floor() as usize % len;
    let i1 = (i0 + 1) % len;
    let frac = rp - rp.floor();
    buf[i0] * (1.0 - frac) + buf[i1] * frac
}

struct Delay {
    time_ms: f64,
    feedback: f64,
    tone_hz: f64,
    mix: f64,
    pingpong: bool,

    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write_idx: usize,
    delay_cur: f32,
    delay_tgt: f32,
    lp_l: f32,
    lp_r: f32,
    tone_a: f32,
    sample_rate: f32,

    meter_peak: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl Delay {
    fn delay_samples_for(&self, ms: f64) -> f32 {
        (ms as f32 * 0.001 * self.sample_rate).max(1.0)
    }
}

impl Plugin for Delay {
    fn new() -> Self {
        Self {
            time_ms: TIME_DEFAULT,
            feedback: FEEDBACK_DEFAULT,
            tone_hz: TONE_DEFAULT,
            mix: MIX_DEFAULT,
            pingpong: false,
            buf_l: Vec::new(),
            buf_r: Vec::new(),
            write_idx: 0,
            delay_cur: 0.0,
            delay_tgt: 0.0,
            lp_l: 0.0,
            lp_r: 0.0,
            tone_a: 1.0,
            sample_rate: 48000.0,
            meter_peak: 0.0,
            frame_count: 0,
            send_interval_frames: 1600,
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.send_interval_frames = (self.sample_rate / 30.0) as u32;
        // Buffer holds the maximum delay plus interpolation slack.
        let len = (TIME_MAX as f32 * 0.001 * self.sample_rate).ceil() as usize + 4;
        self.buf_l = vec![0.0; len];
        self.buf_r = vec![0.0; len];
        self.write_idx = 0;
        self.tone_a = lp_coeff(self.tone_hz as f32, self.sample_rate);
        self.delay_tgt = self.delay_samples_for(self.time_ms);
        self.delay_cur = self.delay_tgt;
        self.lp_l = 0.0;
        self.lp_r = 0.0;
    }

    fn reset(&mut self) {
        for v in self.buf_l.iter_mut() { *v = 0.0; }
        for v in self.buf_r.iter_mut() { *v = 0.0; }
        self.lp_l = 0.0;
        self.lp_r = 0.0;
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_TIME => self.time_ms,
            PID_FEEDBACK => self.feedback,
            PID_TONE => self.tone_hz,
            PID_MIX => self.mix,
            PID_PINGPONG => if self.pingpong { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_TIME => {
                self.time_ms = value.clamp(TIME_MIN, TIME_MAX);
                self.delay_tgt = self.delay_samples_for(self.time_ms);
            }
            PID_FEEDBACK => self.feedback = value.clamp(FEEDBACK_MIN, FEEDBACK_MAX),
            PID_TONE => {
                self.tone_hz = value.clamp(TONE_MIN, TONE_MAX);
                self.tone_a = lp_coeff(self.tone_hz as f32, self.sample_rate);
            }
            PID_MIX => self.mix = value.clamp(MIX_MIN, MIX_MAX),
            PID_PINGPONG => self.pingpong = value >= 0.5,
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        if self.buf_l.is_empty() {
            wclap_plugin::silence(ctx);
            return ProcessStatus::Continue;
        }
        let len = self.buf_l.len();
        let fb = self.feedback as f32;
        let mix = self.mix as f32;
        let tone_a = self.tone_a;
        let pingpong = self.pingpong;
        // Glide the delay length toward target (~5 ms time constant).
        let glide = 1.0 - (-1.0_f32 / (0.005 * self.sample_rate)).exp();
        let mut delay = self.delay_cur;
        let delay_tgt = self.delay_tgt.min((len - 2) as f32);
        let mut w = self.write_idx;
        let mut lp_l = self.lp_l;
        let mut lp_r = self.lp_r;
        let mut peak = self.meter_peak;
        let mut n_processed: u32 = 0;

        if ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2 {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo { input_l, input_r, output_l, output_r } = io;
                let n = input_l.len();
                n_processed = n as u32;
                for f in 0..n {
                    delay += (delay_tgt - delay) * glide;
                    let rl = read_interp(&self.buf_l, w, delay);
                    let rr = read_interp(&self.buf_r, w, delay);
                    // Tone roll-off inside the feedback path.
                    lp_l += (rl - lp_l) * tone_a;
                    lp_r += (rr - lp_r) * tone_a;
                    let dry_l = input_l[f];
                    let dry_r = input_r[f];
                    let (in_l, in_r) = if pingpong {
                        (dry_l + lp_r * fb, dry_r + lp_l * fb)
                    } else {
                        (dry_l + lp_l * fb, dry_r + lp_r * fb)
                    };
                    self.buf_l[w] = in_l;
                    self.buf_r[w] = in_r;
                    let yl = dry_l * (1.0 - mix) + rl * mix;
                    let yr = dry_r * (1.0 - mix) + rr * mix;
                    output_l[f] = yl;
                    output_r[f] = yr;
                    let m = yl.abs().max(yr.abs());
                    if m > peak { peak = m; }
                    w = (w + 1) % len;
                }
            }
        }

        if n_processed == 0 {
            if let Some(io) = ctx.mono_io() {
                let wclap_plugin::MonoIo { input, output } = io;
                n_processed = input.len() as u32;
                for f in 0..input.len() {
                    delay += (delay_tgt - delay) * glide;
                    let r = read_interp(&self.buf_l, w, delay);
                    lp_l += (r - lp_l) * tone_a;
                    let dry = input[f];
                    self.buf_l[w] = dry + lp_l * fb;
                    let y = dry * (1.0 - mix) + r * mix;
                    output[f] = y;
                    if y.abs() > peak { peak = y.abs(); }
                    w = (w + 1) % len;
                }
            }
        }

        self.delay_cur = delay;
        self.write_idx = w;
        self.lp_l = lp_l;
        self.lp_r = lp_r;
        self.meter_peak = peak;
        self.frame_count += n_processed;
        if self.frame_count >= self.send_interval_frames {
            let mut buf = [0u8; 32];
            let pairs = [(PID_METER_PEAK, amp_to_db(self.meter_peak) as f64)];
            let l = encode_params(&mut buf, &pairs);
            if l > 0 {
                ctx.send_to_ui(&buf[..l]);
            }
            self.meter_peak *= 0.5;
            self.frame_count = 0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<Delay>(&PLUGIN_DEF);
}
