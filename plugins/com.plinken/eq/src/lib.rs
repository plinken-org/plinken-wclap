//! Parametric EQ — Plinken WCLAP audio-effect plugin.
//!
//! A 4-band stereo equaliser: a low shelf, two fully parametric peaking
//! bands (freq / gain / Q), and a high shelf, cascaded per channel.
//! Coefficients use the RBJ "Audio EQ Cookbook" formulas; each biquad runs
//! as a transposed direct-form II section. Coefficients are recomputed at
//! control rate (on `set_param`) — the audio loop just runs the filters.
//!
//! Params (10): LowShelf freq/gain, Peak1 freq/gain/Q, Peak2 freq/gain/Q,
//! HighShelf freq/gain.

extern crate alloc;

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, PARAM_IS_AUTOMATABLE,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.eq\0",
    name: b"Parametric EQ\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.0\0",
    description: b"4-band parametric EQ: low shelf, two peaks, high shelf (RBJ biquads).\0",
    features: &[b"audio-effect\0", b"equalizer\0", b"eq\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

// Band indices into the coefficient/param arrays.
const B_LS: usize = 0;
const B_P1: usize = 1;
const B_P2: usize = 2;
const B_HS: usize = 3;
const N_BANDS: usize = 4;

const PID_LS_FREQ: u32 = 0x0001;
const PID_LS_GAIN: u32 = 0x0002;
const PID_P1_FREQ: u32 = 0x0003;
const PID_P1_GAIN: u32 = 0x0004;
const PID_P1_Q: u32 = 0x0005;
const PID_P2_FREQ: u32 = 0x0006;
const PID_P2_GAIN: u32 = 0x0007;
const PID_P2_Q: u32 = 0x0008;
const PID_HS_FREQ: u32 = 0x0009;
const PID_HS_GAIN: u32 = 0x000A;

const PID_METER_PEAK: u32 = 0x1000;

const GAIN_MIN: f64 = -18.0;
const GAIN_MAX: f64 = 18.0;
const Q_MIN: f64 = 0.3;
const Q_MAX: f64 = 6.0;

static PARAMS: &[ParamDef] = &[
    ParamDef { id: PID_LS_FREQ, flags: PARAM_IS_AUTOMATABLE, name: b"Low Freq\0", module: b"Low Shelf\0", min: 20.0, max: 400.0, default: 100.0 },
    ParamDef { id: PID_LS_GAIN, flags: PARAM_IS_AUTOMATABLE, name: b"Low Gain\0", module: b"Low Shelf\0", min: GAIN_MIN, max: GAIN_MAX, default: 0.0 },
    ParamDef { id: PID_P1_FREQ, flags: PARAM_IS_AUTOMATABLE, name: b"Lo-Mid Freq\0", module: b"Peak 1\0", min: 80.0, max: 2000.0, default: 400.0 },
    ParamDef { id: PID_P1_GAIN, flags: PARAM_IS_AUTOMATABLE, name: b"Lo-Mid Gain\0", module: b"Peak 1\0", min: GAIN_MIN, max: GAIN_MAX, default: 0.0 },
    ParamDef { id: PID_P1_Q, flags: PARAM_IS_AUTOMATABLE, name: b"Lo-Mid Q\0", module: b"Peak 1\0", min: Q_MIN, max: Q_MAX, default: 1.0 },
    ParamDef { id: PID_P2_FREQ, flags: PARAM_IS_AUTOMATABLE, name: b"Hi-Mid Freq\0", module: b"Peak 2\0", min: 500.0, max: 12000.0, default: 3000.0 },
    ParamDef { id: PID_P2_GAIN, flags: PARAM_IS_AUTOMATABLE, name: b"Hi-Mid Gain\0", module: b"Peak 2\0", min: GAIN_MIN, max: GAIN_MAX, default: 0.0 },
    ParamDef { id: PID_P2_Q, flags: PARAM_IS_AUTOMATABLE, name: b"Hi-Mid Q\0", module: b"Peak 2\0", min: Q_MIN, max: Q_MAX, default: 1.0 },
    ParamDef { id: PID_HS_FREQ, flags: PARAM_IS_AUTOMATABLE, name: b"High Freq\0", module: b"High Shelf\0", min: 2000.0, max: 18000.0, default: 8000.0 },
    ParamDef { id: PID_HS_GAIN, flags: PARAM_IS_AUTOMATABLE, name: b"High Gain\0", module: b"High Shelf\0", min: GAIN_MIN, max: GAIN_MAX, default: 0.0 },
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

/// Normalised biquad coefficients (a0 divided out).
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Identity (pass-through) filter.
    const fn flat() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }

    fn peaking(freq: f32, gain_db: f32, q: f32, sr: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * core::f32::consts::PI * (freq / sr).clamp(1.0e-5, 0.49);
        let (sin, cos) = (w0.sin(), w0.cos());
        let alpha = sin / (2.0 * q.max(1.0e-3));
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    fn low_shelf(freq: f32, gain_db: f32, sr: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * core::f32::consts::PI * (freq / sr).clamp(1.0e-5, 0.49);
        let (sin, cos) = (w0.sin(), w0.cos());
        // S = 1 (standard shelf slope) → alpha derived below.
        let alpha = sin / 2.0 * (((a + 1.0 / a) * (1.0 / 1.0 - 1.0)) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha;
        Self {
            b0: a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha) / a0,
            b1: 2.0 * a * ((a - 1.0) - (a + 1.0) * cos) / a0,
            b2: a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha) / a0,
            a1: -2.0 * ((a - 1.0) + (a + 1.0) * cos) / a0,
            a2: ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha) / a0,
        }
    }

    fn high_shelf(freq: f32, gain_db: f32, sr: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * core::f32::consts::PI * (freq / sr).clamp(1.0e-5, 0.49);
        let (sin, cos) = (w0.sin(), w0.cos());
        let alpha = sin / 2.0 * (((a + 1.0 / a) * (1.0 / 1.0 - 1.0)) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha;
        Self {
            b0: a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha) / a0,
            b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * cos) / a0,
            b2: a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha) / a0,
            a1: 2.0 * ((a - 1.0) - (a + 1.0) * cos) / a0,
            a2: ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha) / a0,
        }
    }
}

/// Transposed direct-form II state for one biquad on one channel.
#[derive(Clone, Copy, Default)]
struct BiquadState {
    s1: f32,
    s2: f32,
}

impl BiquadState {
    #[inline]
    fn tick(&mut self, c: &Biquad, x: f32) -> f32 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }
}

struct Eq {
    // Param values, indexed by band where it makes sense.
    freq: [f64; N_BANDS],
    gain: [f64; N_BANDS],
    q: [f64; 2], // Peak1, Peak2 only.

    coeffs: [Biquad; N_BANDS],
    state_l: [BiquadState; N_BANDS],
    state_r: [BiquadState; N_BANDS],
    sample_rate: f32,

    meter_peak: f32,
    frame_count: u32,
    send_interval_frames: u32,
}

impl Eq {
    fn recalc_band(&mut self, band: usize) {
        let sr = self.sample_rate;
        self.coeffs[band] = match band {
            B_LS => Biquad::low_shelf(self.freq[B_LS] as f32, self.gain[B_LS] as f32, sr),
            B_P1 => Biquad::peaking(self.freq[B_P1] as f32, self.gain[B_P1] as f32, self.q[0] as f32, sr),
            B_P2 => Biquad::peaking(self.freq[B_P2] as f32, self.gain[B_P2] as f32, self.q[1] as f32, sr),
            _ => Biquad::high_shelf(self.freq[B_HS] as f32, self.gain[B_HS] as f32, sr),
        };
    }

    fn recalc_all(&mut self) {
        for b in 0..N_BANDS {
            self.recalc_band(b);
        }
    }
}

impl Plugin for Eq {
    fn new() -> Self {
        let mut p = Self {
            freq: [100.0, 400.0, 3000.0, 8000.0],
            gain: [0.0, 0.0, 0.0, 0.0],
            q: [1.0, 1.0],
            coeffs: [Biquad::flat(); N_BANDS],
            state_l: [BiquadState::default(); N_BANDS],
            state_r: [BiquadState::default(); N_BANDS],
            sample_rate: 48000.0,
            meter_peak: 0.0,
            frame_count: 0,
            send_interval_frames: 1600,
        };
        p.recalc_all();
        p
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.send_interval_frames = (self.sample_rate / 30.0) as u32;
        self.recalc_all();
    }

    fn reset(&mut self) {
        self.state_l = [BiquadState::default(); N_BANDS];
        self.state_r = [BiquadState::default(); N_BANDS];
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_LS_FREQ => self.freq[B_LS],
            PID_LS_GAIN => self.gain[B_LS],
            PID_P1_FREQ => self.freq[B_P1],
            PID_P1_GAIN => self.gain[B_P1],
            PID_P1_Q => self.q[0],
            PID_P2_FREQ => self.freq[B_P2],
            PID_P2_GAIN => self.gain[B_P2],
            PID_P2_Q => self.q[1],
            PID_HS_FREQ => self.freq[B_HS],
            PID_HS_GAIN => self.gain[B_HS],
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_LS_FREQ => { self.freq[B_LS] = value.clamp(20.0, 400.0); self.recalc_band(B_LS); }
            PID_LS_GAIN => { self.gain[B_LS] = value.clamp(GAIN_MIN, GAIN_MAX); self.recalc_band(B_LS); }
            PID_P1_FREQ => { self.freq[B_P1] = value.clamp(80.0, 2000.0); self.recalc_band(B_P1); }
            PID_P1_GAIN => { self.gain[B_P1] = value.clamp(GAIN_MIN, GAIN_MAX); self.recalc_band(B_P1); }
            PID_P1_Q => { self.q[0] = value.clamp(Q_MIN, Q_MAX); self.recalc_band(B_P1); }
            PID_P2_FREQ => { self.freq[B_P2] = value.clamp(500.0, 12000.0); self.recalc_band(B_P2); }
            PID_P2_GAIN => { self.gain[B_P2] = value.clamp(GAIN_MIN, GAIN_MAX); self.recalc_band(B_P2); }
            PID_P2_Q => { self.q[1] = value.clamp(Q_MIN, Q_MAX); self.recalc_band(B_P2); }
            PID_HS_FREQ => { self.freq[B_HS] = value.clamp(2000.0, 18000.0); self.recalc_band(B_HS); }
            PID_HS_GAIN => { self.gain[B_HS] = value.clamp(GAIN_MIN, GAIN_MAX); self.recalc_band(B_HS); }
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let coeffs = self.coeffs;
        let mut peak = self.meter_peak;
        let mut n_processed: u32 = 0;

        if ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2 {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo { input_l, input_r, output_l, output_r } = io;
                let n = input_l.len();
                n_processed = n as u32;
                for f in 0..n {
                    let mut xl = input_l[f];
                    let mut xr = input_r[f];
                    for b in 0..N_BANDS {
                        xl = self.state_l[b].tick(&coeffs[b], xl);
                        xr = self.state_r[b].tick(&coeffs[b], xr);
                    }
                    output_l[f] = xl;
                    output_r[f] = xr;
                    let m = xl.abs().max(xr.abs());
                    if m > peak { peak = m; }
                }
            }
        }

        if n_processed == 0 {
            if let Some(io) = ctx.mono_io() {
                let wclap_plugin::MonoIo { input, output } = io;
                n_processed = input.len() as u32;
                for f in 0..input.len() {
                    let mut x = input[f];
                    for b in 0..N_BANDS {
                        x = self.state_l[b].tick(&coeffs[b], x);
                    }
                    output[f] = x;
                    if x.abs() > peak { peak = x.abs(); }
                }
            }
        }

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
    init_plugin::<Eq>(&PLUGIN_DEF);
}
