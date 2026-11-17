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
        // Coefficient of a one-pole stage whose corner is the cut-off:
        // 1 − e^(−2πf). (It was tanh(πf), about half that for low f, which
        // put the corner an octave below the frequency asked for.)
        let g = 1.0 - (-2.0 * PI * f).exp();
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
        // 12 dB/oct high-pass from the first two stages: in − 2·s1 + s2,
        // which is zero at DC and the input itself far above the cut-off.
        // (It was `in − s2 − s4`, which passes the lows inverted.)
        let hp = input_sat - 2.0 * self.stage[0] + self.stage[1];

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

    fn sine_rms(mode: i32, tone_hz: f32, cutoff: f32) -> f32 {
        let sr = 48000.0;
        let mut f = MoogFilter::new(sr);
        f.set_mode(mode);
        f.set_poles(1);
        let n = 9600;
        let mut acc = 0.0f32;
        for i in 0..n {
            let x = 0.1 * (i as f32 * tone_hz / sr * 2.0 * PI).sin();
            let y = f.process(x, cutoff, 0.0);
            if i >= n / 2 {
                acc += y * y;
            }
        }
        (acc / (n / 2) as f32).sqrt()
    }

    /// Four one-pole stages each pass 1/√2 at their corner, so a tone at
    /// the cut-off comes through the 4-pole low-pass at 1/4 (−12 dB).
    #[test]
    fn the_corner_is_at_the_cutoff_asked_for() {
        let input = 0.1 / 2f32.sqrt();
        for cutoff in [100.0f32, 1000.0, 5000.0] {
            let at_corner = sine_rms(0, cutoff, cutoff) / input;
            assert!((0.22..=0.29).contains(&at_corner), "{cutoff} Hz: gain {at_corner} at the corner");
            // An octave below the corner is nearly through; an octave above is well down.
            assert!(sine_rms(0, cutoff / 2.0, cutoff) / input > 0.55);
            assert!(sine_rms(0, cutoff * 2.0, cutoff) / input < 0.06);
        }
    }

    #[test]
    fn highpass_removes_lows_and_passes_highs() {
        let input = 0.1 / 2f32.sqrt();
        let low = sine_rms(2, 50.0, 2000.0);
        let high = sine_rms(2, 12000.0, 500.0);
        assert!(low < 0.01 * input, "50 Hz through a 2 kHz high-pass: {low}");
        assert!((0.7..=1.2).contains(&(high / input)), "12 kHz through a 500 Hz high-pass: {high}");
    }

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
