//! Chorus — Plinken WCLAP audio-effect plugin.
//!
//! A classic LFO-modulated delay chorus: up to three voices, each tapping
//! the delay line at a sinusoidally-modulated position with a different LFO
//! phase, summed and blended with the dry signal. The right channel reads a
//! phase-shifted LFO (the Spread control) for a wide stereo image. No
//! feedback — the delay line carries only the dry input.
//!
//! Params: Rate, Depth, Voices, Mix, Spread.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, PARAM_IS_AUTOMATABLE,
    PARAM_IS_STEPPED,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.chorus\0",
    name: b"Chorus\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.0\0",
    description: b"LFO-modulated stereo chorus, up to three voices, with stereo spread.\0",
    features: &[b"audio-effect\0", b"chorus\0", b"modulation\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

const PID_RATE: u32 = 0x0001;
const PID_DEPTH: u32 = 0x0002;
const PID_VOICES: u32 = 0x0003;
const PID_MIX: u32 = 0x0004;
const PID_SPREAD: u32 = 0x0005;

const PID_METER_PEAK: u32 = 0x1000;

const RATE_MIN: f64 = 0.05;
const RATE_MAX: f64 = 8.0;
const RATE_DEFAULT: f64 = 0.8;

const DEPTH_MIN: f64 = 0.0;
const DEPTH_MAX: f64 = 1.0;
const DEPTH_DEFAULT: f64 = 0.4;

const VOICES_MIN: f64 = 1.0;
const VOICES_MAX: f64 = 3.0;
const VOICES_DEFAULT: f64 = 2.0;

const MIX_MIN: f64 = 0.0;
const MIX_MAX: f64 = 1.0;
const MIX_DEFAULT: f64 = 0.5;

const SPREAD_MIN: f64 = 0.0;
const SPREAD_MAX: f64 = 1.0;
const SPREAD_DEFAULT: f64 = 0.7;

/// Centre delay and modulation depth (ms). Centre > max depth so the read
/// position never crosses the write head.
const BASE_MS: f32 = 12.0;
const DEPTH_MS: f32 = 8.0;

static PARAMS: &[ParamDef] = &[
    ParamDef { id: PID_RATE, flags: PARAM_IS_AUTOMATABLE, name: b"Rate\0", module: b"\0", min: RATE_MIN, max: RATE_MAX, default: RATE_DEFAULT },
    ParamDef { id: PID_DEPTH, flags: PARAM_IS_AUTOMATABLE, name: b"Depth\0", module: b"\0", min: DEPTH_MIN, max: DEPTH_MAX, default: DEPTH_DEFAULT },
    ParamDef { id: PID_VOICES, flags: PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED, name: b"Voices\0", module: b"\0", min: VOICES_MIN, max: VOICES_MAX, default: VOICES_DEFAULT },
    ParamDef { id: PID_MIX, flags: PARAM_IS_AUTOMATABLE, name: b"Mix\0", module: b"\0", min: MIX_MIN, max: MIX_MAX, default: MIX_DEFAULT },
    ParamDef { id: PID_SPREAD, flags: PARAM_IS_AUTOMATABLE, name: b"Spread\0", module: b"\0", min: SPREAD_MIN, max: SPREAD_MAX, default: SPREAD_DEFAULT },
];

fn amp_to_db(amp: f32) -> f32 {
    if amp <= 1.0e-9 { -120.0 } else { 20.0 * amp.log10() }
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

#[inline]
fn read_interp(buf: &[f32], write_idx: usize, delay: f32) -> f32 {
    let len = buf.len();
    if len == 0 {
        return 0.0;
    }
    let mut rp = (write_idx as f32 - delay) % len as f32;
    if rp < 0.0 {
        rp += len as f32;
    }
    let i0 = rp.floor() as usize % len;
    let i1 = (i0 + 1) % len;
    let frac = rp - rp.floor();
    buf[i0] * (1.0 - frac) + buf[i1] * frac
}

struct Chorus {
    rate_hz: f64,
    depth: f64,
    voices: u8,
    mix: f64,
    spread: f64,

    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write_idx: usize,
    lfo_phase: f32,
    sample_rate: f32,

    meter_peak: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl Plugin for Chorus {
    fn new() -> Self {
        Self {
            rate_hz: RATE_DEFAULT,
            depth: DEPTH_DEFAULT,
            voices: 2,
            mix: MIX_DEFAULT,
            spread: SPREAD_DEFAULT,
            buf_l: Vec::new(),
            buf_r: Vec::new(),
            write_idx: 0,
            lfo_phase: 0.0,
            sample_rate: 48000.0,
            meter_peak: 0.0,
            frame_count: 0,
            send_interval_frames: 1600,
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.send_interval_frames = (self.sample_rate / 30.0) as u32;
        // Holds centre + full modulation depth plus interpolation slack.
        let max_ms = BASE_MS + DEPTH_MS + 4.0;
        let len = (max_ms * 0.001 * self.sample_rate).ceil() as usize + 4;
        self.buf_l = vec![0.0; len];
        self.buf_r = vec![0.0; len];
        self.write_idx = 0;
        self.lfo_phase = 0.0;
    }

    fn reset(&mut self) {
        for v in self.buf_l.iter_mut() { *v = 0.0; }
        for v in self.buf_r.iter_mut() { *v = 0.0; }
        self.lfo_phase = 0.0;
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_RATE => self.rate_hz,
            PID_DEPTH => self.depth,
            PID_VOICES => self.voices as f64,
            PID_MIX => self.mix,
            PID_SPREAD => self.spread,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_RATE => self.rate_hz = value.clamp(RATE_MIN, RATE_MAX),
            PID_DEPTH => self.depth = value.clamp(DEPTH_MIN, DEPTH_MAX),
            PID_VOICES => self.voices = value.clamp(VOICES_MIN, VOICES_MAX).round() as u8,
            PID_MIX => self.mix = value.clamp(MIX_MIN, MIX_MAX),
            PID_SPREAD => self.spread = value.clamp(SPREAD_MIN, SPREAD_MAX),
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        if self.buf_l.is_empty() {
            wclap_plugin::silence(ctx);
            return ProcessStatus::Continue;
        }
        let len = self.buf_l.len();
        let sr = self.sample_rate;
        let voices = self.voices.clamp(1, 3);
        let inv_voices = 1.0 / voices as f32;
        let depth_ms = DEPTH_MS * self.depth as f32;
        let mix = self.mix as f32;
        let spread = self.spread as f32;
        let phase_inc = self.rate_hz as f32 / sr;
        let two_pi = core::f32::consts::TAU;
        let mut phase = self.lfo_phase;
        let mut w = self.write_idx;
        let mut peak = self.meter_peak;
        let mut n_processed: u32 = 0;

        // Process either stereo or, for mono, duplicate L into R math.
        let stereo = ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2;
        if stereo {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo { input_l, input_r, output_l, output_r } = io;
                let n = input_l.len();
                n_processed = n as u32;
                for f in 0..n {
                    let dry_l = input_l[f];
                    let dry_r = input_r[f];
                    self.buf_l[w] = dry_l;
                    self.buf_r[w] = dry_r;
                    let mut wet_l = 0.0;
                    let mut wet_r = 0.0;
                    for v in 0..voices {
                        let voff = v as f32 * inv_voices;
                        let ph = phase + voff;
                        let lfo_l = (two_pi * ph).sin();
                        let lfo_r = (two_pi * (ph + spread * 0.25)).sin();
                        let d_l = (BASE_MS + depth_ms * lfo_l) * 0.001 * sr;
                        let d_r = (BASE_MS + depth_ms * lfo_r) * 0.001 * sr;
                        wet_l += read_interp(&self.buf_l, w, d_l);
                        wet_r += read_interp(&self.buf_r, w, d_r);
                    }
                    wet_l *= inv_voices;
                    wet_r *= inv_voices;
                    let yl = dry_l * (1.0 - mix) + wet_l * mix;
                    let yr = dry_r * (1.0 - mix) + wet_r * mix;
                    output_l[f] = yl;
                    output_r[f] = yr;
                    let m = yl.abs().max(yr.abs());
                    if m > peak { peak = m; }
                    phase += phase_inc;
                    if phase >= 1.0 { phase -= 1.0; }
                    w = (w + 1) % len;
                }
            }
        } else if let Some(io) = ctx.mono_io() {
            let wclap_plugin::MonoIo { input, output } = io;
            n_processed = input.len() as u32;
            for f in 0..input.len() {
                let dry = input[f];
                self.buf_l[w] = dry;
                let mut wet = 0.0;
                for v in 0..voices {
                    let ph = phase + v as f32 * inv_voices;
                    let lfo = (two_pi * ph).sin();
                    let d = (BASE_MS + depth_ms * lfo) * 0.001 * sr;
                    wet += read_interp(&self.buf_l, w, d);
                }
                wet *= inv_voices;
                let y = dry * (1.0 - mix) + wet * mix;
                output[f] = y;
                if y.abs() > peak { peak = y.abs(); }
                phase += phase_inc;
                if phase >= 1.0 { phase -= 1.0; }
                w = (w + 1) % len;
            }
        }

        self.lfo_phase = phase;
        self.write_idx = w;
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
    init_plugin::<Chorus>(&PLUGIN_DEF);
}
