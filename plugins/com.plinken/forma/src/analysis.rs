//! Source import + spectral analysis.
//!
//! A [`Source`] is the mono PCM a slot plays. The [`Analyzer`] turns it
//! into a [`SpectralModel`]: the source approximated as [`LAYERS`] static
//! sounds, each with a gain envelope over the length of the source. The
//! decomposition is a rank-4 non-negative matrix factorisation of the
//! magnitude spectrogram; each layer's spectrum is then sampled at the
//! harmonics of the detected pitch and stored as band-limited single-cycle
//! wavetables, so the Spectral engine plays it back as an oscillator.
//!
//! The plugin's wasm runs on the audio thread, so the analysis is a state
//! machine advanced a small unit at a time from `process` — it never
//! blocks a render. Everything is allocated in [`Analyzer::new`]; `step`
//! allocates nothing.

use plinken_sample_core::SampleData;
use std::f32::consts::PI;
use std::sync::Arc;

pub const LAYERS: usize = 4;
pub const TABLE_LEN: usize = 2048;
/// Stored table length: one guard sample so interpolation never wraps.
pub const TABLE_STRIDE: usize = TABLE_LEN + 1;
/// Mip `m` keeps harmonics `1..=(MAX_HARMONICS >> m)`.
pub const MIPS: usize = 9;
pub const MAX_HARMONICS: usize = 512;
/// Table variants per layer: 0 = full spectrum, 1 = strongest partials only.
pub const VARIANTS: usize = 2;

/// Longest source kept per slot. Longer imports are truncated: the source
/// is saved inside the project state, so its size is the project's size.
pub const MAX_SOURCE_SECONDS: f32 = 20.0;
const MIN_SOURCE_FRAMES: usize = 64;

const FFT_N: usize = 2048;
const BINS: usize = FFT_N / 2 + 1;
const BANDS: usize = 64;
const MAX_FRAMES: usize = 1024;
const MIN_HOP: usize = 256;
const NMF_ITERS: usize = 60;
/// Root used to sample the layer spectra when the source has no clear
/// pitch (C3): dense enough to follow the spectral envelope.
const UNPITCHED_ROOT_HZ: f32 = 130.812_78;
/// Loudest-frame RMS the resynthesis is normalised to.
const TARGET_RMS: f32 = 0.25;
/// Frames per point of a source's level curve.
const LEVEL_BLOCK: usize = 512;
/// A source normalises to this peak.
const SOURCE_PEAK: f32 = 0.9;
/// Neighbourhood of the source level curve, in blocks each side (≈ 85 ms
/// at 48 kHz).
const LEVEL_SMOOTH_BLOCKS: usize = 8;
/// The same for the spectral model, in analysis frames each side.
const LEVEL_SMOOTH_FRAMES: usize = 3;

/// Mono source audio for one slot, peak-normalised.
pub struct Source {
    pub sr: f32,
    pub data: Vec<f32>,
    /// Peak level of each [`LEVEL_BLOCK`] frames (after normalising).
    pub level: Vec<f32>,
}

/// A level curve the make-up gain can follow without pumping: each point
/// becomes the loudest of its neighbourhood (`radius` points each side, the
/// curve treated as a loop), then the result is averaged over the same
/// span. Individual transients no longer swing the gain point to point.
pub fn smooth_level(peaks: &[f32], radius: usize) -> Vec<f32> {
    let n = peaks.len();
    if n == 0 {
        return Vec::new();
    }
    let radius = radius.min((n - 1) / 2);
    let around = |i: usize, d: usize| (i + n + d - radius) % n;
    let held: Vec<f32> = (0..n).map(|i| (0..=2 * radius).map(|d| peaks[around(i, d)]).fold(0.0f32, f32::max)).collect();
    (0..n).map(|i| (0..=2 * radius).map(|d| held[around(i, d)]).sum::<f32>() / (2 * radius + 1) as f32).collect()
}

/// Gain that evens out a recording's own dynamics. An oscillator made from
/// a quiet passage of the source should still be an audible oscillator: a
/// passage at a fifth of the loudest level plays at about 60 %, not 20 %.
/// Below 5 % of `reference` the boost stops growing, so silence stays quiet.
pub fn level_makeup(level: f32, reference: f32) -> f32 {
    if reference <= 0.0 {
        return 1.0;
    }
    (reference / level.max(0.05 * reference)).powf(0.7)
}

impl Source {
    /// Mix a decoded sample to mono, truncate to [`MAX_SOURCE_SECONDS`] and
    /// normalise the peak. `None` when there is too little audio to play.
    pub fn from_sample(s: &SampleData) -> Option<Source> {
        let sr = s.sample_rate as f32;
        if sr < 1000.0 {
            return None;
        }
        let max = (MAX_SOURCE_SECONDS * sr) as usize;
        let n = s.frame_count.min(s.left.len()).min(max);
        let stereo = s.channels == 2 && s.right.len() >= n;
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            data.push(if stereo { 0.5 * (s.left[i] + s.right[i]) } else { s.left[i] });
        }
        Self::from_mono(sr, data)
    }

    pub fn from_mono(sr: f32, mut data: Vec<f32>) -> Option<Source> {
        if data.len() < MIN_SOURCE_FRAMES || !sr.is_finite() || sr < 1000.0 {
            return None;
        }
        let mut peak = 0.0f32;
        for v in data.iter_mut() {
            if !v.is_finite() {
                *v = 0.0;
            }
            peak = peak.max(v.abs());
        }
        if peak > 1e-6 {
            let g = SOURCE_PEAK / peak;
            for v in data.iter_mut() {
                *v *= g;
            }
        }
        let peaks: Vec<f32> = data.chunks(LEVEL_BLOCK).map(|c| c.iter().fold(0.0f32, |p, v| p.max(v.abs()))).collect();
        Some(Source { sr, data, level: smooth_level(&peaks, LEVEL_SMOOTH_BLOCKS) })
    }

    /// Factory source so a fresh instance makes sound before anything is
    /// imported: 1.5 s of a C3 tone whose formant sweeps (variant 0), or
    /// the same with a breath-noise layer fading in (variant 1).
    pub fn builtin(variant: u32) -> Source {
        let sr = 48_000.0f32;
        let n = (sr * 1.5) as usize;
        let f0 = UNPITCHED_ROOT_HZ;
        let mut data = vec![0.0f32; n];
        let mut rng = 0x9e37_79b9u32 ^ variant;
        let mut lp = 0.0f32;
        // Each partial is a rotating phasor (two multiplies a sample) and
        // the formant gains are refreshed every 64 samples: this runs when
        // a plugin instance is created, on the host's audio thread, so it
        // must not cost a sine and an exponential per partial per sample.
        const PARTIALS: usize = 40;
        let mut phasor = [(0.0f32, 1.0f32); PARTIALS];
        let mut step = [(0.0f32, 0.0f32); PARTIALS];
        let mut gain = [0.0f32; PARTIALS];
        for (h, st) in step.iter_mut().enumerate() {
            let w = 2.0 * PI * f0 * (h + 1) as f32 / sr;
            *st = (w.sin(), w.cos());
        }
        for (i, out) in data.iter_mut().enumerate() {
            let t = i as f32 / n as f32;
            if i % 64 == 0 {
                let formant = 400.0 + 2200.0 * (0.5 - 0.5 * (2.0 * PI * t).cos());
                for (h, g) in gain.iter_mut().enumerate() {
                    let d = (f0 * (h + 1) as f32 - formant) / 500.0;
                    *g = (0.25 + (-d * d).exp()) / (h + 1) as f32;
                }
            }
            let mut v = 0.0f32;
            for h in 0..PARTIALS {
                let (s0, c0) = phasor[h];
                v += gain[h] * s0;
                let (ds, dc) = step[h];
                phasor[h] = (s0 * dc + c0 * ds, c0 * dc - s0 * ds);
            }
            if variant != 0 {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let white = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
                lp += 0.3 * (white - lp);
                v += 1.5 * t * lp;
            }
            *out = v;
        }
        Source::from_mono(sr, data).expect("builtin source is long enough")
    }

    pub fn seconds(&self) -> f32 {
        self.data.len() as f32 / self.sr
    }

    /// Make-up gain for a grain read at `pos` (0..1 of the source).
    pub fn makeup_at(&self, pos: f32) -> f32 {
        let n = self.level.len();
        let x = (pos - pos.floor()) * n as f32;
        let i = (x as usize).min(n - 1);
        let j = if i + 1 >= n { 0 } else { i + 1 };
        let level = self.level[i] + (self.level[j] - self.level[i]) * (x - i as f32);
        level_makeup(level, SOURCE_PEAK)
    }
}

/// The analysed form of a source: four static sounds and their envelopes.
pub struct SpectralModel {
    pub root_hz: f32,
    pub pitched: bool,
    pub frames: usize,
    /// Layer gain per analysis frame, `[layer * frames + frame]`. Layers
    /// are ordered dark → bright.
    pub env: Vec<f32>,
    /// `env` with the fast movement averaged out (the Smooth control).
    pub env_smooth: Vec<f32>,
    /// `[((layer * VARIANTS + variant) * MIPS + mip) * TABLE_STRIDE ..]`.
    pub tables: Vec<f32>,
    /// Source peak level per analysis frame (for the UI overview).
    pub peaks: Vec<f32>,
}

impl SpectralModel {
    #[inline]
    pub fn table(&self, layer: usize, variant: usize, mip: usize) -> &[f32] {
        let i = ((layer * VARIANTS + variant) * MIPS + mip) * TABLE_STRIDE;
        &self.tables[i..i + TABLE_STRIDE]
    }

    /// Layer gains at position `pos` (0..1, looped), blended towards the
    /// smoothed envelopes by `smooth`.
    pub fn gains_at(&self, pos: f32, smooth: f32) -> [f32; LAYERS] {
        let t = self.frames;
        let x = (pos - pos.floor()) * t as f32;
        let i0 = (x as usize).min(t - 1);
        let i1 = if i0 + 1 >= t { 0 } else { i0 + 1 };
        let f = x - i0 as f32;
        let mut out = [0.0; LAYERS];
        for (k, o) in out.iter_mut().enumerate() {
            let e = &self.env[k * t..(k + 1) * t];
            let s = &self.env_smooth[k * t..(k + 1) * t];
            let a = e[i0] + (e[i1] - e[i0]) * f;
            let b = s[i0] + (s[i1] - s[i0]) * f;
            *o = a + (b - a) * smooth;
        }
        out
    }
}

/// Iterative radix-2 complex FFT with precomputed tables.
struct Fft {
    n: usize,
    rev: Vec<u32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        debug_assert!(n.is_power_of_two());
        let bits = n.trailing_zeros();
        let rev = (0..n as u32).map(|i| i.reverse_bits() >> (32 - bits)).collect();
        let cos = (0..n / 2).map(|i| (2.0 * PI * i as f32 / n as f32).cos()).collect();
        let sin = (0..n / 2).map(|i| (2.0 * PI * i as f32 / n as f32).sin()).collect();
        Fft { n, rev, cos, sin }
    }

    /// In place, unnormalised: `X[k] = Σ x[n]·e^{-2πikn/N}`.
    fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            let mut start = 0;
            while start < n {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * c + im[b] * s;
                    let ti = im[b] * c - re[b] * s;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
                start += len;
            }
            len *= 2;
        }
    }

    /// In place, unnormalised: `x[n] = Σ X[k]·e^{+2πikn/N}`.
    fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        self.forward(im, re);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Bands(usize),
    Pitch,
    NmfH(usize),
    NmfW(usize),
    Sort,
    Layers(usize),
    Harmonics,
    Tables(usize),
    Done,
}

/// Incremental analysis of one source. Call [`Analyzer::step`] until it
/// returns `true`, then [`Analyzer::take_model`].
pub struct Analyzer {
    src: Arc<Source>,
    stage: Stage,
    fft: Fft,
    fft2: Fft,
    hann: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    re2: Vec<f32>,
    im2: Vec<f32>,
    mag: Vec<f32>,
    hop: usize,
    frames: usize,
    band_of: Vec<u8>,
    band_inv_n: [f32; BANDS],
    /// Band spectrogram, `[frame * BANDS + band]`.
    vb: Vec<f32>,
    energy: Vec<f32>,
    w: [[f32; LAYERS]; BANDS],
    /// Activations, `[layer * frames + frame]`.
    h: Vec<f32>,
    root_hz: f32,
    pitched: bool,
    root_known: bool,
    /// Full-resolution layer spectra, `[layer * BINS + bin]`.
    wfull: Vec<f32>,
    den: [f32; LAYERS],
    harm: Vec<f32>,
    pure: Vec<f32>,
    harm_cos: Vec<f32>,
    harm_sin: Vec<f32>,
    model: Option<SpectralModel>,
    done_units: usize,
    total_units: usize,
}

const TABLE_UNITS: usize = LAYERS * VARIANTS * MIPS;

impl Analyzer {
    pub fn new(src: Arc<Source>) -> Analyzer {
        let len = src.data.len();
        let hop = MIN_HOP.max(len.div_ceil(MAX_FRAMES));
        let frames = len.div_ceil(hop).clamp(1, MAX_FRAMES);

        // Log-spaced bands over bins 1..=1024, at least one bin wide.
        let mut band_of = vec![0u8; BINS];
        let mut band_inv_n = [0.0f32; BANDS];
        let mut lo = 1usize;
        for b in 0..BANDS {
            let ideal = ((BINS - 1) as f32).powf((b + 1) as f32 / BANDS as f32).round() as usize + 1;
            // Leave one bin for every band still to come.
            let hi = if b + 1 == BANDS { BINS } else { ideal.clamp(lo + 1, BINS - (BANDS - 1 - b)) };
            for slot in &mut band_of[lo..hi] {
                *slot = b as u8;
            }
            band_inv_n[b] = 1.0 / (hi - lo) as f32;
            lo = hi;
        }

        // Schroeder phases: low crest factor, and identical for every
        // layer so layers mix as spectra rather than cancelling.
        let phase = |h: usize| -PI * (h * (h - 1)) as f32 / MAX_HARMONICS as f32;
        let harm_cos = (0..=MAX_HARMONICS).map(|h| if h == 0 { 0.0 } else { phase(h).cos() }).collect();
        let harm_sin = (0..=MAX_HARMONICS).map(|h| if h == 0 { 0.0 } else { phase(h).sin() }).collect();

        let model = SpectralModel {
            root_hz: UNPITCHED_ROOT_HZ,
            pitched: false,
            frames,
            env: vec![0.0; LAYERS * frames],
            env_smooth: vec![0.0; LAYERS * frames],
            tables: vec![0.0; TABLE_UNITS * TABLE_STRIDE],
            peaks: vec![0.0; frames],
        };

        Analyzer {
            src,
            stage: Stage::Bands(0),
            fft: Fft::new(FFT_N),
            fft2: Fft::new(FFT_N * 2),
            hann: (0..FFT_N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT_N as f32).cos()).collect(),
            re: vec![0.0; FFT_N],
            im: vec![0.0; FFT_N],
            re2: vec![0.0; FFT_N * 2],
            im2: vec![0.0; FFT_N * 2],
            mag: vec![0.0; BINS],
            hop,
            frames,
            band_of,
            band_inv_n,
            vb: vec![0.0; BANDS * frames],
            energy: vec![0.0; frames],
            w: [[0.0; LAYERS]; BANDS],
            h: vec![0.0; LAYERS * frames],
            root_hz: UNPITCHED_ROOT_HZ,
            pitched: false,
            root_known: false,
            wfull: vec![0.0; LAYERS * BINS],
            den: [0.0; LAYERS],
            harm: vec![0.0; LAYERS * (MAX_HARMONICS + 1)],
            pure: vec![0.0; LAYERS * (MAX_HARMONICS + 1)],
            harm_cos,
            harm_sin,
            model: Some(model),
            done_units: 0,
            total_units: 2 * frames + 2 * NMF_ITERS + 3 + TABLE_UNITS,
        }
    }

    pub fn source(&self) -> &Arc<Source> {
        &self.src
    }

    /// 0..1.
    pub fn progress(&self) -> f32 {
        self.done_units as f32 / self.total_units as f32
    }

    /// Detected root once the pitch stage has run.
    pub fn root_hz(&self) -> Option<f32> {
        self.root_known.then_some(self.root_hz)
    }

    pub fn take_model(&mut self) -> Option<SpectralModel> {
        if self.stage == Stage::Done {
            self.model.take()
        } else {
            None
        }
    }

    /// Advance by one unit of work (about one FFT, or one NMF half-step).
    /// Returns `true` once the model is complete.
    pub fn step(&mut self) -> bool {
        let next = match self.stage {
            Stage::Bands(t) => {
                self.bands_frame(t);
                if t + 1 < self.frames { Stage::Bands(t + 1) } else { Stage::Pitch }
            }
            Stage::Pitch => {
                self.detect_pitch();
                self.nmf_init();
                Stage::NmfH(0)
            }
            Stage::NmfH(i) => {
                self.nmf_h();
                Stage::NmfW(i)
            }
            Stage::NmfW(i) => {
                self.nmf_w();
                if i + 1 < NMF_ITERS { Stage::NmfH(i + 1) } else { Stage::Sort }
            }
            Stage::Sort => {
                self.sort_layers();
                Stage::Layers(0)
            }
            Stage::Layers(t) => {
                self.layers_frame(t);
                if t + 1 < self.frames { Stage::Layers(t + 1) } else { Stage::Harmonics }
            }
            Stage::Harmonics => {
                self.harmonics_and_envelopes();
                Stage::Tables(0)
            }
            Stage::Tables(i) => {
                self.build_table(i);
                if i + 1 < TABLE_UNITS { Stage::Tables(i + 1) } else { Stage::Done }
            }
            Stage::Done => return true,
        };
        self.done_units += 1;
        self.stage = next;
        next == Stage::Done
    }

    /// Windowed magnitude spectrum of frame `t` into `self.mag` (DC zeroed).
    fn spectrum(&mut self, t: usize) {
        let data = &self.src.data;
        let start = t * self.hop;
        for i in 0..FFT_N {
            self.re[i] = data.get(start + i).copied().unwrap_or(0.0) * self.hann[i];
            self.im[i] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        self.mag[0] = 0.0;
        for f in 1..BINS {
            self.mag[f] = (self.re[f] * self.re[f] + self.im[f] * self.im[f]).sqrt();
        }
    }

    fn bands_frame(&mut self, t: usize) {
        self.spectrum(t);
        let row = &mut self.vb[t * BANDS..(t + 1) * BANDS];
        row.fill(0.0);
        let mut e = 0.0f32;
        for f in 1..BINS {
            let m = self.mag[f];
            row[self.band_of[f] as usize] += m;
            e += m * m;
        }
        for (v, inv) in row.iter_mut().zip(self.band_inv_n.iter()) {
            *v *= inv;
        }
        self.energy[t] = e;
        let data = &self.src.data;
        let (a, b) = ((t * self.hop).min(data.len()), ((t + 1) * self.hop).min(data.len()));
        let peak = data[a..b].iter().fold(0.0f32, |p, v| p.max(v.abs()));
        if let Some(m) = &mut self.model {
            m.peaks[t] = peak;
        }
    }

    /// Autocorrelation pitch estimate on the loudest frame.
    fn detect_pitch(&mut self) {
        let sr = self.src.sr;
        let mut best = 0;
        for t in 1..self.frames {
            if self.energy[t] > self.energy[best] {
                best = t;
            }
        }
        let data = &self.src.data;
        let start = best * self.hop;
        let n2 = FFT_N * 2;
        for i in 0..n2 {
            self.re2[i] = if i < FFT_N { data.get(start + i).copied().unwrap_or(0.0) * self.hann[i] } else { 0.0 };
            self.im2[i] = 0.0;
        }
        self.fft2.forward(&mut self.re2, &mut self.im2);
        for i in 0..n2 {
            self.re2[i] = self.re2[i] * self.re2[i] + self.im2[i] * self.im2[i];
            self.im2[i] = 0.0;
        }
        self.fft2.inverse(&mut self.re2, &mut self.im2);
        let r = &self.re2;
        self.root_known = true;
        self.root_hz = UNPITCHED_ROOT_HZ;
        self.pitched = false;
        if r[0] <= 1e-9 {
            return;
        }
        let lo = ((sr / 1000.0) as usize).max(2);
        let hi = ((sr / 40.0) as usize).min(FFT_N - 2);
        if lo + 2 >= hi {
            return;
        }
        // The Hann window tapers the correlation at long lags; divide by
        // the window's own autocorrelation so a low note is not under-rated
        // against its octave.
        let norm = |lag: usize| -> f32 {
            let x = lag as f32 / FFT_N as f32;
            let taper = (1.0 - x) * (2.0 / 3.0 + (2.0 * PI * x).cos() / 3.0) + (2.0 * PI * x).sin() / (2.0 * PI);
            r[lag] / (r[0] * taper.max(0.05))
        };
        let mut peak = 0.0f32;
        for lag in lo..hi {
            peak = peak.max(norm(lag));
        }
        if peak < 0.5 {
            return;
        }
        // First local maximum close to the global one: the fundamental
        // rather than one of its sub-octaves.
        for lag in lo + 1..hi - 1 {
            let v = norm(lag);
            if v >= 0.9 * peak && v >= norm(lag - 1) && v >= norm(lag + 1) {
                let (a, b, c) = (r[lag - 1], r[lag], r[lag + 1]);
                let d = a - 2.0 * b + c;
                let off = if d.abs() > 1e-12 { 0.5 * (a - c) / d } else { 0.0 };
                self.root_hz = sr / (lag as f32 + off.clamp(-0.5, 0.5));
                self.pitched = true;
                return;
            }
        }
    }

    fn nmf_init(&mut self) {
        let t = self.frames;
        let mean = self.vb.iter().sum::<f32>() / self.vb.len() as f32;
        // Fixed seed: the same source always yields the same model, so a
        // reloaded project sounds identical.
        let mut rng = 0x2545_f491u32;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            0.5 + rng as f32 / u32::MAX as f32
        };
        for row in self.w.iter_mut() {
            for v in row.iter_mut() {
                *v = next();
            }
        }
        let scale = mean.max(1e-9) / LAYERS as f32;
        for v in self.h[..LAYERS * t].iter_mut() {
            *v = next() * scale;
        }
    }

    /// Multiplicative update of the activations (Euclidean NMF).
    fn nmf_h(&mut self) {
        let t = self.frames;
        let mut wtw = [[0.0f32; LAYERS]; LAYERS];
        for row in self.w.iter() {
            for k in 0..LAYERS {
                for j in 0..LAYERS {
                    wtw[k][j] += row[k] * row[j];
                }
            }
        }
        for ti in 0..t {
            let v = &self.vb[ti * BANDS..(ti + 1) * BANDS];
            let mut num = [0.0f32; LAYERS];
            for (b, row) in self.w.iter().enumerate() {
                for k in 0..LAYERS {
                    num[k] += row[k] * v[b];
                }
            }
            let mut cur = [0.0f32; LAYERS];
            for k in 0..LAYERS {
                cur[k] = self.h[k * t + ti];
            }
            for k in 0..LAYERS {
                let mut den = 1e-12f32;
                for j in 0..LAYERS {
                    den += wtw[k][j] * cur[j];
                }
                self.h[k * t + ti] = cur[k] * num[k] / den;
            }
        }
    }

    /// Multiplicative update of the band spectra.
    fn nmf_w(&mut self) {
        let t = self.frames;
        let mut hht = [[0.0f32; LAYERS]; LAYERS];
        let mut num = [[0.0f32; LAYERS]; BANDS];
        for ti in 0..t {
            let mut cur = [0.0f32; LAYERS];
            for k in 0..LAYERS {
                cur[k] = self.h[k * t + ti];
            }
            for k in 0..LAYERS {
                for j in 0..LAYERS {
                    hht[k][j] += cur[k] * cur[j];
                }
            }
            let v = &self.vb[ti * BANDS..(ti + 1) * BANDS];
            for b in 0..BANDS {
                for k in 0..LAYERS {
                    num[b][k] += v[b] * cur[k];
                }
            }
        }
        for (row, n) in self.w.iter_mut().zip(num.iter()) {
            let cur = *row;
            for k in 0..LAYERS {
                let mut den = 1e-12f32;
                for j in 0..LAYERS {
                    den += cur[j] * hht[j][k];
                }
                row[k] = cur[k] * n[k] / den;
            }
        }
    }

    /// Order layers dark → bright by spectral centroid (Tilt relies on it).
    fn sort_layers(&mut self) {
        let t = self.frames;
        let mut centroid = [0.0f32; LAYERS];
        for (k, c) in centroid.iter_mut().enumerate() {
            let (mut a, mut s) = (0.0f32, 1e-12f32);
            for (b, row) in self.w.iter().enumerate() {
                a += b as f32 * row[k];
                s += row[k];
            }
            *c = a / s;
        }
        // Selection sort with swaps: four elements, and no allocation.
        for i in 0..LAYERS {
            let mut m = i;
            for j in i + 1..LAYERS {
                if centroid[j] < centroid[m] {
                    m = j;
                }
            }
            if m != i {
                centroid.swap(i, m);
                for row in self.w.iter_mut() {
                    row.swap(i, m);
                }
                for ti in 0..t {
                    self.h.swap(i * t + ti, m * t + ti);
                }
            }
        }
    }

    /// Second pass: split each full-resolution frame between the layers
    /// by the band model's share, accumulating each layer's own spectrum.
    fn layers_frame(&mut self, t: usize) {
        self.spectrum(t);
        let frames = self.frames;
        let mut cur = [0.0f32; LAYERS];
        for k in 0..LAYERS {
            cur[k] = self.h[k * frames + t];
            self.den[k] += cur[k] * cur[k];
        }
        for f in 1..BINS {
            let row = &self.w[self.band_of[f] as usize];
            let mut part = [0.0f32; LAYERS];
            let mut sum = 1e-12f32;
            for k in 0..LAYERS {
                part[k] = row[k] * cur[k];
                sum += part[k];
            }
            let scale = self.mag[f] / sum;
            for k in 0..LAYERS {
                self.wfull[k * BINS + f] += scale * part[k] * cur[k];
            }
        }
    }

    fn harmonics_and_envelopes(&mut self) {
        let frames = self.frames;
        let bin_hz = self.src.sr / FFT_N as f32;
        let root_bins = self.root_hz / bin_hz;
        let hs = MAX_HARMONICS + 1;
        let mut rms = [0.0f32; LAYERS];
        let mut rms_pure = [0.0f32; LAYERS];
        for k in 0..LAYERS {
            let inv = 1.0 / (self.den[k] + 1e-12);
            let spec = &self.wfull[k * BINS..(k + 1) * BINS];
            let mut max = 0.0f32;
            for h in 1..=MAX_HARMONICS {
                let p = h as f32 * root_bins;
                let a = if p >= (BINS - 1) as f32 {
                    0.0
                } else if root_bins >= 3.0 {
                    // Take the partial's peak, wherever it sits near k·f0.
                    let lo = ((p - root_bins / 3.0).round() as usize).max(1);
                    let hi = ((p + root_bins / 3.0).round() as usize).min(BINS - 1);
                    spec[lo..=hi].iter().fold(0.0f32, |m, v| m.max(*v))
                } else {
                    let i = p as usize;
                    spec[i] + (spec[i + 1] - spec[i]) * (p - i as f32)
                } * inv;
                self.harm[k * hs + h] = a;
                max = max.max(a);
            }
            let (mut e, mut ep) = (0.0f32, 0.0f32);
            for h in 1..=MAX_HARMONICS {
                let a = self.harm[k * hs + h];
                let x = if max > 0.0 { a / max } else { 0.0 };
                let pa = a * x * x * x;
                self.pure[k * hs + h] = pa;
                e += a * a;
                ep += pa * pa;
            }
            rms[k] = (0.5 * e).sqrt();
            rms_pure[k] = (0.5 * ep).sqrt();
        }

        // Loudest-frame level of the resynthesis, from a sample of frames.
        let mut peak_ms = 0.0f32;
        let stride = frames.div_ceil(64);
        let mut t = 0;
        while t < frames {
            let mut ms = 0.0f32;
            for h in 1..=MAX_HARMONICS {
                let mut a = 0.0f32;
                for k in 0..LAYERS {
                    a += self.h[k * frames + t] * self.harm[k * hs + h];
                }
                ms += 0.5 * a * a;
            }
            peak_ms = peak_ms.max(ms);
            t += stride;
        }
        let scale = if peak_ms > 1e-18 { TARGET_RMS / peak_ms.sqrt() } else { 0.0 };

        let Some(model) = &mut self.model else { return };
        model.root_hz = self.root_hz;
        model.pitched = self.pitched;
        // Not allocation-free, but this unit runs once per import.
        let level = smooth_level(&model.peaks, LEVEL_SMOOTH_FRAMES);
        let top = level.iter().fold(0.0f32, |p, v| p.max(*v));
        for k in 0..LAYERS {
            for t in 0..frames {
                // The envelopes carry the source's timbre over time, with
                // its loudness swings evened out (see `level_makeup`).
                let makeup = level_makeup(level[t], top);
                model.env[k * frames + t] = self.h[k * frames + t] * rms[k] * scale * makeup;
            }
            // Tables are stored at unit RMS; the envelope carries the level.
            let (n, np) = (rms[k], rms_pure[k]);
            for h in 1..=MAX_HARMONICS {
                self.harm[k * hs + h] = if n > 0.0 { self.harm[k * hs + h] / n } else { 0.0 };
                self.pure[k * hs + h] = if np > 0.0 { self.pure[k * hs + h] / np } else { 0.0 };
            }
            // Circular moving average over a twelfth of the source each side.
            let radius = (frames / 12).max(1).min(frames.saturating_sub(1) / 2);
            let span = (2 * radius + 1) as f32;
            let env = &model.env[k * frames..(k + 1) * frames];
            let mut acc = 0.0f32;
            for d in 0..=2 * radius {
                acc += env[(frames + d - radius) % frames];
            }
            for t in 0..frames {
                model.env_smooth[k * frames + t] = acc / span;
                acc += env[(t + radius + 1) % frames] - env[(t + frames - radius) % frames];
            }
        }
    }

    fn build_table(&mut self, i: usize) {
        let mip = i % MIPS;
        let variant = (i / MIPS) % VARIANTS;
        let layer = i / (MIPS * VARIANTS);
        let hs = MAX_HARMONICS + 1;
        let amps = if variant == 0 { &self.harm } else { &self.pure };
        self.re.fill(0.0);
        self.im.fill(0.0);
        for h in 1..=(MAX_HARMONICS >> mip) {
            let a = 0.5 * amps[layer * hs + h];
            let (c, s) = (a * self.harm_cos[h], a * self.harm_sin[h]);
            self.re[h] = c;
            self.im[h] = s;
            self.re[FFT_N - h] = c;
            self.im[FFT_N - h] = -s;
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        let Some(model) = &mut self.model else { return };
        let out = &mut model.tables[i * TABLE_STRIDE..(i + 1) * TABLE_STRIDE];
        out[..TABLE_LEN].copy_from_slice(&self.re);
        out[TABLE_LEN] = self.re[0];
    }
}

/// Run an analysis to completion (tests, and offline callers).
pub fn analyze_blocking(src: Arc<Source>) -> SpectralModel {
    let mut a = Analyzer::new(src);
    while !a.step() {}
    a.take_model().expect("analysis finished")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(f0: f32, harmonics: u32, seconds: f32) -> Arc<Source> {
        let sr = 48_000.0;
        let n = (sr * seconds) as usize;
        let data = (0..n)
            .map(|i| {
                let ph = 2.0 * PI * f0 * i as f32 / sr;
                (1..=harmonics).map(|h| (ph * h as f32).sin() / h as f32).sum::<f32>()
            })
            .collect();
        Arc::new(Source::from_mono(sr, data).unwrap())
    }

    #[test]
    fn fft_round_trips() {
        let fft = Fft::new(64);
        let orig: Vec<f32> = (0..64).map(|i| ((i * 7 % 13) as f32) - 6.0).collect();
        let (mut re, mut im) = (orig.clone(), vec![0.0; 64]);
        fft.forward(&mut re, &mut im);
        fft.inverse(&mut re, &mut im);
        for (a, b) in re.iter().zip(orig.iter()) {
            assert!((a / 64.0 - b).abs() < 1e-4);
        }
    }

    #[test]
    fn detects_the_pitch_of_a_harmonic_tone() {
        for f0 in [82.41f32, 220.0, 440.0, 880.0] {
            let m = analyze_blocking(tone(f0, 12, 0.5));
            assert!(m.pitched, "{f0} Hz not detected as pitched");
            let cents = 1200.0 * (m.root_hz / f0).log2();
            assert!(cents.abs() < 20.0, "{f0} Hz detected as {} Hz", m.root_hz);
        }
    }

    #[test]
    fn noise_is_unpitched() {
        let mut rng = 1u32;
        let data = (0..24_000)
            .map(|_| {
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (rng >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        let m = analyze_blocking(Arc::new(Source::from_mono(48_000.0, data).unwrap()));
        assert!(!m.pitched);
    }

    #[test]
    fn model_is_finite_normalised_and_deterministic() {
        let a = analyze_blocking(Arc::new(Source::builtin(1)));
        let b = analyze_blocking(Arc::new(Source::builtin(1)));
        assert_eq!(a.env, b.env);
        assert_eq!(a.tables, b.tables);
        assert!(a.env.iter().chain(a.tables.iter()).all(|v| v.is_finite()));
        assert!(a.env.iter().all(|v| *v >= 0.0));
        // Full-spectrum tables are unit RMS wherever the layer has content.
        for k in 0..LAYERS {
            let t = &a.table(k, 0, 0)[..TABLE_LEN];
            let rms = (t.iter().map(|v| v * v).sum::<f32>() / TABLE_LEN as f32).sqrt();
            assert!(rms == 0.0 || (rms - 1.0).abs() < 0.05, "layer {k} rms {rms}");
        }
        assert!(a.env.iter().fold(0.0f32, |m, v| m.max(*v)) > 0.01);
    }

    #[test]
    fn resynthesis_keeps_the_spectrum_of_a_static_tone() {
        // A static saw-like tone: the summed layers must reproduce 1/h.
        let m = analyze_blocking(tone(220.0, 20, 1.0));
        let gains = m.gains_at(0.5, 0.0);
        let fft = Fft::new(TABLE_LEN);
        let mut re = vec![0.0f32; TABLE_LEN];
        let mut im = vec![0.0f32; TABLE_LEN];
        for k in 0..LAYERS {
            for (o, v) in re.iter_mut().zip(m.table(k, 0, 0).iter()) {
                *o += gains[k] * v;
            }
        }
        fft.forward(&mut re, &mut im);
        let amp = |h: usize| (re[h] * re[h] + im[h] * im[h]).sqrt();
        let a1 = amp(1);
        assert!(a1 > 0.0);
        for h in 2..=10 {
            let ratio = amp(h) / a1;
            let want = 1.0 / h as f32;
            assert!((ratio - want).abs() < 0.25 * want, "harmonic {h}: {ratio} vs {want}");
        }
        // Nothing above the source's 20 harmonics.
        assert!(amp(30) / a1 < 0.01);
    }

    #[test]
    fn layers_follow_a_timbre_change() {
        // First half dull, second half bright: the darkest and brightest
        // layers must swap dominance across the source.
        let sr = 48_000.0;
        let n = 48_000usize;
        let data = (0..n)
            .map(|i| {
                let ph = 2.0 * PI * 110.0 * i as f32 / sr;
                if i < n / 2 {
                    ph.sin()
                } else {
                    (10..=30).map(|h| (ph * h as f32).sin() * 0.2).sum::<f32>()
                }
            })
            .collect();
        let m = analyze_blocking(Arc::new(Source::from_mono(sr, data).unwrap()));
        let early = m.gains_at(0.2, 0.0);
        let late = m.gains_at(0.8, 0.0);
        assert!(early[0] > 4.0 * early[LAYERS - 1], "early {early:?}");
        assert!(late[LAYERS - 1] > 4.0 * late[0], "late {late:?}");
    }

    #[test]
    fn short_and_silent_sources_do_not_break() {
        assert!(Source::from_mono(48_000.0, vec![0.0; 10]).is_none());
        let m = analyze_blocking(Arc::new(Source::from_mono(48_000.0, vec![0.0; 500]).unwrap()));
        assert!(m.env.iter().chain(m.tables.iter()).all(|v| *v == 0.0));
        let m = analyze_blocking(Arc::new(Source::from_mono(8_000.0, vec![0.3; 100]).unwrap()));
        assert!(m.env.iter().chain(m.tables.iter()).all(|v| v.is_finite()));
    }
}
