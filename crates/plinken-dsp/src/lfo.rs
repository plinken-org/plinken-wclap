//! LFO with multiple waveforms and onset delay.

use core::f32::consts::PI;

const TWO_PI: f32 = 2.0 * PI;

/// LFO with multiple waveforms
///
/// Shapes: 0 = sine, 1 = triangle, 2 = saw, 3 = square,
/// 4 = sample & hold.
#[derive(Clone, Copy)]
pub struct Lfo {
    phase: f32,
    rate: f32,
    sample_rate: f32,
    shape: i32,
    last_sh: f32,
    delay_counter: f32,
    delay_samples: f32,
    rng_state: u32,
}

impl Default for Lfo {
    fn default() -> Self {
        Self {
            phase: 0.0,
            rate: 1.0,
            sample_rate: 48000.0,
            shape: 0,
            last_sh: 0.0,
            delay_counter: 0.0,
            delay_samples: 0.0,
            rng_state: 12345,
        }
    }
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            ..Default::default()
        }
    }

    pub fn set_rate(&mut self, hz: f32) {
        self.rate = hz;
    }

    pub fn set_shape(&mut self, shape: i32) {
        self.shape = shape;
    }

    pub fn set_delay(&mut self, delay_sec: f32) {
        self.delay_samples = delay_sec * self.sample_rate;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.delay_counter = 0.0;
    }

    // Simple fast random
    fn fast_rand(&mut self) -> f32 {
        self.rng_state = self.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (self.rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        // Handle delay
        if self.delay_counter < self.delay_samples {
            self.delay_counter += 1.0;
            return 0.0;
        }

        let phase_inc = self.rate / self.sample_rate;
        self.phase += phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            // Sample & Hold: sample new random value at phase wrap
            if self.shape == 4 {
                self.last_sh = self.fast_rand();
            }
        }

        match self.shape {
            0 => (self.phase * TWO_PI).sin(), // Sine
            1 => {
                // Triangle
                if self.phase < 0.5 {
                    4.0 * self.phase - 1.0
                } else {
                    3.0 - 4.0 * self.phase
                }
            }
            2 => 2.0 * self.phase - 1.0, // Saw
            3 => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            } // Square
            4 => self.last_sh, // Sample & Hold
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_lfo_bounded_and_periodic() {
        let mut lfo = Lfo::new(1000.0);
        lfo.set_rate(10.0); // 100 samples per cycle
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for _ in 0..1000 {
            let v = lfo.process();
            min = min.min(v);
            max = max.max(v);
        }
        assert!(max > 0.99 && max <= 1.001, "max={max}");
        assert!(min < -0.99 && min >= -1.001, "min={min}");
    }

    #[test]
    fn delay_holds_output_at_zero() {
        let mut lfo = Lfo::new(1000.0);
        lfo.set_rate(100.0);
        lfo.set_delay(0.1); // 100 samples of delay
        for _ in 0..100 {
            assert_eq!(lfo.process(), 0.0);
        }
        let mut nonzero = false;
        for _ in 0..100 {
            if lfo.process() != 0.0 {
                nonzero = true;
            }
        }
        assert!(nonzero);
    }
}
