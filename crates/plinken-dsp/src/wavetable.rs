//! Band-limited wavetables: a table is a row of single-cycle frames the
//! oscillator scans across, each frame stored once per octave with the
//! harmonics above that octave's Nyquist limit removed.
//!
//! A frame is described by its harmonic series ([`Spectrum`]), never by
//! raw samples, so removing harmonics is exact: every octave level is an
//! inverse FFT of the same spectrum cut at a different harmonic. That is
//! what keeps a wavetable free of aliasing at any pitch.
//!
//! Everything here that allocates ([`Wavetable::from_spectra`], the FFT)
//! runs at build time. [`Wavetable::read`] is the only audio-path entry:
//! no allocation, no branches on data, two table lookups.

/// Samples per frame at the lowest octave level.
pub const FRAME_LEN: usize = 2048;
/// Harmonics kept at the lowest octave level. With 2048 samples that is
/// 4× oversampled, which the cubic reader needs; notes below ~47 Hz lose
/// only partials above 12 kHz.
pub const MAX_HARMONICS: usize = 512;
/// Octave levels: 512, 256, … 1 harmonics.
pub const LEVELS: usize = 10;

/// Upper levels hold few harmonics; below 64 samples a cubic read of
/// even a plain sine distorts audibly.
const MIN_LEVEL_LEN: usize = 64;
/// Each stored frame carries one wrapped sample before and two after, so
/// the 4-point read never needs a modulo.
const GUARD: usize = 3;
/// Resolution a time-domain recipe is sampled at before analysis. High
/// enough that the aliasing of a hard edge lands below -80 dB in the 512
/// harmonics we keep.
const ANALYSIS_LEN: usize = 16384;

const fn level_len(level: usize) -> usize {
    let n = FRAME_LEN >> level;
    if n < MIN_LEVEL_LEN {
        MIN_LEVEL_LEN
    } else {
        n
    }
}

const fn level_harmonics(level: usize) -> usize {
    MAX_HARMONICS >> level
}

/// The level whose harmonics all stay below Nyquist for a phase
/// increment (cycles per sample): the smallest `k` with
/// `(512 >> k) * inc <= 0.5`, i.e. `ceil(log2(1024 * inc))`.
#[inline]
fn level_for(phase_inc: f32) -> usize {
    let x = phase_inc * (2 * MAX_HARMONICS) as f32;
    if x <= 1.0 {
        return 0;
    }
    // ceil(log2 x) off the float bits: the exponent, plus one unless x
    // is an exact power of two.
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    let level = exp + (bits & 0x007f_ffff != 0) as i32;
    (level as usize).min(LEVELS - 1)
}

/// One single-cycle frame as a harmonic series:
/// `x(t) = Σ cos[h]·cos(2πht) + sin[h]·sin(2πht)` for `h = 1..=512`.
/// Index 0 is unused — a frame never carries DC.
#[derive(Clone)]
pub struct Spectrum {
    pub cos: Vec<f32>,
    pub sin: Vec<f32>,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new()
    }
}

impl Spectrum {
    pub fn new() -> Self {
        Self {
            cos: vec![0.0; MAX_HARMONICS + 1],
            sin: vec![0.0; MAX_HARMONICS + 1],
        }
    }

    /// Build from the sine amplitude of each harmonic (`f(h)`), the usual
    /// way to write a classic waveform.
    pub fn from_sines(f: impl Fn(usize) -> f64) -> Self {
        let mut s = Self::new();
        for h in 1..=MAX_HARMONICS {
            s.sin[h] = f(h) as f32;
        }
        s
    }

    /// Analyse one cycle given as a function of phase `t` in `[0, 1)`.
    /// For recipes that are easier to write as a waveform (sync, FM,
    /// folding) than as a harmonic series.
    pub fn from_fn(f: impl Fn(f64) -> f64) -> Self {
        let n = ANALYSIS_LEN;
        let mut re: Vec<f64> = (0..n).map(|j| f(j as f64 / n as f64)).collect();
        let mut im = vec![0.0f64; n];
        fft(&mut re, &mut im, false);
        let mut s = Self::new();
        let scale = 2.0 / n as f64;
        for h in 1..=MAX_HARMONICS {
            s.cos[h] = (re[h] * scale) as f32;
            s.sin[h] = (-im[h] * scale) as f32;
        }
        s
    }

    /// Blend two frames harmonic by harmonic (`t` = 0 → `a`, 1 → `b`).
    pub fn lerp(a: &Spectrum, b: &Spectrum, t: f32) -> Self {
        let mix = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(x, y)| x + (y - x) * t).collect();
        Self {
            cos: mix(&a.cos, &b.cos),
            sin: mix(&a.sin, &b.sin),
        }
    }

    /// Render one cycle of `len` samples from the first `harmonics`
    /// partials. `len` must be a power of two above `2 * harmonics`.
    fn render(&self, len: usize, harmonics: usize, re: &mut Vec<f64>, im: &mut Vec<f64>) {
        re.clear();
        re.resize(len, 0.0);
        im.clear();
        im.resize(len, 0.0);
        for h in 1..=harmonics {
            let (c, s) = (self.cos[h] as f64 * 0.5, self.sin[h] as f64 * 0.5);
            re[h] = c;
            im[h] = -s;
            re[len - h] = c;
            im[len - h] = s;
        }
        fft(re, im, true);
    }
}

/// In-place radix-2 FFT, unnormalised in both directions
/// (`inverse` flips the sign of the exponent only).
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = sign * core::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                // Computing each twiddle directly keeps the error flat
                // across the block; a running product drifts at 16384.
                let (ws, wc) = (step * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * wc - im[b] * ws;
                let ti = re[b] * ws + im[b] * wc;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// A wavetable ready to play: every frame at every octave level.
pub struct Wavetable {
    frames: usize,
    /// Level by level, frame by frame; each frame is
    /// `[last, s0 … s(n-1), s0, s1]`.
    data: Vec<f32>,
    level_offset: [usize; LEVELS],
}

impl Wavetable {
    /// Build the octave levels for a row of frames. Each frame is
    /// normalised to a peak of 1 so scanning a table never jumps in
    /// level. Panics on an empty slice — a table without frames is a
    /// programming error, not a runtime condition.
    pub fn from_spectra(spectra: &[Spectrum]) -> Self {
        assert!(!spectra.is_empty(), "a wavetable needs at least one frame");
        let frames = spectra.len();
        let mut level_offset = [0usize; LEVELS];
        let mut total = 0;
        for (level, offset) in level_offset.iter_mut().enumerate() {
            *offset = total;
            total += frames * (level_len(level) + GUARD);
        }
        let mut data = vec![0.0f32; total];
        let (mut re, mut im) = (Vec::new(), Vec::new());

        for (f, spectrum) in spectra.iter().enumerate() {
            let mut gain = 1.0f64;
            for (level, &offset) in level_offset.iter().enumerate() {
                let n = level_len(level);
                spectrum.render(n, level_harmonics(level), &mut re, &mut im);
                if level == 0 {
                    // The full-band level sets the gain for all of them,
                    // so the levels stay matched as pitch moves.
                    let peak = re.iter().fold(0.0f64, |m, x| m.max(x.abs()));
                    if peak > 1e-9 {
                        gain = 1.0 / peak;
                    }
                }
                let at = offset + f * (n + GUARD);
                let out = &mut data[at..at + n + GUARD];
                out[0] = (re[n - 1] * gain) as f32;
                for (o, x) in out[1..=n].iter_mut().zip(re.iter()) {
                    *o = (x * gain) as f32;
                }
                out[n + 1] = (re[0] * gain) as f32;
                out[n + 2] = (re[1] * gain) as f32;
            }
        }
        Self {
            frames,
            data,
            level_offset,
        }
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Read the table. `phase` in `[0, 1)`, `phase_inc` in cycles per
    /// sample (picks the octave level), `position` in `[0, 1]` across the
    /// frames (adjacent frames are crossfaded).
    #[inline]
    pub fn read(&self, phase: f32, phase_inc: f32, position: f32) -> f32 {
        let level = level_for(phase_inc.abs());
        let n = level_len(level);
        let stride = n + GUARD;

        let last = self.frames - 1;
        let pos = position.clamp(0.0, 1.0) * last as f32;
        let frame_a = (pos as usize).min(last);
        let frame_b = (frame_a + 1).min(last);
        let frame_mix = pos - frame_a as f32;

        let x = phase.clamp(0.0, 1.0) * n as f32;
        let i = (x as usize).min(n - 1);
        let frac = x - i as f32;

        let base = self.level_offset[level] + i;
        let a = hermite(&self.data[base + frame_a * stride..][..4], frac);
        let b = hermite(&self.data[base + frame_b * stride..][..4], frac);
        a + (b - a) * frame_mix
    }
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) between `y[1]` and `y[2]`.
#[inline]
fn hermite(y: &[f32], t: f32) -> f32 {
    let c1 = 0.5 * (y[2] - y[0]);
    let c2 = y[0] - 2.5 * y[1] + 2.0 * y[2] - 0.5 * y[3];
    let c3 = 0.5 * (y[3] - y[0]) + 1.5 * (y[1] - y[2]);
    ((c3 * t + c2) * t + c1) * t + y[1]
}

/// An ordered set of wavetables an oscillator selects from by index.
pub struct WavetableBank {
    tables: Vec<Wavetable>,
}

impl WavetableBank {
    pub fn new(tables: Vec<Wavetable>) -> Self {
        Self { tables }
    }

    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Wavetable> {
        self.tables.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw() -> Spectrum {
        Spectrum::from_sines(|h| -2.0 / (core::f64::consts::PI * h as f64))
    }

    /// Magnitude of one harmonic of a rendered buffer holding whole cycles.
    fn bin(buf: &[f32], cycles: f64) -> f64 {
        let n = buf.len() as f64;
        let (mut c, mut s) = (0.0f64, 0.0f64);
        for (j, x) in buf.iter().enumerate() {
            let a = core::f64::consts::TAU * cycles * j as f64 / n;
            c += *x as f64 * a.cos();
            s += *x as f64 * a.sin();
        }
        2.0 * (c * c + s * s).sqrt() / n
    }

    #[test]
    fn fft_round_trips() {
        let n = 256;
        let src: Vec<f64> = (0..n).map(|j| ((j * 7919) % 101) as f64 / 50.0 - 1.0).collect();
        let mut re = src.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(&src) {
            assert!((a / n as f64 - b).abs() < 1e-9);
        }
    }

    #[test]
    fn analysis_recovers_a_known_series() {
        let s = Spectrum::from_fn(|t| {
            let a = core::f64::consts::TAU * t;
            0.5 * a.sin() + 0.25 * (3.0 * a).cos()
        });
        assert!((s.sin[1] - 0.5).abs() < 1e-5);
        assert!((s.cos[3] - 0.25).abs() < 1e-5);
        assert!(s.sin[2].abs() < 1e-5 && s.cos[1].abs() < 1e-5);
    }

    #[test]
    fn level_tracks_the_nyquist_limit() {
        for &inc in &[1e-4f32, 0.001, 0.0045, 0.01, 0.03, 0.11, 0.26, 0.45] {
            let level = level_for(inc);
            let harmonics = level_harmonics(level) as f32;
            assert!(harmonics * inc <= 0.5 || level == LEVELS - 1, "inc={inc} aliases");
            if level > 0 {
                // Never darker than it has to be: the level below would alias.
                assert!(level_harmonics(level - 1) as f32 * inc > 0.5, "inc={inc} too dull");
            }
        }
        assert_eq!(level_for(1.0 / 1024.0), 0, "exact power of two stays put");
    }

    #[test]
    fn frames_are_normalised_and_dc_free() {
        let table = Wavetable::from_spectra(&[saw()]);
        let n = 4096;
        let buf: Vec<f32> = (0..n).map(|j| table.read(j as f32 / n as f32, 1e-5, 0.0)).collect();
        let peak = buf.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let mean = buf.iter().sum::<f32>() / n as f32;
        assert!((peak - 1.0).abs() < 0.02, "peak={peak}");
        assert!(mean.abs() < 1e-3, "mean={mean}");
    }

    #[test]
    fn high_notes_carry_no_partials_above_nyquist() {
        let table = Wavetable::from_spectra(&[saw()]);
        // 2640 Hz at 48 kHz: 9 harmonics fit below Nyquist, so the reader
        // must pick the 8-harmonic level.
        let inc = 0.055f32;
        let n = 8192;
        let buf: Vec<f32> = (0..n).map(|j| table.read(j as f32 / n as f32, inc, 0.0)).collect();
        assert!(bin(&buf, 1.0) > 0.3, "fundamental present");
        assert!(bin(&buf, 8.0) > 0.03, "highest legal partial kept");
        for h in 9..40 {
            let m = bin(&buf, h as f64);
            assert!(m < 2e-3, "harmonic {h} would alias: {m}");
        }
    }

    #[test]
    fn position_crossfades_between_frames() {
        let sine = Spectrum::from_sines(|h| if h == 1 { 1.0 } else { 0.0 });
        let third = Spectrum::from_sines(|h| if h == 3 { 1.0 } else { 0.0 });
        let table = Wavetable::from_spectra(&[sine, third]);
        let n = 4096;
        let render = |pos: f32| -> Vec<f32> { (0..n).map(|j| table.read(j as f32 / n as f32, 1e-4, pos)).collect() };
        let (a, mid, b) = (render(0.0), render(0.5), render(1.0));
        assert!(bin(&a, 1.0) > 0.99 && bin(&a, 3.0) < 1e-3);
        assert!(bin(&b, 3.0) > 0.99 && bin(&b, 1.0) < 1e-3);
        assert!((bin(&mid, 1.0) - 0.5).abs() < 0.01 && (bin(&mid, 3.0) - 0.5).abs() < 0.01);
        // Out-of-range positions hold the end frames.
        assert_eq!(table.read(0.3, 1e-4, -2.0), table.read(0.3, 1e-4, 0.0));
        assert_eq!(table.read(0.3, 1e-4, 9.0), table.read(0.3, 1e-4, 1.0));
    }

    #[test]
    fn read_is_continuous_across_the_cycle_seam() {
        let table = Wavetable::from_spectra(&[Spectrum::from_sines(|h| if h == 1 { 1.0 } else { 0.0 })]);
        let before = table.read(0.999_99, 1e-4, 0.0);
        let after = table.read(0.0, 1e-4, 0.0);
        assert!((before - after).abs() < 1e-3, "{before} vs {after}");
    }
}
