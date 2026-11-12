//! Spectral engine — one instance per voice per slot.
//!
//! Plays a [`SpectralModel`]: four single-cycle wavetables read at one
//! shared phase and mixed by the model's envelopes at the current
//! position. Because every layer was built with the same partial phases,
//! mixing the tables is the same as interpolating their spectra.
//!
//! The transformers act like a filter cutoff — cheap enough to modulate:
//! Pure crossfades to tables holding only the strongest partials, Warp
//! bends the read phase (adds overtones, saw-like), FM modulates it.

use crate::analysis::{SpectralModel, LAYERS, MAX_HARMONICS, MIPS, TABLE_LEN};
use core::f32::consts::TAU;

/// Largest left/right detune at Spread = 1, in cents (each side).
const SPREAD_CENTS: f32 = 7.0;
/// Phase modulation depth at FM = 1, in cycles.
const FM_DEPTH: f32 = 0.6;

/// Control-rate inputs, constant across a control block.
#[derive(Clone, Copy)]
pub struct SpectralParams {
    /// Cycles per output sample.
    pub freq: f32,
    /// Target layer gains (envelope × tilt × level).
    pub gains: [f32; LAYERS],
    pub pure: f32,
    pub warp: f32,
    pub fm: f32,
    pub ratio: f32,
    pub spread: f32,
    /// Per-sample coefficient the layer gains glide with.
    pub glide: f32,
}

#[derive(Clone)]
pub struct SpectralVoice {
    phase_l: f32,
    phase_r: f32,
    mod_phase: f32,
    gains: [f32; LAYERS],
}

impl Default for SpectralVoice {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectralVoice {
    pub fn new() -> Self {
        SpectralVoice { phase_l: 0.0, phase_r: 0.0, mod_phase: 0.0, gains: [0.0; LAYERS] }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Jump the layer gains to their targets (note start: no glide-in).
    pub fn snap(&mut self, gains: [f32; LAYERS]) {
        self.gains = gains;
    }

    /// Render one stereo sample.
    #[inline]
    pub fn tick(&mut self, model: &SpectralModel, p: &SpectralParams) -> (f32, f32) {
        for (g, target) in self.gains.iter_mut().zip(p.gains.iter()) {
            *g += (target - *g) * p.glide;
        }
        // Warp and FM add partials above the table's own, so step down a
        // mip when either is in use.
        let extra = if p.warp > 0.001 || p.fm > 0.001 { 1 } else { 0 };
        let mip = (mip_for(p.freq) + extra).min(MIPS - 1);

        let detune = p.spread * SPREAD_CENTS * (1.0 / 1200.0) * core::f32::consts::LN_2;
        self.phase_l = wrap(self.phase_l + p.freq * (1.0 - detune));
        self.phase_r = wrap(self.phase_r + p.freq * (1.0 + detune));
        let fm = if p.fm > 0.001 {
            self.mod_phase = wrap(self.mod_phase + p.freq * p.ratio);
            p.fm * FM_DEPTH * (TAU * self.mod_phase).sin()
        } else {
            0.0
        };
        // Phase distortion: the first half-cycle is squeezed into `knee`.
        let knee = 0.5 - 0.47 * p.warp;
        let shape = |ph: f32| -> f32 {
            let w = if ph < knee { ph * (0.5 / knee) } else { 0.5 + (ph - knee) * (0.5 / (1.0 - knee)) };
            wrap(w + fm) * TABLE_LEN as f32
        };
        let (xl, xr) = (shape(self.phase_l), shape(self.phase_r));
        let (il, ir) = ((xl as usize).min(TABLE_LEN - 1), (xr as usize).min(TABLE_LEN - 1));
        let (fl, fr) = (xl - il as f32, xr - ir as f32);

        let (mut l, mut r) = (0.0f32, 0.0f32);
        for k in 0..LAYERS {
            let g = self.gains[k];
            if g <= 1e-7 {
                continue;
            }
            let full = model.table(k, 0, mip);
            let mut sl = full[il] + (full[il + 1] - full[il]) * fl;
            let mut sr = full[ir] + (full[ir + 1] - full[ir]) * fr;
            if p.pure > 0.001 {
                let pure = model.table(k, 1, mip);
                let pl = pure[il] + (pure[il + 1] - pure[il]) * fl;
                let pr = pure[ir] + (pure[ir + 1] - pure[ir]) * fr;
                sl += (pl - sl) * p.pure;
                sr += (pr - sr) * p.pure;
            }
            l += g * sl;
            r += g * sr;
        }
        (l, r)
    }
}

#[inline]
fn wrap(x: f32) -> f32 {
    x - x.floor()
}

/// Smallest mip whose highest partial stays below Nyquist at `freq`
/// (cycles per sample).
#[inline]
fn mip_for(freq: f32) -> usize {
    let allowed = (0.5 / freq.max(1e-6)) as usize;
    let mut mip = 0;
    while mip + 1 < MIPS && (MAX_HARMONICS >> mip) > allowed {
        mip += 1;
    }
    mip
}

/// Layer weights for Tilt: negative fades out the brightest layers first,
/// positive the darkest, until one layer is left at ±1.
pub fn tilt_weights(tilt: f32) -> [f32; LAYERS] {
    let mut w = [1.0; LAYERS];
    for (k, v) in w.iter_mut().enumerate() {
        // Steps from the layer that survives.
        let rank = if tilt < 0.0 { k } else { LAYERS - 1 - k } as f32;
        *v = (1.0 - tilt.abs() * rank).clamp(0.0, 1.0);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{analyze_blocking, Source};
    use std::sync::Arc;

    fn params(freq: f32, gains: [f32; LAYERS]) -> SpectralParams {
        SpectralParams { freq, gains, pure: 0.0, warp: 0.0, fm: 0.0, ratio: 2.0, spread: 0.0, glide: 1.0 }
    }

    #[test]
    fn mip_keeps_partials_below_nyquist() {
        for hz in [20.0f32, 110.0, 440.0, 1760.0, 7040.0] {
            let freq = hz / 48_000.0;
            let top = (MAX_HARMONICS >> mip_for(freq)) as f32 * hz;
            assert!(top <= 24_000.0 || mip_for(freq) == MIPS - 1, "{hz} Hz → top partial {top}");
        }
        assert_eq!(mip_for(20.0 / 48_000.0), 0);
    }

    #[test]
    fn tilt_removes_layers_in_order() {
        assert_eq!(tilt_weights(0.0), [1.0; LAYERS]);
        assert_eq!(tilt_weights(-1.0), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(tilt_weights(1.0), [0.0, 0.0, 0.0, 1.0]);
        let half = tilt_weights(-0.5);
        assert!(half[0] == 1.0 && half[3] <= half[2] && half[2] < half[1] && half[1] < 1.0);
    }

    #[test]
    fn plays_at_the_requested_frequency() {
        let model = analyze_blocking(Arc::new(Source::builtin(0)));
        let gains = model.gains_at(0.3, 0.0);
        let mut v = SpectralVoice::new();
        let hz = 311.13f32;
        let p = params(hz / 48_000.0, gains);
        let out: Vec<f32> = (0..48_000).map(|_| v.tick(&model, &p).0).collect();
        let period = 48_000.0 / hz;
        let (mut err, mut e) = (0.0f32, 0.0f32);
        for i in 1000..20_000 {
            let d = i as f32 + 10.0 * period;
            let j = d as usize;
            let later = out[j] + (out[j + 1] - out[j]) * (d - j as f32);
            err += (out[i] - later).powi(2);
            e += out[i] * out[i];
        }
        assert!(e > 1.0, "silent: {e}");
        assert!(err / e < 1e-3, "not periodic at {hz} Hz: {}", err / e);
    }

    #[test]
    fn transformers_change_the_sound_and_stay_finite() {
        let model = analyze_blocking(Arc::new(Source::builtin(0)));
        let gains = model.gains_at(0.3, 0.0);
        let render = |f: &dyn Fn(&mut SpectralParams)| -> Vec<f32> {
            let mut p = params(220.0 / 48_000.0, gains);
            f(&mut p);
            let mut v = SpectralVoice::new();
            (0..4800).map(|_| v.tick(&model, &p).0).collect()
        };
        let base = render(&|_| {});
        type Tweak<'a> = &'a dyn Fn(&mut SpectralParams);
        let cases: [(&str, Tweak); 3] =
            [("pure", &|p| p.pure = 1.0), ("warp", &|p| p.warp = 1.0), ("fm", &|p| p.fm = 1.0)];
        for (name, f) in cases {
            let out = render(f);
            assert!(out.iter().all(|v| v.is_finite()), "{name}");
            let diff: f32 = out.iter().zip(base.iter()).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff > 1.0, "{name} had no effect");
        }
    }

    #[test]
    fn spread_detunes_left_against_right() {
        let model = analyze_blocking(Arc::new(Source::builtin(0)));
        let mut p = params(220.0 / 48_000.0, model.gains_at(0.3, 0.0));
        p.spread = 1.0;
        let mut v = SpectralVoice::new();
        let diff: f32 = (0..48_000).map(|_| v.tick(&model, &p)).map(|(l, r)| (l - r).abs()).sum();
        assert!(diff > 10.0);
    }
}
