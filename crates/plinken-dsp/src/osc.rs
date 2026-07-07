//! Multi-mode oscillator with saw/pulse morphing, FM input and hard
//! sync.

/// Oscillator synthesis mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(u8)]
pub enum OscMode {
    /// Classic analog: saw/pulse morphing
    #[default]
    Analog = 0,
    /// Wavetable: scan through wavetable frames (future)
    Wavetable = 1,
    /// Granular: granular cloud synthesis (future)
    Granular = 2,
    /// Physical: physical modeling / Karplus-Strong (future)
    Physical = 3,
}

impl OscMode {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Self::Analog,
            1 => Self::Wavetable,
            2 => Self::Granular,
            3 => Self::Physical,
            _ => Self::Analog,
        }
    }
}

/// Multi-mode oscillator with pluggable synthesis modes.
#[derive(Clone, Copy, Default)]
pub struct Oscillator {
    phase: f32,
    freq: f32,
    sample_rate: f32,
    mode: OscMode,
}

impl Oscillator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            freq: 440.0,
            mode: OscMode::Analog,
            ..Default::default()
        }
    }

    pub fn set_freq(&mut self, freq: f32) {
        self.freq = freq.clamp(0.1, 20000.0);
    }

    pub fn set_mode(&mut self, mode: OscMode) {
        self.mode = mode;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Generate sample with shape morphing.
    /// Returns (sample, phase_wrapped) for hard sync.
    #[inline]
    pub fn process(&mut self, shape: f32, fm: f32) -> (f32, bool) {
        match self.mode {
            OscMode::Analog => self.process_analog(shape, fm),
            OscMode::Wavetable => self.process_analog(shape, fm), // TODO: wavetable scan
            OscMode::Granular => self.process_analog(shape, fm),  // TODO: granular cloud
            OscMode::Physical => self.process_analog(shape, fm),  // TODO: Karplus-Strong
        }
    }

    /// Classic analog: saw/pulse morphing (0=saw, 1=pulse)
    #[inline]
    fn process_analog(&mut self, shape: f32, fm: f32) -> (f32, bool) {
        let freq = (self.freq * (1.0 + fm)).clamp(0.1, 20000.0);
        let phase_inc = freq / self.sample_rate;

        self.phase += phase_inc;
        let wrapped = self.phase >= 1.0;
        if wrapped {
            self.phase -= 1.0;
        }

        // Morph between saw and pulse
        let saw = 2.0 * self.phase - 1.0;
        let pulse_width = 0.5;
        let pulse = if self.phase < pulse_width { 1.0 } else { -1.0 };

        let sample = saw * (1.0 - shape) + pulse * shape;
        (sample, wrapped)
    }

    /// Hard sync: reset phase
    #[inline]
    pub fn sync(&mut self) {
        self.phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saw_wraps_at_expected_rate() {
        let mut osc = Oscillator::new(1000.0);
        osc.set_freq(100.0); // wraps every 10 samples
        let mut wraps = 0;
        for _ in 0..1000 {
            let (s, wrapped) = osc.process(0.0, 0.0);
            assert!((-1.0..=1.0).contains(&s));
            if wrapped {
                wraps += 1;
            }
        }
        assert!((99..=101).contains(&wraps), "wraps={wraps}");
    }

    #[test]
    fn pulse_shape_is_binary() {
        let mut osc = Oscillator::new(48000.0);
        osc.set_freq(440.0);
        for _ in 0..1000 {
            let (s, _) = osc.process(1.0, 0.0);
            assert!(s == 1.0 || s == -1.0);
        }
    }
}
