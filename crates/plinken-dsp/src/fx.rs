//! Effects: delay, reverb, and modulation FX (chorus / phaser /
//! flanger).

use core::f32::consts::PI;

const TWO_PI: f32 = 2.0 * PI;

/// Delay effect
pub struct Delay {
    buffer: Vec<f32>,
    write_pos: usize,
    sample_rate: f32,
}

impl Delay {
    pub fn new(sample_rate: f32, max_time: f32) -> Self {
        let size = (sample_rate * max_time) as usize + 1;
        Self {
            buffer: vec![0.0; size],
            write_pos: 0,
            sample_rate,
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32, time: f32, feedback: f32, mix: f32) -> f32 {
        let delay_samples = ((time * self.sample_rate) as usize).min(self.buffer.len() - 1);

        let read_pos = if self.write_pos >= delay_samples {
            self.write_pos - delay_samples
        } else {
            self.buffer.len() - (delay_samples - self.write_pos)
        };

        let delayed = self.buffer[read_pos];
        self.buffer[self.write_pos] = input + delayed * feedback;

        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        input * (1.0 - mix) + delayed * mix
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}

/// Simple reverb (comb + allpass)
pub struct Reverb {
    comb: [Vec<f32>; 4],
    comb_pos: [usize; 4],
    comb_filter: [f32; 4],
    allpass: [Vec<f32>; 2],
    allpass_pos: [usize; 2],
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let comb_times = [0.0297, 0.0371, 0.0411, 0.0437];
        let allpass_times = [0.005, 0.0017];

        Self {
            comb: core::array::from_fn(|i| vec![0.0; (comb_times[i] * sample_rate) as usize + 1]),
            comb_pos: [0; 4],
            comb_filter: [0.0; 4],
            allpass: core::array::from_fn(|i| {
                vec![0.0; (allpass_times[i] * sample_rate) as usize + 1]
            }),
            allpass_pos: [0; 2],
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32, decay: f32, damping: f32, mix: f32) -> f32 {
        let mut output = 0.0;

        for i in 0..4 {
            let delayed = self.comb[i][self.comb_pos[i]];
            self.comb_filter[i] = delayed * (1.0 - damping) + self.comb_filter[i] * damping;
            self.comb[i][self.comb_pos[i]] = input + self.comb_filter[i] * decay;
            self.comb_pos[i] = (self.comb_pos[i] + 1) % self.comb[i].len();
            output += delayed;
        }
        output *= 0.25;

        for i in 0..2 {
            let delayed = self.allpass[i][self.allpass_pos[i]];
            let temp = output + delayed * 0.5;
            self.allpass[i][self.allpass_pos[i]] = output;
            output = delayed - temp * 0.5;
            self.allpass_pos[i] = (self.allpass_pos[i] + 1) % self.allpass[i].len();
        }

        input * (1.0 - mix) + output * mix
    }

    pub fn clear(&mut self) {
        for c in &mut self.comb {
            c.fill(0.0);
        }
        for a in &mut self.allpass {
            a.fill(0.0);
        }
        self.comb_filter = [0.0; 4];
    }
}

/// Chorus/Flanger/Phaser effect
///
/// `fx_type`: 0 = bypass, 1 = phaser, 2 = flanger, 3 = chorus.
pub struct ModulationFx {
    buffer: Vec<f32>,
    write_pos: usize,
    lfo_phase: f32,
    sample_rate: f32,
}

impl ModulationFx {
    pub fn new(sample_rate: f32) -> Self {
        let size = (sample_rate * 0.05) as usize + 1;
        Self {
            buffer: vec![0.0; size],
            write_pos: 0,
            lfo_phase: 0.0,
            sample_rate,
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32, fx_type: i32, rate: f32, depth: f32, mix: f32) -> f32 {
        if fx_type == 0 {
            return input;
        }

        self.lfo_phase += rate / self.sample_rate;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }
        let lfo = (self.lfo_phase * TWO_PI).sin();

        let (base_delay, mod_amount) = match fx_type {
            1 => (0.005, 0.003 * depth), // Phaser
            2 => (0.001, 0.005 * depth), // Flanger
            3 => (0.015, 0.010 * depth), // Chorus
            _ => (0.010, 0.005 * depth),
        };

        let delay_time = base_delay + mod_amount * (lfo * 0.5 + 0.5);
        let delay_samples = ((delay_time * self.sample_rate) as usize).min(self.buffer.len() - 1);

        let read_pos = if self.write_pos >= delay_samples {
            self.write_pos - delay_samples
        } else {
            self.buffer.len() - (delay_samples - self.write_pos)
        };

        let delayed = self.buffer[read_pos];
        let feedback = if fx_type == 2 { 0.7 * depth } else { 0.0 };
        self.buffer[self.write_pos] = input + delayed * feedback;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        input * (1.0 - mix) + delayed * mix
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_echoes_impulse_at_delay_time() {
        let sr = 1000.0;
        let mut d = Delay::new(sr, 1.0);
        // Impulse, full wet so output is the delayed signal only.
        let first = d.process(1.0, 0.1, 0.0, 1.0);
        assert_eq!(first, 0.0);
        let mut echo_at = None;
        for i in 1..200 {
            let out = d.process(0.0, 0.1, 0.0, 1.0);
            if out > 0.5 {
                echo_at = Some(i);
                break;
            }
        }
        assert_eq!(echo_at, Some(100)); // 0.1 s at 1 kHz
    }

    #[test]
    fn reverb_produces_tail_and_decays() {
        let mut r = Reverb::new(48000.0);
        r.process(1.0, 0.7, 0.2, 1.0);
        let mut energy_early = 0.0f32;
        let mut energy_late = 0.0f32;
        for i in 0..48000 {
            let out = r.process(0.0, 0.7, 0.2, 1.0);
            assert!(out.is_finite());
            if i < 4800 {
                energy_early += out * out;
            } else if i >= 43200 {
                energy_late += out * out;
            }
        }
        assert!(energy_early > 0.0);
        assert!(energy_late < energy_early);
    }

    #[test]
    fn modfx_bypass_is_identity() {
        let mut m = ModulationFx::new(48000.0);
        for i in 0..100 {
            let x = (i as f32 * 0.01).sin();
            assert_eq!(m.process(x, 0, 1.0, 0.5, 0.5), x);
        }
    }

    #[test]
    fn modfx_chorus_stays_finite() {
        let mut m = ModulationFx::new(48000.0);
        for i in 0..48000 {
            let x = (i as f32 * 0.05).sin();
            let out = m.process(x, 3, 0.8, 1.0, 0.5);
            assert!(out.is_finite());
            assert!(out.abs() < 4.0);
        }
    }
}
