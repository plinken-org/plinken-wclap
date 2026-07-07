//! One-pole parameter smoother — zipper-free param changes for the
//! audio thread. Every plugin that maps a UI control to a per-sample
//! gain/amount should run the target through one of these instead of
//! jumping the raw value.

/// One-pole exponential smoother.
///
/// `next(target)` advances one sample toward `target` with the
/// configured time constant (the output reaches ~63% of a step after
/// `time_ms`). Allocation-free, denormal-safe via the snap threshold.
#[derive(Clone, Copy)]
pub struct Smoother {
    value: f32,
    coeff: f32,
}

impl Smoother {
    pub fn new(sample_rate: f32, time_ms: f32, initial: f32) -> Self {
        Self {
            value: initial,
            coeff: Self::coeff_for(sample_rate, time_ms),
        }
    }

    fn coeff_for(sample_rate: f32, time_ms: f32) -> f32 {
        if time_ms <= 0.0 || sample_rate <= 0.0 {
            return 0.0;
        }
        (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
    }

    pub fn set_time(&mut self, sample_rate: f32, time_ms: f32) {
        self.coeff = Self::coeff_for(sample_rate, time_ms);
    }

    /// Jump to `value` immediately (activate / state load — anything
    /// that shouldn't audibly glide).
    pub fn snap(&mut self, value: f32) {
        self.value = value;
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    /// Advance one sample toward `target` and return the new value.
    #[inline]
    pub fn next(&mut self, target: f32) -> f32 {
        self.value = target + (self.value - target) * self.coeff;
        // Snap when close enough — kills denormals and ends the tail.
        // Scale-aware: 1e-4 relative (≈ -80 dB) so it works for unit
        // gains and large-range params (Hz) alike.
        if (self.value - target).abs() < 1.0e-4 * target.abs().max(1.0) {
            self.value = target;
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaches_target_and_stays() {
        let sr = 48000.0;
        let mut s = Smoother::new(sr, 10.0, 0.0);
        let mut v = 0.0;
        for _ in 0..(sr * 0.1) as usize {
            v = s.next(1.0);
        }
        assert_eq!(v, 1.0);
        assert_eq!(s.next(1.0), 1.0);
    }

    #[test]
    fn approx_63_percent_after_time_constant() {
        let sr = 48000.0;
        let mut s = Smoother::new(sr, 10.0, 0.0);
        let mut v = 0.0;
        for _ in 0..480 {
            // exactly 10 ms
            v = s.next(1.0);
        }
        assert!((v - 0.632).abs() < 0.01, "v={v}");
    }

    #[test]
    fn monotonic_no_overshoot() {
        let mut s = Smoother::new(48000.0, 5.0, 0.0);
        let mut prev = 0.0;
        for _ in 0..1000 {
            let v = s.next(1.0);
            assert!(v >= prev && v <= 1.0);
            prev = v;
        }
    }

    #[test]
    fn snap_jumps_immediately() {
        let mut s = Smoother::new(48000.0, 100.0, 0.0);
        s.snap(0.5);
        assert_eq!(s.value(), 0.5);
    }
}
