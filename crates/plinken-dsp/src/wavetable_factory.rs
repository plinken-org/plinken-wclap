//! The factory wavetables, computed rather than shipped: every table is a
//! short recipe (a harmonic series or one cycle of a formula) evaluated
//! at build time. Nothing here is sampled from anyone else's content, so
//! the set carries no third-party licence, and it costs no bundle size.
//!
//! **The order of [`FACTORY_TABLE_NAMES`] is saved in presets** (a table
//! is selected by index). Append new tables; never reorder or remove.

use crate::wavetable::{Spectrum, Wavetable, WavetableBank, MAX_HARMONICS};
use core::f64::consts::{PI, TAU};

/// Frames per factory table. Adjacent frames are crossfaded on playback,
/// so 32 is smooth for these recipes and keeps the bank near 5 MB.
pub const FACTORY_FRAMES: usize = 32;

/// Display names, in bank order. The Synome faceplate mirrors this list.
pub const FACTORY_TABLE_NAMES: [&str; 10] = [
    "Basic", "PWM", "Harm", "Reso", "Vowel", "Sync", "FM", "Fold", "Organ", "Glass",
];

/// Build the whole factory bank. Allocates and runs a few thousand FFTs:
/// call it at plugin setup, never from the audio path.
pub fn factory_bank() -> WavetableBank {
    let recipes: [fn(f64) -> Spectrum; FACTORY_TABLE_NAMES.len()] =
        [basic, pwm, harmonics, resonant, vowel, sync, fm, fold, organ, glass];
    let tables = recipes
        .iter()
        .map(|recipe| {
            let frames: Vec<Spectrum> = (0..FACTORY_FRAMES)
                .map(|f| recipe(f as f64 / (FACTORY_FRAMES - 1) as f64))
                .collect();
            Wavetable::from_spectra(&frames)
        })
        .collect();
    WavetableBank::new(tables)
}

/// Position `u` in `[0, 1]` across `keys`: the two neighbours and the
/// blend between them.
fn segment(u: f64, keys: usize) -> (usize, usize, f64) {
    let x = u.clamp(0.0, 1.0) * (keys - 1) as f64;
    let a = (x as usize).min(keys - 1);
    (a, (a + 1).min(keys - 1), x - a as f64)
}

fn sine_wave() -> Spectrum {
    Spectrum::from_sines(|h| if h == 1 { 1.0 } else { 0.0 })
}

fn triangle_wave() -> Spectrum {
    Spectrum::from_sines(|h| {
        if h % 2 == 0 {
            return 0.0;
        }
        let sign = if (h / 2) % 2 == 0 { 1.0 } else { -1.0 };
        sign * 8.0 / (PI * PI * (h * h) as f64)
    })
}

/// Rising ramp, the same polarity as the analog oscillator's saw.
fn saw_wave() -> Spectrum {
    Spectrum::from_sines(|h| -2.0 / (PI * h as f64))
}

fn square_wave() -> Spectrum {
    Spectrum::from_sines(|h| if h % 2 == 1 { 4.0 / (PI * h as f64) } else { 0.0 })
}

/// Sine → triangle → saw → square.
fn basic(u: f64) -> Spectrum {
    let keys = [sine_wave(), triangle_wave(), saw_wave(), square_wave()];
    let (a, b, t) = segment(u, keys.len());
    Spectrum::lerp(&keys[a], &keys[b], t as f32)
}

/// Pulse narrowing from a square (50 %) to a 3 % spike.
fn pwm(u: f64) -> Spectrum {
    let duty = 0.5 - 0.47 * u;
    let mut s = Spectrum::new();
    for h in 1..=MAX_HARMONICS {
        let k = 2.0 / (PI * h as f64);
        let a = TAU * h as f64 * duty;
        s.cos[h] = (k * a.sin()) as f32;
        s.sin[h] = (k * (1.0 - a.cos())) as f32;
    }
    s
}

/// A saw opening up: a 24 dB/oct cut-off sweeping from the fundamental
/// to the 256th harmonic.
fn harmonics(u: f64) -> Spectrum {
    let cutoff = 2f64.powf(8.0 * u);
    Spectrum::from_sines(|h| {
        let r = h as f64 / cutoff;
        let gain = if r <= 1.0 { 1.0 } else { r.powi(-4) };
        -2.0 / (PI * h as f64) * gain
    })
}

/// The same sweep through a resonant 2-pole low-pass (Q = 6), for a
/// filter sweep that needs no filter.
fn resonant(u: f64) -> Spectrum {
    let cutoff = 1.5 * 64f64.powf(u);
    let q = 6.0;
    Spectrum::from_sines(|h| {
        let r = h as f64 / cutoff;
        let gain = 1.0 / ((1.0 - r * r).powi(2) + (r / q).powi(2)).sqrt();
        -2.0 / (PI * h as f64) * gain
    })
}

/// A – E – I – O – U. The formant frequencies glide between vowels (a
/// blend of two vowel spectra would give two vowels at once, not the one
/// in between). Voiced at 110 Hz, so the vowel is truest around A2.
fn vowel(u: f64) -> Spectrum {
    const FORMANTS: [[f64; 3]; 5] = [
        [730.0, 1090.0, 2440.0],
        [530.0, 1840.0, 2480.0],
        [270.0, 2290.0, 3010.0],
        [570.0, 840.0, 2410.0],
        [300.0, 870.0, 2240.0],
    ];
    const WIDTH: [f64; 3] = [70.0, 100.0, 130.0];
    const LEVEL: [f64; 3] = [1.0, 0.5, 0.25];
    let (a, b, t) = segment(u, FORMANTS.len());
    Spectrum::from_sines(|h| {
        let hz = h as f64 * 110.0;
        let mut gain = 0.02;
        for i in 0..3 {
            let centre = FORMANTS[a][i] + (FORMANTS[b][i] - FORMANTS[a][i]) * t;
            gain += LEVEL[i] / (1.0 + ((hz - centre) / WIDTH[i]).powi(2));
        }
        gain / h as f64
    })
}

/// Hard-synced saw: the slave runs from 1× to 8× the master.
fn sync(u: f64) -> Spectrum {
    let ratio = 1.0 + 7.0 * u;
    Spectrum::from_fn(|t| 2.0 * (t * ratio).fract() - 1.0)
}

/// Two-operator FM, modulator an octave above the carrier, index 0 → 6.
fn fm(u: f64) -> Spectrum {
    let index = 6.0 * u;
    Spectrum::from_fn(|t| (TAU * t + index * (2.0 * TAU * t).sin()).sin())
}

/// A sine driven into a sine wavefolder, 1 → 8 folds.
fn fold(u: f64) -> Spectrum {
    let drive = 1.0 + 7.0 * u;
    Spectrum::from_fn(|t| (0.5 * PI * drive * (TAU * t).sin()).sin())
}

/// Drawbar registrations from a lone 8' flute to all stops out.
fn organ(u: f64) -> Spectrum {
    // 8', 4', 2 2/3', 2', 1 3/5', 1 1/3', 1' as harmonics of the 8'.
    const PIPES: [usize; 7] = [1, 2, 3, 4, 5, 6, 8];
    const STOPS: [[f64; 7]; 5] = [
        [8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [8.0, 6.0, 0.0, 4.0, 0.0, 0.0, 0.0],
        [8.0, 8.0, 8.0, 0.0, 0.0, 0.0, 0.0],
        [8.0, 8.0, 8.0, 8.0, 4.0, 4.0, 4.0],
        [8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0],
    ];
    // A drawbar step is about 3 dB; fully in (0) is silent.
    let level = |stop: f64| if stop <= 0.0 { 0.0 } else { 10f64.powf((stop - 8.0) * 3.0 / 20.0) };
    let (a, b, t) = segment(u, STOPS.len());
    let mut s = Spectrum::new();
    for (i, &h) in PIPES.iter().enumerate() {
        s.sin[h] = (level(STOPS[a][i]) + (level(STOPS[b][i]) - level(STOPS[a][i])) * t) as f32;
    }
    s
}

/// A sparse, bell-like set of partials whose weight slides from the
/// lowest to the highest.
fn glass(u: f64) -> Spectrum {
    const PARTIALS: [usize; 8] = [1, 3, 4, 7, 11, 18, 29, 47];
    let centre = u * (PARTIALS.len() - 1) as f64;
    let mut s = Spectrum::new();
    for (i, &h) in PARTIALS.iter().enumerate() {
        s.sin[h] = (-0.6 * (i as f64 - centre).abs()).exp() as f32;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycle(table: &Wavetable, position: f32) -> Vec<f32> {
        let n = 2048;
        (0..n).map(|j| table.read(j as f32 / n as f32, 1e-5, position)).collect()
    }

    #[test]
    fn every_factory_frame_is_audible_bounded_and_finite() {
        let bank = factory_bank();
        assert_eq!(bank.len(), FACTORY_TABLE_NAMES.len());
        for (t, name) in FACTORY_TABLE_NAMES.iter().enumerate() {
            let table = bank.get(t).unwrap();
            assert_eq!(table.frames(), FACTORY_FRAMES);
            for f in 0..FACTORY_FRAMES {
                let buf = cycle(table, f as f32 / (FACTORY_FRAMES - 1) as f32);
                assert!(buf.iter().all(|x| x.is_finite()), "{name} frame {f} not finite");
                let peak = buf.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                assert!((0.9..=1.05).contains(&peak), "{name} frame {f} peak={peak}");
            }
        }
    }

    #[test]
    fn scanning_a_table_changes_the_sound() {
        let bank = factory_bank();
        for (t, name) in FACTORY_TABLE_NAMES.iter().enumerate() {
            let table = bank.get(t).unwrap();
            let (start, end) = (cycle(table, 0.0), cycle(table, 1.0));
            let diff = start.iter().zip(&end).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(diff > 0.1, "{name} is the same at both ends");
        }
    }

    #[test]
    fn basic_table_starts_as_a_sine_and_ends_as_a_square() {
        let bank = factory_bank();
        let table = bank.get(0).unwrap();
        let start = cycle(table, 0.0);
        for (j, x) in start.iter().enumerate() {
            let want = (TAU * j as f64 / start.len() as f64).sin() as f32;
            assert!((x - want).abs() < 1e-3);
        }
        let end = cycle(table, 1.0);
        // Away from the edges a square sits flat near its (normalised) level.
        assert!(end[512] > 0.75 && end[1536] < -0.75);
    }

    #[test]
    fn tables_differ_from_each_other() {
        let bank = factory_bank();
        let mids: Vec<Vec<f32>> = (0..bank.len()).map(|t| cycle(bank.get(t).unwrap(), 0.5)).collect();
        for a in 0..mids.len() {
            for b in a + 1..mids.len() {
                let diff = mids[a].iter().zip(&mids[b]).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                assert!(diff > 0.05, "{} and {} sound the same", FACTORY_TABLE_NAMES[a], FACTORY_TABLE_NAMES[b]);
            }
        }
    }
}
