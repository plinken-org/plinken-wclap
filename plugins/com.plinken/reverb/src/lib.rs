//! Reverb — Plinken WCLAP audio-effect plugin.
//!
//! A Freeverb-style algorithmic stereo reverb: eight damped comb filters in
//! parallel feeding four allpass filters in series, per channel, with the
//! classic stereo-spread offset between the L and R tunings. A pre-delay
//! line sits in front of the wet send; the dry signal stays dry.
//!
//! Params: Size (decay), Damp, Pre-delay, Mix, Width.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, PARAM_IS_AUTOMATABLE,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.reverb\0",
    name: b"Reverb\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.0\0",
    description: b"Freeverb-style algorithmic stereo reverb with pre-delay and width.\0",
    features: &[b"audio-effect\0", b"reverb\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

const PID_SIZE: u32 = 0x0001;
const PID_DAMP: u32 = 0x0002;
const PID_PREDELAY: u32 = 0x0003;
const PID_MIX: u32 = 0x0004;
const PID_WIDTH: u32 = 0x0005;

const PID_METER_PEAK: u32 = 0x1000;

const SIZE_DEFAULT: f64 = 0.5;
const DAMP_DEFAULT: f64 = 0.5;
const PREDELAY_MAX: f64 = 200.0;
const PREDELAY_DEFAULT: f64 = 0.0;
const MIX_DEFAULT: f64 = 0.3;
const WIDTH_DEFAULT: f64 = 1.0;

// Freeverb constants (tunings in samples at 44.1 kHz).
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const FIXED_GAIN: f32 = 0.015;
const SCALE_ROOM: f32 = 0.28;
const OFFSET_ROOM: f32 = 0.7;
const SCALE_DAMP: f32 = 0.4;

static PARAMS: &[ParamDef] = &[
    ParamDef { id: PID_SIZE, flags: PARAM_IS_AUTOMATABLE, name: b"Size\0", module: b"\0", min: 0.0, max: 1.0, default: SIZE_DEFAULT },
    ParamDef { id: PID_DAMP, flags: PARAM_IS_AUTOMATABLE, name: b"Damp\0", module: b"\0", min: 0.0, max: 1.0, default: DAMP_DEFAULT },
    ParamDef { id: PID_PREDELAY, flags: PARAM_IS_AUTOMATABLE, name: b"Pre-Delay\0", module: b"\0", min: 0.0, max: PREDELAY_MAX, default: PREDELAY_DEFAULT },
    ParamDef { id: PID_MIX, flags: PARAM_IS_AUTOMATABLE, name: b"Mix\0", module: b"\0", min: 0.0, max: 1.0, default: MIX_DEFAULT },
    ParamDef { id: PID_WIDTH, flags: PARAM_IS_AUTOMATABLE, name: b"Width\0", module: b"\0", min: 0.0, max: 1.0, default: WIDTH_DEFAULT },
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

/// Damped feedback comb filter.
struct Comb {
    buffer: Vec<f32>,
    idx: usize,
    filterstore: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buffer: vec![0.0; len.max(1)], idx: 0, filterstore: 0.0, feedback: 0.5, damp1: 0.0, damp2: 1.0 }
    }
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.idx];
        self.filterstore = output * self.damp2 + self.filterstore * self.damp1;
        self.buffer[self.idx] = input + self.filterstore * self.feedback;
        self.idx += 1;
        if self.idx >= self.buffer.len() {
            self.idx = 0;
        }
        output
    }
    fn clear(&mut self) {
        for v in self.buffer.iter_mut() { *v = 0.0; }
        self.filterstore = 0.0;
    }
}

/// Allpass filter (fixed feedback 0.5).
struct Allpass {
    buffer: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buffer: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let bufout = self.buffer[self.idx];
        let output = -input + bufout;
        self.buffer[self.idx] = input + bufout * 0.5;
        self.idx += 1;
        if self.idx >= self.buffer.len() {
            self.idx = 0;
        }
        output
    }
    fn clear(&mut self) {
        for v in self.buffer.iter_mut() { *v = 0.0; }
    }
}

struct Reverb {
    size: f64,
    damp: f64,
    predelay_ms: f64,
    mix: f64,
    width: f64,

    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allpass_l: Vec<Allpass>,
    allpass_r: Vec<Allpass>,

    // Pre-delay lines.
    pre_l: Vec<f32>,
    pre_r: Vec<f32>,
    pre_idx: usize,

    sample_rate: f32,
    meter_peak: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl Reverb {
    fn apply_size_damp(&mut self) {
        let feedback = self.size as f32 * SCALE_ROOM + OFFSET_ROOM;
        let damp1 = self.damp as f32 * SCALE_DAMP;
        let damp2 = 1.0 - damp1;
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.feedback = feedback;
            c.damp1 = damp1;
            c.damp2 = damp2;
        }
    }
}

impl Plugin for Reverb {
    fn new() -> Self {
        Self {
            size: SIZE_DEFAULT,
            damp: DAMP_DEFAULT,
            predelay_ms: PREDELAY_DEFAULT,
            mix: MIX_DEFAULT,
            width: WIDTH_DEFAULT,
            combs_l: Vec::new(),
            combs_r: Vec::new(),
            allpass_l: Vec::new(),
            allpass_r: Vec::new(),
            pre_l: Vec::new(),
            pre_r: Vec::new(),
            pre_idx: 0,
            sample_rate: 48000.0,
            meter_peak: 0.0,
            frame_count: 0,
            send_interval_frames: 1600,
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.send_interval_frames = (self.sample_rate / 30.0) as u32;
        // Scale the 44.1 kHz tunings to the running sample rate.
        let scale = self.sample_rate / 44100.0;
        let sz = |t: usize| ((t as f32 * scale) as usize).max(1);
        self.combs_l = COMB_TUNING.iter().map(|&t| Comb::new(sz(t))).collect();
        self.combs_r = COMB_TUNING.iter().map(|&t| Comb::new(sz(t + STEREO_SPREAD))).collect();
        self.allpass_l = ALLPASS_TUNING.iter().map(|&t| Allpass::new(sz(t))).collect();
        self.allpass_r = ALLPASS_TUNING.iter().map(|&t| Allpass::new(sz(t + STEREO_SPREAD))).collect();
        let pre_len = (PREDELAY_MAX as f32 * 0.001 * self.sample_rate).ceil() as usize + 1;
        self.pre_l = vec![0.0; pre_len];
        self.pre_r = vec![0.0; pre_len];
        self.pre_idx = 0;
        self.apply_size_damp();
    }

    fn reset(&mut self) {
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) { c.clear(); }
        for a in self.allpass_l.iter_mut().chain(self.allpass_r.iter_mut()) { a.clear(); }
        for v in self.pre_l.iter_mut() { *v = 0.0; }
        for v in self.pre_r.iter_mut() { *v = 0.0; }
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_SIZE => self.size,
            PID_DAMP => self.damp,
            PID_PREDELAY => self.predelay_ms,
            PID_MIX => self.mix,
            PID_WIDTH => self.width,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_SIZE => { self.size = value.clamp(0.0, 1.0); self.apply_size_damp(); }
            PID_DAMP => { self.damp = value.clamp(0.0, 1.0); self.apply_size_damp(); }
            PID_PREDELAY => self.predelay_ms = value.clamp(0.0, PREDELAY_MAX),
            PID_MIX => self.mix = value.clamp(0.0, 1.0),
            PID_WIDTH => self.width = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        if self.combs_l.is_empty() {
            wclap_plugin::silence(ctx);
            return ProcessStatus::Continue;
        }
        let mix = self.mix as f32;
        let width = self.width as f32;
        let wet1 = width * 0.5 + 0.5;
        let wet2 = (1.0 - width) * 0.5;
        let pre_len = self.pre_l.len();
        let pre_samples =
            ((self.predelay_ms as f32 * 0.001 * self.sample_rate) as usize).min(pre_len.saturating_sub(1));
        let mut pre_idx = self.pre_idx;
        let mut peak = self.meter_peak;
        let mut n_processed: u32 = 0;

        let stereo = ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2;
        if stereo {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo { input_l, input_r, output_l, output_r } = io;
                let n = input_l.len();
                n_processed = n as u32;
                for f in 0..n {
                    let dry_l = input_l[f];
                    let dry_r = input_r[f];
                    // Pre-delay the wet send only.
                    let read = (pre_idx + pre_len - pre_samples) % pre_len;
                    let send_l = self.pre_l[read];
                    let send_r = self.pre_r[read];
                    self.pre_l[pre_idx] = dry_l;
                    self.pre_r[pre_idx] = dry_r;
                    pre_idx = (pre_idx + 1) % pre_len;

                    let input = (send_l + send_r) * FIXED_GAIN;
                    let mut acc_l = 0.0;
                    let mut acc_r = 0.0;
                    for c in self.combs_l.iter_mut() { acc_l += c.process(input); }
                    for c in self.combs_r.iter_mut() { acc_r += c.process(input); }
                    for a in self.allpass_l.iter_mut() { acc_l = a.process(acc_l); }
                    for a in self.allpass_r.iter_mut() { acc_r = a.process(acc_r); }
                    let rl = acc_l * wet1 + acc_r * wet2;
                    let rr = acc_r * wet1 + acc_l * wet2;
                    let yl = dry_l * (1.0 - mix) + rl * mix;
                    let yr = dry_r * (1.0 - mix) + rr * mix;
                    output_l[f] = yl;
                    output_r[f] = yr;
                    let m = yl.abs().max(yr.abs());
                    if m > peak { peak = m; }
                }
            }
        } else if let Some(io) = ctx.mono_io() {
            let wclap_plugin::MonoIo { input, output } = io;
            n_processed = input.len() as u32;
            for f in 0..input.len() {
                let dry = input[f];
                let read = (pre_idx + pre_len - pre_samples) % pre_len;
                let send = self.pre_l[read];
                self.pre_l[pre_idx] = dry;
                pre_idx = (pre_idx + 1) % pre_len;
                let inp = send * 2.0 * FIXED_GAIN;
                let mut acc = 0.0;
                for c in self.combs_l.iter_mut() { acc += c.process(inp); }
                for a in self.allpass_l.iter_mut() { acc = a.process(acc); }
                let y = dry * (1.0 - mix) + acc * mix;
                output[f] = y;
                if y.abs() > peak { peak = y.abs(); }
            }
        }

        self.pre_idx = pre_idx;
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
    init_plugin::<Reverb>(&PLUGIN_DEF);
}
