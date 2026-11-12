//! Grain engine — one instance per voice per slot.
//!
//! Grains are spawned by a phase accumulator running at the grain rate.
//! When that rate follows the played note the stream of grains is periodic
//! at the note's frequency, so the source snippet becomes the waveform of
//! an oscillator: rate sets the pitch, grain size and playback speed set
//! the timbre. Onsets are placed with sub-sample accuracy — rounding them
//! to whole samples would detune and roughen high notes.

/// Enough for the largest Size (16 overlapping grains) plus jitter bunching.
pub const MAX_GRAINS: usize = 24;
const MIN_GRAIN_SAMPLES: f32 = 4.0;

#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Read position in source frames, kept inside `0..len`.
    pos: f64,
    /// Source frames advanced per output sample.
    speed: f64,
    /// Output samples since onset (fractional at spawn).
    age: f32,
    inv_dur: f32,
    gain_l: f32,
    gain_r: f32,
}

/// Control-rate inputs, constant across a control block.
#[derive(Clone, Copy)]
pub struct GrainParams {
    /// Grains per output sample.
    pub rate: f32,
    /// Grain length in grain periods.
    pub size: f32,
    /// 0 = nearly rectangular window, 1 = fully rounded.
    pub shape: f32,
    /// Source frames per output sample.
    pub speed: f64,
    /// Read position, 0..1 of the source.
    pub pos: f32,
    pub scatter: f32,
    pub jitter: f32,
    pub spread: f32,
    pub gain: f32,
    /// One cycle of the source at its detected pitch, in source frames, or
    /// 0 when the source has no clear pitch. Grains then start a whole
    /// number of cycles apart, so neighbouring grains line up in phase and
    /// add instead of beating against each other while the position moves.
    pub period: f32,
}

#[derive(Clone)]
pub struct GrainVoice {
    grains: [Grain; MAX_GRAINS],
    phase: f32,
    /// Phase at which the next grain starts (1 ± jitter).
    next_at: f32,
    rng: u32,
}

impl GrainVoice {
    pub fn new(seed: u32) -> Self {
        GrainVoice { grains: [Grain::default(); MAX_GRAINS], phase: 1.0, next_at: 1.0, rng: seed | 1 }
    }

    /// Silence all grains; the next `tick` starts a grain immediately.
    pub fn reset(&mut self) {
        for g in self.grains.iter_mut() {
            g.active = false;
        }
        self.phase = 1.0;
        self.next_at = 1.0;
    }

    #[inline]
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    fn spawn(&mut self, src: &[f32], p: &GrainParams, age: f32) {
        let Some(slot) = self.grains.iter().position(|g| !g.active) else { return };
        let len = src.len() as f64;
        let dur = (p.size / p.rate).max(MIN_GRAIN_SAMPLES);
        let scatter = if p.scatter > 0.0 { p.scatter * (self.rand() - 0.5) } else { 0.0 };
        let pan = if p.spread > 0.0 { p.spread * (self.rand() * 2.0 - 1.0) } else { 0.0 };
        let mut start = (p.pos + scatter).rem_euclid(1.0) as f64 * len;
        if p.period >= 2.0 {
            let period = p.period as f64;
            start = (start / period).round() * period;
        }
        let theta = (pan + 1.0) * core::f32::consts::FRAC_PI_4;
        let g = &mut self.grains[slot];
        g.active = true;
        g.speed = p.speed;
        g.pos = (start + age as f64 * p.speed).rem_euclid(len);
        g.age = age;
        g.inv_dur = 1.0 / dur;
        g.gain_l = p.gain * core::f32::consts::SQRT_2 * theta.cos();
        g.gain_r = p.gain * core::f32::consts::SQRT_2 * theta.sin();
    }

    /// Render one stereo sample. `src` must hold at least four frames.
    #[inline]
    pub fn tick(&mut self, src: &[f32], p: &GrainParams) -> (f32, f32) {
        self.phase += p.rate;
        while self.phase >= self.next_at {
            self.phase -= self.next_at;
            // The grain started `phase / rate` samples before this one.
            self.spawn(src, p, self.phase / p.rate);
            self.next_at = if p.jitter > 0.0 { (1.0 + p.jitter * (self.rand() * 2.0 - 1.0)).max(0.05) } else { 1.0 };
        }

        let len = src.len();
        let lenf = len as f64;
        let fade = (p.shape * 0.5).max(0.02);
        let inv_fade = 1.0 / fade;
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for g in self.grains.iter_mut() {
            if !g.active {
                continue;
            }
            let t = g.age * g.inv_dur;
            if t >= 1.0 {
                g.active = false;
                continue;
            }
            // Smoothstep fades: a Tukey-style window without a cosine per sample.
            let edge = t.min(1.0 - t);
            let w = if edge < fade {
                let x = edge * inv_fade;
                x * x * (3.0 - 2.0 * x)
            } else {
                1.0
            };
            let s = hermite(src, len, g.pos) * w;
            l += s * g.gain_l;
            r += s * g.gain_r;
            g.age += 1.0;
            g.pos += g.speed;
            if g.pos >= lenf {
                g.pos -= lenf;
            } else if g.pos < 0.0 {
                g.pos += lenf;
            }
        }
        (l, r)
    }
}

/// 4-point Hermite read at `pos` (inside `0..len`), wrapping at the ends.
#[inline]
fn hermite(src: &[f32], len: usize, pos: f64) -> f32 {
    let i = (pos as usize).min(len - 1);
    let f = (pos - i as f64) as f32;
    let im1 = if i == 0 { len - 1 } else { i - 1 };
    let i1 = if i + 1 >= len { i + 1 - len } else { i + 1 };
    let i2 = if i + 2 >= len { i + 2 - len } else { i + 2 };
    let (y0, y1, y2, y3) = (src[im1], src[i], src[i1], src[i2]);
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * f + c2) * f + c1) * f + y1
}

/// Gain that keeps overlapping grains at the level of the source: grains
/// in step add in amplitude, scattered ones in power.
pub fn overlap_gain(size: f32, shape: f32, scatter: f32) -> f32 {
    let overlap = (size * (1.0 - 0.5 * shape)).max(1.0);
    overlap.powf(-(1.0 - 0.5 * scatter))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(rate: f32) -> GrainParams {
        GrainParams { rate, size: 2.0, shape: 0.5, speed: 1.0, pos: 0.25, scatter: 0.0, jitter: 0.0, spread: 0.0, gain: 1.0, period: 0.0 }
    }

    fn source() -> Vec<f32> {
        (0..4800).map(|i| (i as f32 * 0.05).sin() + 0.3 * (i as f32 * 0.31).sin()).collect()
    }

    #[test]
    fn steady_grains_are_periodic_at_the_grain_rate() {
        // 441.7 Hz at 48 kHz is not a whole number of samples per period,
        // so this only holds with sub-sample grain onsets.
        let src = source();
        let rate = 441.7 / 48_000.0;
        let mut v = GrainVoice::new(7);
        let p = params(rate);
        let out: Vec<f32> = (0..9600).map(|_| v.tick(&src, &p).0).collect();
        let period = 1.0 / rate;
        // Compare each sample with the one a whole number of periods later,
        // interpolating the fractional delay.
        let mut err = 0.0f32;
        let mut ref_e = 0.0f32;
        for i in 2000..6000 {
            let d = i as f32 + 8.0 * period;
            let j = d as usize;
            let f = d - j as f32;
            let later = out[j] + (out[j + 1] - out[j]) * f;
            err += (out[i] - later).powi(2);
            ref_e += out[i] * out[i];
        }
        assert!(ref_e > 1.0);
        assert!(err / ref_e < 2e-3, "relative error {}", err / ref_e);
    }

    #[test]
    fn output_is_bounded_at_every_size() {
        let src = source();
        for size in [0.25f32, 1.0, 4.0, 16.0] {
            let mut v = GrainVoice::new(3);
            let mut p = params(200.0 / 48_000.0);
            p.size = size;
            p.gain = overlap_gain(size, p.shape, 0.0);
            let peak = (0..20_000).map(|_| v.tick(&src, &p).0.abs()).fold(0.0f32, f32::max);
            assert!(peak > 0.05 && peak < 2.0, "size {size}: peak {peak}");
        }
    }

    #[test]
    fn spread_and_scatter_decorrelate_the_channels() {
        let src = source();
        let mut v = GrainVoice::new(11);
        let mut p = params(40.0 / 48_000.0);
        p.spread = 1.0;
        p.scatter = 1.0;
        p.jitter = 1.0;
        let mut diff = 0.0f32;
        for _ in 0..48_000 {
            let (l, r) = v.tick(&src, &p);
            assert!(l.is_finite() && r.is_finite());
            diff += (l - r).abs();
        }
        assert!(diff > 10.0);
    }

    #[test]
    fn reverse_and_fast_playback_stay_in_bounds() {
        let src = source();
        for speed in [-3.7f64, 0.0, 9.3] {
            let mut v = GrainVoice::new(5);
            let mut p = params(20.0 / 48_000.0);
            p.size = 16.0;
            p.speed = speed;
            for _ in 0..48_000 {
                assert!(v.tick(&src, &p).0.is_finite());
            }
        }
    }
}
