//! Small math helpers shared across the DSP primitives.

/// Convert MIDI note to frequency
#[inline]
pub fn midi_to_freq(note: f32) -> f32 {
    440.0 * (2.0f32).powf((note - 69.0) / 12.0)
}

/// Fast tanh approximation for saturation
#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Soft clipping for gentle saturation
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x > 1.0 {
        1.0 - 1.0 / (1.0 + (x - 1.0) * 2.0)
    } else if x < -1.0 {
        -1.0 + 1.0 / (1.0 + (-x - 1.0) * 2.0)
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_to_freq_reference_pitches() {
        assert!((midi_to_freq(69.0) - 440.0).abs() < 1e-3);
        assert!((midi_to_freq(57.0) - 220.0).abs() < 1e-3);
        assert!((midi_to_freq(60.0) - 261.6256).abs() < 1e-2);
    }

    #[test]
    fn fast_tanh_tracks_tanh() {
        for i in -20..=20 {
            let x = i as f32 * 0.1;
            assert!((fast_tanh(x) - x.tanh()).abs() < 0.03, "x={x}");
        }
    }

    #[test]
    fn soft_clip_bounded_and_passthrough() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert_eq!(soft_clip(-0.5), -0.5);
        assert!(soft_clip(10.0) <= 1.5);
        assert!(soft_clip(-10.0) >= -1.5);
        assert!(soft_clip(10.0) > soft_clip(2.0));
    }
}
