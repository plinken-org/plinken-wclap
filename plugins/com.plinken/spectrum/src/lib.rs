//! Spectrum — Plinken WCLAP audio-effect plugin.
//!
//! The audio path is a literal stereo passthrough built with fundsp
//! (`multipass::<U2>()`). In parallel, a side-tap accumulates the mono sum
//! (L+R) * 0.5 into a 1024-sample ring buffer, Hann-windows it, runs
//! `fundsp::fft::real_fft`, bins the |X(k)| magnitudes into 64 log-spaced
//! bands between 20 Hz and Nyquist, and pushes the band dB values to the
//! UI iframe via `clap_host_webview.send` at ~30 Hz.
//!
//! Two automatable params:
//!   * Floor   — meter floor in dB (controls the bottom of the UI's range)
//!   * Smooth  — UI-side smoothing factor 0..1 (forwarded for the canvas)

use fundsp::audionode::MultiPass;
use fundsp::audiounit::AudioUnit;
use fundsp::combinator::An;
use fundsp::prelude32::{multipass, U2};

use wclap_plugin::{
    init_plugin, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.spectrum\0",
    name: b"Spectrum\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.0.1\0",
    description: b"Real-time spectrum analyzer (audio passthrough + FFT side-tap).\0",
    features: &[b"audio-effect\0", b"analyzer\0", b"utility\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

const PID_FLOOR: u32 = 0x0001;
const PID_SMOOTH: u32 = 0x0002;

// Readonly metadata id — pushed once on first process tick so the UI can
// compute log-frequency label positions against the actual Nyquist.
const PID_SAMPLE_RATE: u32 = 0x1000;

const FLOOR_MIN: f64 = -120.0;
const FLOOR_MAX: f64 = -20.0;
const FLOOR_DEFAULT: f64 = -80.0;

const SMOOTH_MIN: f64 = 0.0;
const SMOOTH_MAX: f64 = 0.95;
const SMOOTH_DEFAULT: f64 = 0.55;

static PARAMS: &[ParamDef] = &[
    ParamDef {
        id: PID_FLOOR,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Floor\0",
        module: b"\0",
        min: FLOOR_MIN,
        max: FLOOR_MAX,
        default: FLOOR_DEFAULT,
    },
    ParamDef {
        id: PID_SMOOTH,
        flags: PARAM_IS_AUTOMATABLE,
        name: b"Smooth\0",
        module: b"\0",
        min: SMOOTH_MIN,
        max: SMOOTH_MAX,
        default: SMOOTH_DEFAULT,
    },
];

const FFT_SIZE: usize = 1024;
const N_BINS: usize = 64;
const F_MIN: f32 = 20.0;

/// Encode `{ "spec": <byte string of N_BINS f32 big-endian dB values> }`.
/// Layout fixed at 1 (map) + 1 + 4 ("spec") + 3 (bstr 0x59 + u16 len) +
/// N_BINS*4 = 265 bytes for N_BINS = 64.
fn encode_spectrum(buf: &mut [u8], bands_db: &[f32; N_BINS]) -> usize {
    const HEADER_LEN: usize = 1 + 1 + 4 + 3;
    let payload_len = N_BINS * 4;
    if buf.len() < HEADER_LEN + payload_len {
        return 0;
    }
    let mut i = 0;
    buf[i] = 0xa1; i += 1;                            // map(1)
    buf[i] = 0x64; i += 1;                            // text(4)
    buf[i..i + 4].copy_from_slice(b"spec"); i += 4;
    buf[i] = 0x59; i += 1;                            // byte string, u16 length
    buf[i..i + 2].copy_from_slice(&(payload_len as u16).to_be_bytes()); i += 2;
    for v in bands_db.iter() {
        buf[i..i + 4].copy_from_slice(&v.to_be_bytes()); i += 4;
    }
    i
}

/// Returns the FFT bin index range `[low, high)` for band `b`, log-spaced
/// from `F_MIN` to Nyquist.
fn band_range(b: usize, sample_rate: f32) -> (usize, usize) {
    let nyquist = sample_rate * 0.5;
    let ratio = (nyquist / F_MIN).max(1.0);
    let lo_freq = F_MIN * ratio.powf(b as f32 / N_BINS as f32);
    let hi_freq = F_MIN * ratio.powf((b + 1) as f32 / N_BINS as f32);
    let bin_step = sample_rate / FFT_SIZE as f32;
    let mut lo = (lo_freq / bin_step) as usize;
    let mut hi = (hi_freq / bin_step) as usize;
    if lo < 1 { lo = 1; }
    if hi <= lo { hi = lo + 1; }
    let max_bin = FFT_SIZE / 2;
    if hi > max_bin { hi = max_bin; }
    if lo >= hi { lo = hi - 1; }
    (lo, hi)
}

struct Spectrum {
    chain: An<MultiPass<U2>>,

    sample_rate: f32,

    // Floor + smoothing are mirrored to the UI; the DSP side just stores
    // them and forwards via the readonly param channel so the canvas can
    // react without a separate message shape.
    floor_db: f64,
    smooth: f64,

    // Mono ring buffer of FFT_SIZE samples; we run an FFT every `hop`
    // samples (FFT_SIZE/2) for ~93 Hz at 48 kHz.
    ring: [f32; FFT_SIZE],
    ring_pos: usize,
    samples_since_fft: usize,
    hop: usize,

    // Pre-computed Hann window; only depends on FFT_SIZE.
    window: [f32; FFT_SIZE],

    // Pre-computed bin-range per band (recomputed on activate).
    bands: [(u16, u16); N_BINS],

    // FFT scratch — fundsp's real_fft does in-place transform on this.
    fft_buf: [f32; FFT_SIZE],

    // Last raw band dB values (peak in band, dBFS, clamped to floor).
    band_db: [f32; N_BINS],

    // UI update throttle: send a spectrum frame every ~33 ms.
    frames_since_send: u32,
    send_interval_frames: u32,

    // Set after the sample rate has been pushed to the UI; reset on activate
    // so the UI sees the updated rate if the host re-activates with a
    // different sample rate.
    sent_sample_rate: bool,
}

fn hann(n: usize, total: usize) -> f32 {
    let phase = 2.0 * core::f32::consts::PI * n as f32 / (total - 1) as f32;
    0.5 * (1.0 - phase.cos())
}

fn amp_to_db(amp: f32, floor: f32) -> f32 {
    if amp <= 1.0e-9 {
        floor
    } else {
        (20.0 * amp.log10()).max(floor)
    }
}

impl Spectrum {
    fn recompute_bands(&mut self) {
        for b in 0..N_BINS {
            let (lo, hi) = band_range(b, self.sample_rate);
            self.bands[b] = (lo as u16, hi as u16);
        }
    }

    /// Emit a one-shot `{ params: { 0x1000: <sample_rate> } }` snapshot so
    /// the UI can compute log-frequency label positions accurately. Shape
    /// matches widgets/cbor.mjs `decodeParamsSnapshot`.
    fn send_sample_rate(&self, ctx: &mut ProcessCtx) {
        let mut buf = [0u8; 1 + 1 + 6 + 1 + 1 + 4 + 1 + 8];
        let mut i = 0;
        buf[i] = 0xa1; i += 1;                                     // map(1)
        buf[i] = 0x66; i += 1;                                     // text(6)
        buf[i..i + 6].copy_from_slice(b"params"); i += 6;
        buf[i] = 0xa1; i += 1;                                     // map(1)
        buf[i] = 0x1a; i += 1;                                     // u32
        buf[i..i + 4].copy_from_slice(&PID_SAMPLE_RATE.to_be_bytes()); i += 4;
        buf[i] = 0xfb; i += 1;                                     // f64
        buf[i..i + 8].copy_from_slice(&(self.sample_rate as f64).to_be_bytes()); i += 8;
        ctx.send_to_ui(&buf[..i]);
    }

    fn run_fft_and_send(&mut self, ctx: &mut ProcessCtx) {
        // Window + copy ring (oldest-first) into fft_buf.
        for i in 0..FFT_SIZE {
            let idx = (self.ring_pos + i) % FFT_SIZE;
            self.fft_buf[i] = self.ring[idx] * self.window[i];
        }

        // In-place real FFT — returns &mut [Complex32] of length FFT_SIZE/2.
        // DC is at index 0; Nyquist is packed into DC's imaginary part (we
        // don't use it for visualization).
        let spectrum = fundsp::fft::real_fft(&mut self.fft_buf);

        let floor = self.floor_db as f32;
        let norm = 2.0 / FFT_SIZE as f32; // 1-sided amplitude normalization

        // Peak magnitude inside each band → dB.
        for (b, range) in self.bands.iter().enumerate() {
            let (lo, hi) = (range.0 as usize, range.1 as usize);
            let mut peak = 0.0_f32;
            for k in lo..hi {
                let c = spectrum[k];
                let mag = (c.re * c.re + c.im * c.im).sqrt();
                if mag > peak {
                    peak = mag;
                }
            }
            self.band_db[b] = amp_to_db(peak * norm, floor);
        }

        let mut buf = [0u8; 1 + 1 + 4 + 3 + N_BINS * 4];
        let len = encode_spectrum(&mut buf, &self.band_db);
        if len > 0 {
            ctx.send_to_ui(&buf[..len]);
        }
    }
}

impl Plugin for Spectrum {
    fn new() -> Self {
        let mut window = [0.0_f32; FFT_SIZE];
        for i in 0..FFT_SIZE {
            window[i] = hann(i, FFT_SIZE);
        }
        let mut p = Self {
            chain: multipass::<U2>(),
            sample_rate: 48000.0,
            floor_db: FLOOR_DEFAULT,
            smooth: SMOOTH_DEFAULT,
            ring: [0.0; FFT_SIZE],
            ring_pos: 0,
            samples_since_fft: 0,
            hop: FFT_SIZE / 2,
            window,
            bands: [(0, 0); N_BINS],
            fft_buf: [0.0; FFT_SIZE],
            band_db: [FLOOR_DEFAULT as f32; N_BINS],
            frames_since_send: 0,
            send_interval_frames: 1440, // ~30 Hz @ 48 kHz; updated in activate.
            sent_sample_rate: false,
        };
        p.recompute_bands();
        p
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        AudioUnit::set_sample_rate(&mut self.chain, sample_rate);
        self.send_interval_frames = (sample_rate as f32 / 30.0) as u32;
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.samples_since_fft = 0;
        self.sent_sample_rate = false;
        self.recompute_bands();
    }

    fn reset(&mut self) {
        AudioUnit::reset(&mut self.chain);
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.samples_since_fft = 0;
        self.band_db.fill(self.floor_db as f32);
    }

    fn params() -> &'static [ParamDef] {
        PARAMS
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            PID_FLOOR => self.floor_db,
            PID_SMOOTH => self.smooth,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PID_FLOOR => self.floor_db = value.clamp(FLOOR_MIN, FLOOR_MAX),
            PID_SMOOTH => self.smooth = value.clamp(SMOOTH_MIN, SMOOTH_MAX),
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let stereo_in = ctx.input_channel_count() == 2 && ctx.output_channel_count() == 2;
        let mut n_processed: u32 = 0;

        if stereo_in {
            if let Some(io) = ctx.stereo_io() {
                let wclap_plugin::StereoIo {
                    input_l,
                    input_r,
                    output_l,
                    output_r,
                } = io;
                let n = input_l.len();
                n_processed = n as u32;

                let mut in_frame = [0.0_f32; 2];
                let mut out_frame = [0.0_f32; 2];
                for f in 0..n {
                    in_frame[0] = input_l[f];
                    in_frame[1] = input_r[f];
                    AudioUnit::tick(&mut self.chain, &in_frame, &mut out_frame);
                    output_l[f] = out_frame[0];
                    output_r[f] = out_frame[1];

                    let mono = (in_frame[0] + in_frame[1]) * 0.5;
                    self.ring[self.ring_pos] = mono;
                    self.ring_pos = (self.ring_pos + 1) % FFT_SIZE;
                    self.samples_since_fft += 1;
                }
            }
        }

        // Mono fallback.
        if n_processed == 0 {
            if let Some(io) = ctx.mono_io() {
                let wclap_plugin::MonoIo { input, output } = io;
                n_processed = input.len() as u32;
                for f in 0..input.len() {
                    let x = input[f];
                    output[f] = x;
                    self.ring[self.ring_pos] = x;
                    self.ring_pos = (self.ring_pos + 1) % FFT_SIZE;
                    self.samples_since_fft += 1;
                }
            }
        }

        // Hop-aligned FFT runs. With FFT_SIZE=1024 hop=512 this fires ~93×/s
        // at 48 kHz — we additionally gate the actual UI send on
        // `send_interval_frames` so the postMessage cost stays bounded.
        while self.samples_since_fft >= self.hop {
            self.samples_since_fft -= self.hop;
            self.frames_since_send += self.hop as u32;
            if self.frames_since_send >= self.send_interval_frames {
                self.frames_since_send = 0;
                if !self.sent_sample_rate {
                    self.send_sample_rate(ctx);
                    self.sent_sample_rate = true;
                }
                self.run_fft_and_send(ctx);
            }
        }

        let _ = n_processed;
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<Spectrum>(&PLUGIN_DEF);
}
