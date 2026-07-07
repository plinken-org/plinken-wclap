//! Planar PCM sample buffer with linear-interpolated fractional reads.
//!
//! Ported from the private monorepo's `plugins/Sampler/src/sample_cache.rs`
//! (`SampleData`), minus every loading path — hosts hand us decoded PCM.

/// Decoded audio sample, planar layout.
#[derive(Debug, Clone, Default)]
pub struct SampleData {
    /// Sample rate of the PCM in Hz (playback resamples to the host rate).
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u32,
    /// Total frames (samples per channel).
    pub frame_count: usize,
    /// Left/mono channel data.
    pub left: Vec<f32>,
    /// Right channel data (empty if mono).
    pub right: Vec<f32>,
}

impl SampleData {
    /// Number of frames (samples per channel).
    pub fn len(&self) -> usize {
        self.frame_count
    }

    pub fn is_empty(&self) -> bool {
        self.frame_count == 0
    }

    /// Resident buffer size in bytes (for memory budgeting).
    pub fn memory_bytes(&self) -> usize {
        (self.left.len() + self.right.len()) * core::mem::size_of::<f32>()
    }

    /// Sample at a specific frame and channel. Out-of-range frames read as
    /// 0.0; a right-channel read on a mono sample falls back to left.
    #[inline]
    pub fn get_sample(&self, frame: usize, channel: usize) -> f32 {
        if frame >= self.frame_count {
            return 0.0;
        }
        match channel {
            0 => self.left.get(frame).copied().unwrap_or(0.0),
            1 if self.channels == 2 => self.right.get(frame).copied().unwrap_or(0.0),
            _ => self.left.get(frame).copied().unwrap_or(0.0),
        }
    }

    /// Sample value at a fractional position, linear interpolation.
    #[inline]
    pub fn get_sample_interpolated(&self, position: f64, channel: usize) -> f32 {
        if position < 0.0 || position >= self.frame_count as f64 {
            return 0.0;
        }
        let frame0 = position as usize;
        let frame1 = (frame0 + 1).min(self.frame_count.saturating_sub(1));
        let frac = (position - frame0 as f64) as f32;
        let s0 = self.get_sample(frame0, channel);
        let s1 = self.get_sample(frame1, channel);
        s0 + (s1 - s0) * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> SampleData {
        SampleData {
            sample_rate: 48000,
            channels: 1,
            frame_count: 4,
            left: vec![0.0, 1.0, 2.0, 3.0],
            right: vec![],
        }
    }

    #[test]
    fn interpolation_midpoints() {
        let s = ramp();
        assert_eq!(s.get_sample_interpolated(0.0, 0), 0.0);
        assert_eq!(s.get_sample_interpolated(0.5, 0), 0.5);
        assert_eq!(s.get_sample_interpolated(2.5, 0), 2.5);
        // Past the end reads silent, before the start too.
        assert_eq!(s.get_sample_interpolated(4.0, 0), 0.0);
        assert_eq!(s.get_sample_interpolated(-0.1, 0), 0.0);
    }

    #[test]
    fn mono_fallback_on_right_channel() {
        let s = ramp();
        assert_eq!(s.get_sample(1, 1), 1.0);
    }
}
