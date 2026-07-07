//! White and pink noise generators.

/// Noise generator
#[derive(Clone, Copy)]
pub struct Noise {
    pink_state: [f32; 7],
    rng_state: u32,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            pink_state: [0.0; 7],
            rng_state: 54321,
        }
    }
}

impl Noise {
    fn fast_rand(&mut self) -> f32 {
        self.rng_state = self.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (self.rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    #[inline]
    pub fn white(&mut self) -> f32 {
        self.fast_rand()
    }

    #[inline]
    pub fn pink(&mut self) -> f32 {
        let white = self.white();
        self.pink_state[0] = 0.99886 * self.pink_state[0] + white * 0.0555179;
        self.pink_state[1] = 0.99332 * self.pink_state[1] + white * 0.0750759;
        self.pink_state[2] = 0.96900 * self.pink_state[2] + white * 0.1538520;
        self.pink_state[3] = 0.86650 * self.pink_state[3] + white * 0.3104856;
        self.pink_state[4] = 0.55000 * self.pink_state[4] + white * 0.5329522;
        self.pink_state[5] = -0.7616 * self.pink_state[5] - white * 0.0168980;

        let pink = self.pink_state[0]
            + self.pink_state[1]
            + self.pink_state[2]
            + self.pink_state[3]
            + self.pink_state[4]
            + self.pink_state[5]
            + self.pink_state[6]
            + white * 0.5362;
        self.pink_state[6] = white * 0.115926;

        pink * 0.11
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_noise_bounded_and_roughly_zero_mean() {
        let mut n = Noise::default();
        let mut sum = 0.0f64;
        for _ in 0..100_000 {
            let v = n.white();
            assert!((-1.0..=1.0).contains(&v));
            sum += v as f64;
        }
        assert!((sum / 100_000.0).abs() < 0.05);
    }

    #[test]
    fn pink_noise_stays_finite_and_bounded() {
        let mut n = Noise::default();
        for _ in 0..100_000 {
            let v = n.pink();
            assert!(v.is_finite());
            assert!(v.abs() < 2.0);
        }
    }
}
