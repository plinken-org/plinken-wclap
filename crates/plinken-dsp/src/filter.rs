//! Moog ladder filter.

use core::f32::consts::PI;

use crate::math::fast_tanh;

/// Moog Ladder Filter (4-pole resonant lowpass)
/// Based on Antti Huovilainen's non-linear model
#[derive(Clone, Copy, Default)]
pub struct MoogFilter {
    stage: [f32; 4],
    delay: f32,
    sample_rate: f32,
    mode: i32,  // 0=LP, 1=BP, 2=HP
    poles: i32, // 0=2pole, 1=4pole
}

impl MoogFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            ..Default::default()
        }
    }

    pub fn set_mode(&mut self, mode: i32) {
        self.mode = mode.clamp(0, 2);
    }

    pub fn set_poles(&mut self, poles: i32) {
        self.poles = poles.clamp(0, 1);
    }

    pub fn reset(&mut self) {
        self.stage = [0.0; 4];
        self.delay = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32, cutoff_freq: f32, resonance: f32) -> f32 {
        let f = (cutoff_freq / self.sample_rate).min(0.45);
        let g = (f * PI).tanh();
        let k = resonance * 4.0;

        let input_with_feedback = input - k * self.delay;
        let input_sat = fast_tanh(input_with_feedback * 0.5) * 2.0;

        // 4 cascaded 1-pole filters
        self.stage[0] += g * (input_sat - self.stage[0]);
        self.stage[1] += g * (self.stage[0] - self.stage[1]);
        self.stage[2] += g * (self.stage[1] - self.stage[2]);
        self.stage[3] += g * (self.stage[2] - self.stage[3]);

        self.delay = self.stage[3];

        // Output based on mode and pole count
        let lp2 = self.stage[1];
        let lp4 = self.stage[3];
        let bp = self.stage[1] - self.stage[3];
        let hp = input_sat - self.stage[1] - self.stage[3];

        match (self.mode, self.poles) {
            (0, 0) => lp2, // LP 2-pole
            (0, 1) => lp4, // LP 4-pole
            (1, _) => bp,  // BP
            (2, _) => hp,  // HP
            _ => lp4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lowpass at a low cutoff must attenuate a high-frequency input
    /// far more than the same input passed at a high cutoff.
    #[test]
    fn lowpass_attenuates_highs() {
        let sr = 48000.0;
        let rms = |cutoff: f32| {
            let mut f = MoogFilter::new(sr);
            f.set_poles(1);
            let mut acc = 0.0f32;
            let n = 4800;
            for i in 0..n {
                // 8 kHz sine
                let x = (i as f32 * 8000.0 / sr * 2.0 * PI).sin();
                let y = f.process(x, cutoff, 0.0);
                acc += y * y;
            }
            (acc / n as f32).sqrt()
        };
        let open = rms(18000.0);
        let closed = rms(200.0);
        assert!(closed < open * 0.1, "open={open} closed={closed}");
    }

    #[test]
    fn stable_under_full_resonance() {
        let mut f = MoogFilter::new(48000.0);
        f.set_poles(1);
        let mut out = 0.0;
        for i in 0..48000 {
            let x = if i % 100 < 50 { 0.8 } else { -0.8 };
            out = f.process(x, 2000.0, 1.0);
            assert!(out.is_finite());
        }
        assert!(out.abs() < 10.0);
    }
}
