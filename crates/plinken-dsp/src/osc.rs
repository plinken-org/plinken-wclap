//! Multi-mode oscillator with saw/pulse morphing, wavetable scanning,
//! FM input and hard sync.

use crate::wavetable::Wavetable;

/// Oscillator synthesis mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(u8)]
pub enum OscMode {
    /// Classic analog: saw/pulse morphing
    #[default]
    Analog = 0,
    /// Wavetable: scan through band-limited wavetable frames
    Wavetable = 1,
    /// Granular: granular cloud synthesis (future)
    Granular = 2,
    /// Physical: physical modeling / Karplus-Strong (future)
    Physical = 3,
    /// Sample: pitched sample playback. The Oscillator itself renders
    /// silence in this mode — the synth substitutes a sample-core voice
    /// (see Synome's render loop), this variant exists so param values
    /// map one enum end-to-end.
    Sample = 4,
}

impl OscMode {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Self::Analog,
            1 => Self::Wavetable,
            2 => Self::Granular,
            3 => Self::Physical,
            4 => Self::Sample,
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
    ///
    /// A wavetable oscillator needs its table, which this call cannot
    /// carry: use [`Oscillator::process_wavetable`]. Called in Wavetable
    /// mode it keeps the phase running (so sync still works) and is
    /// silent rather than passing another engine off as a wavetable.
    #[inline]
    pub fn process(&mut self, shape: f32, fm: f32) -> (f32, bool) {
        match self.mode {
            OscMode::Analog => self.process_analog(shape, fm),
            OscMode::Wavetable => (0.0, self.advance(fm).1),
            OscMode::Granular => self.process_analog(shape, fm), // TODO: granular cloud
            OscMode::Physical => self.process_analog(shape, fm), // TODO: Karplus-Strong
            OscMode::Sample => (0.0, false),                     // rendered by the synth's sample voice
        }
    }

    /// Scan `table` at `position` (0 = first frame, 1 = last).
    /// Returns (sample, phase_wrapped) for hard sync.
    #[inline]
    pub fn process_wavetable(&mut self, table: &Wavetable, position: f32, fm: f32) -> (f32, bool) {
        let (phase_inc, wrapped) = self.advance(fm);
        (table.read(self.phase, phase_inc, position), wrapped)
    }

    /// Step the phase by one sample. Returns (phase increment, wrapped).
    #[inline]
    fn advance(&mut self, fm: f32) -> (f32, bool) {
        let freq = (self.freq * (1.0 + fm)).clamp(0.1, 20000.0);
        let phase_inc = freq / self.sample_rate;

        self.phase += phase_inc;
        let wrapped = self.phase >= 1.0;
        if wrapped {
            self.phase -= 1.0;
        }
        (phase_inc, wrapped)
    }

    /// Classic analog: saw/pulse morphing (0=saw, 1=pulse)
    #[inline]
    fn process_analog(&mut self, shape: f32, fm: f32) -> (f32, bool) {
        let (_, wrapped) = self.advance(fm);

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
    use crate::wavetable::Spectrum;

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
    fn wavetable_mode_plays_the_table_at_the_set_pitch() {
        let sine = Spectrum::from_sines(|h| if h == 1 { 1.0 } else { 0.0 });
        let table = Wavetable::from_spectra(&[sine]);
        let mut osc = Oscillator::new(48000.0);
        osc.set_mode(OscMode::Wavetable);
        osc.set_freq(480.0); // 100 samples per cycle
        let mut wraps = 0;
        for n in 1..=4800 {
            let (s, wrapped) = osc.process_wavetable(&table, 0.0, 0.0);
            let want = (core::f32::consts::TAU * n as f32 / 100.0).sin();
            assert!((s - want).abs() < 2e-3, "n={n} s={s} want={want}");
            wraps += wrapped as u32;
        }
        assert!((47..=49).contains(&wraps), "wraps={wraps}");
    }

    #[test]
    fn wavetable_mode_without_a_table_is_silent_but_still_syncs() {
        let mut osc = Oscillator::new(1000.0);
        osc.set_mode(OscMode::Wavetable);
        osc.set_freq(100.0);
        let mut wraps = 0;
        for _ in 0..1000 {
            let (s, wrapped) = osc.process(0.5, 0.0);
            assert_eq!(s, 0.0);
            wraps += wrapped as u32;
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
