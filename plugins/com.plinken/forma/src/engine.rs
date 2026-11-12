//! FORMA synth engine: voices, the two slots, the mix and the output.
//!
//! Each voice runs both slots. A slot is a Grain or a Spectral engine on
//! that slot's source, placed in the stereo field in mid/side terms. The
//! two slots are crossfaded (or ring-modulated) per voice, then shaped by
//! the amp envelope and the voice filter.

use crate::analysis::{Source, SpectralModel, LAYERS};
use crate::grain::{overlap_gain, GrainParams, GrainVoice};
use crate::params::{self as P, PARAM_COUNT, PARAM_DEFS};
use crate::spectral::{tilt_weights, SpectralParams, SpectralVoice};
use core::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2};
use plinken_dsp::envelope::Envelope;
use plinken_dsp::math::{midi_to_freq, soft_clip};
use std::sync::Arc;

pub const VOICES: usize = 12;
pub const SLOTS: usize = 2;
/// Control values are refreshed every this many samples, counted across
/// blocks so the result does not depend on the host's block size.
const CONTROL_PERIOD: usize = 32;
/// Reference pitch for grain rate with key tracking off (C3).
const FREE_RATE_HZ: f32 = 130.812_78;
/// Make-up gain for the ring product of two signals well below full scale.
const RING_GAIN: f32 = 4.0;
/// Headroom for a chord before the output soft clip.
const VOICE_GAIN: f32 = 0.5;

#[derive(Clone, Copy, PartialEq)]
enum EngineKind {
    Grain,
    Spectral,
}

/// What a slot plays: its source and, once analysed, the spectral model.
#[derive(Clone, Default)]
pub struct SlotContent {
    pub source: Option<Arc<Source>>,
    pub model: Option<Arc<SpectralModel>>,
    /// Detected root of the source (falls back to C3 for unpitched audio).
    pub root_hz: f32,
}

#[derive(Clone, Copy)]
struct Placement {
    /// Mid → left / right gains (balance).
    mid_l: f32,
    mid_r: f32,
    /// Side gain (width).
    side: f32,
}

#[derive(Clone, Copy)]
enum SlotCtl {
    Off,
    Grain(GrainParams),
    Spectral(SpectralParams),
}

#[derive(Clone)]
struct SlotVoice {
    grain: GrainVoice,
    spectral: SpectralVoice,
    /// Scan offset accumulated since note-on, in source lengths.
    scan: f64,
    ctl: SlotCtl,
    /// Engine the control values were last built for.
    kind: Option<EngineKind>,
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// TPT state-variable filter; returns (low, band, high).
    #[inline]
    fn tick(&mut self, x: f32, g: f32, k: f32, a1: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = a1 * (self.ic1 + g * v3);
        let v2 = self.ic2 + g * v1;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, v1, x - k * v1 - v2)
    }
}

/// wasm has no flush-to-zero mode; keep recursive state out of denormals.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

#[derive(Clone)]
struct Voice {
    active: bool,
    held: bool,
    note: u8,
    vel: f32,
    /// Order of note-on, for stealing the oldest voice.
    serial: u64,
    env: Envelope,
    slots: [SlotVoice; SLOTS],
    filter: [Svf; 2],
}

pub struct Engine {
    sr: f32,
    values: [f32; PARAM_COUNT],
    content: [SlotContent; SLOTS],
    voices: Vec<Voice>,
    serial: u64,
    /// Samples until the next control refresh.
    control_in: usize,
    // Control-rate values shared by all voices.
    place: [Placement; SLOTS],
    mix: [f32; 3],
    filt_on: bool,
    filt_g: f32,
    filt_k: f32,
    filt_a1: f32,
    filt_mode: u8,
    volume: f32,
    volume_target: f32,
    /// Playhead (0..1) per slot of the newest sounding voice, for the UI.
    playhead: [f32; SLOTS],
}

impl Engine {
    pub fn new(sample_rate: f32) -> Engine {
        let sr = sample_rate.max(8000.0);
        let voice = |i: usize| Voice {
            active: false,
            held: false,
            note: 0,
            vel: 0.0,
            serial: 0,
            env: Envelope::new(sr),
            slots: core::array::from_fn(|s| SlotVoice {
                grain: GrainVoice::new(0x9e37_79b9u32.wrapping_mul((i * SLOTS + s + 1) as u32)),
                spectral: SpectralVoice::new(),
                scan: 0.0,
                ctl: SlotCtl::Off,
                kind: None,
            }),
            filter: [Svf::default(); 2],
        };
        let mut e = Engine {
            sr,
            values: core::array::from_fn(|i| PARAM_DEFS[i].default as f32),
            content: Default::default(),
            voices: (0..VOICES).map(voice).collect(),
            serial: 0,
            control_in: 0,
            place: [Placement { mid_l: 1.0, mid_r: 1.0, side: 1.0 }; SLOTS],
            mix: [0.0; 3],
            filt_on: false,
            filt_g: 0.0,
            filt_k: 2.0,
            filt_a1: 0.0,
            filt_mode: 0,
            volume: 0.0,
            volume_target: 0.0,
            playhead: [0.0; SLOTS],
        };
        e.volume_target = e.values[P::VOLUME as usize];
        e.volume = e.volume_target;
        e
    }

    pub fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
        }
    }

    pub fn set_content(&mut self, slot: usize, content: SlotContent) {
        if slot < SLOTS {
            self.content[slot] = content;
            // Running grains hold positions into the old source.
            for v in self.voices.iter_mut() {
                v.slots[slot].grain.reset();
                v.slots[slot].kind = None;
            }
        }
    }

    /// Attach analysis results to a slot without disturbing sounding grains.
    pub fn set_analysis(&mut self, slot: usize, root_hz: f32, model: Option<Arc<SpectralModel>>) {
        if slot < SLOTS {
            self.content[slot].root_hz = root_hz;
            if model.is_some() {
                self.content[slot].model = model;
            }
        }
    }

    pub fn playhead(&self, slot: usize) -> f32 {
        self.playhead[slot]
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.serial += 1;
        let serial = self.serial;
        // Retrigger the same key, else a free voice, else the oldest
        // released one, else the oldest of all.
        let pick = self
            .voices
            .iter()
            .position(|v| v.active && v.note == note)
            .or_else(|| self.voices.iter().position(|v| !v.active))
            .or_else(|| oldest(&self.voices, |v| !v.held))
            .or_else(|| oldest(&self.voices, |_| true))
            .unwrap_or(0);
        let (a, d, s, r) = (
            self.values[P::ENV_A as usize],
            self.values[P::ENV_D as usize],
            self.values[P::ENV_S as usize],
            self.values[P::ENV_R as usize],
        );
        let sens = self.values[P::VEL as usize];
        let v = &mut self.voices[pick];
        v.active = true;
        v.held = true;
        v.note = note;
        v.vel = 1.0 - sens + sens * velocity.clamp(0.0, 1.0);
        v.serial = serial;
        v.env = Envelope::new(self.sr);
        v.env.set_adsr(a, d, s, r);
        v.env.trigger();
        v.filter = [Svf::default(); 2];
        for s in v.slots.iter_mut() {
            s.grain.reset();
            s.spectral.reset();
            s.scan = 0.0;
            s.ctl = SlotCtl::Off;
            s.kind = None;
        }
        // Build this voice's control values now rather than at the next
        // control tick, so the note starts on its first sample.
        self.refresh_voice(pick, true);
    }

    pub fn note_off(&mut self, note: u8) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.note == note) {
            v.held = false;
            v.env.release();
        }
    }

    pub fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.held = false;
            v.env.release();
        }
    }

    /// Hard stop: every voice silent at once.
    pub fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.active = false;
            v.held = false;
            v.env.reset();
        }
    }

    fn slot_value(&self, slot: usize, offset: u32) -> f32 {
        self.values[P::slot_index(slot, offset)]
    }

    /// Recompute the values shared by all voices.
    fn refresh_globals(&mut self) {
        for slot in 0..SLOTS {
            let theta = (self.slot_value(slot, P::BALANCE).clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
            self.place[slot] = Placement {
                mid_l: SQRT_2 * theta.cos(),
                mid_r: SQRT_2 * theta.sin(),
                side: self.slot_value(slot, P::WIDTH),
            };
        }
        let fade = self.values[P::FADE as usize].clamp(0.0, 1.0);
        self.mix = if self.values[P::MIX_MODE as usize] >= 0.5 {
            // A alone → ring product → B alone.
            [(1.0 - 2.0 * fade).max(0.0), (2.0 * fade - 1.0).max(0.0), (1.0 - (2.0 * fade - 1.0).abs()) * RING_GAIN]
        } else {
            [(fade * FRAC_PI_2).cos(), (fade * FRAC_PI_2).sin(), 0.0]
        };
        let cutoff = self.values[P::F_CUTOFF as usize].clamp(20.0, 0.45 * self.sr);
        self.filt_mode = self.values[P::F_MODE as usize].round() as u8;
        // A fully open low-pass is the default patch: skip the filter then.
        self.filt_on = !(self.filt_mode == 0 && self.values[P::F_CUTOFF as usize] >= 19_999.0);
        self.filt_g = (PI * cutoff / self.sr).tan();
        self.filt_k = 2.0 - 1.96 * self.values[P::F_RES as usize].clamp(0.0, 1.0);
        self.filt_a1 = 1.0 / (1.0 + self.filt_g * (self.filt_g + self.filt_k));
        self.volume_target = self.values[P::VOLUME as usize];
    }

    /// Recompute one voice's per-slot control values. `start` is true on
    /// the note's first refresh (no scan advance, no gain glide).
    fn refresh_voice(&mut self, index: usize, start: bool) {
        let note = self.voices[index].note as f32;
        let (a, d, s, r) = (
            self.values[P::ENV_A as usize],
            self.values[P::ENV_D as usize],
            self.values[P::ENV_S as usize],
            self.values[P::ENV_R as usize],
        );
        self.voices[index].env.set_adsr(a, d, s, r);
        for slot in 0..SLOTS {
            let val = |off: u32| self.values[P::slot_index(slot, off)];
            let content = &self.content[slot];
            let Some(src) = content.source.as_ref().filter(|s| s.data.len() >= 4) else {
                self.voices[index].slots[slot].ctl = SlotCtl::Off;
                continue;
            };
            let kind = if val(P::ENGINE) >= 0.5 { EngineKind::Spectral } else { EngineKind::Grain };
            let level = val(P::LEVEL);
            let note_hz = midi_to_freq(note + val(P::TUNE));
            let scan_per_sample = val(P::SCAN) as f64 / (src.seconds() as f64 * self.sr as f64);

            let sv = &mut self.voices[index].slots[slot];
            if !start {
                sv.scan += scan_per_sample * CONTROL_PERIOD as f64;
                sv.scan -= sv.scan.floor();
            }
            let pos = (val(P::POSITION) + sv.scan as f32).rem_euclid(1.0);
            let switched = sv.kind != Some(kind);
            sv.kind = Some(kind);

            sv.ctl = match kind {
                EngineKind::Grain => {
                    let base = if val(P::G_TRACK) >= 0.5 { note_hz } else { FREE_RATE_HZ };
                    let rate_hz = (base * val(P::G_RATE).exp2()).clamp(0.05, 0.45 * self.sr);
                    let root = if content.root_hz > 0.0 { content.root_hz } else { FREE_RATE_HZ };
                    let key = if val(P::G_KEYPITCH) >= 0.5 { note_hz / root } else { (val(P::TUNE) / 12.0).exp2() };
                    let speed = key * (val(P::G_PITCH) / 12.0).exp2() * src.sr / self.sr;
                    let (size, shape, scatter) = (val(P::G_SIZE), val(P::G_SHAPE), val(P::G_SCATTER));
                    if switched {
                        sv.grain.reset();
                    }
                    SlotCtl::Grain(GrainParams {
                        rate: rate_hz / self.sr,
                        size,
                        shape,
                        speed: speed as f64,
                        pos,
                        scatter,
                        jitter: val(P::G_JITTER),
                        spread: val(P::SPREAD),
                        gain: level * overlap_gain(size, shape, scatter),
                    })
                }
                EngineKind::Spectral => match content.model.as_ref() {
                    // Still analysing: the slot is silent until the model lands.
                    None => SlotCtl::Off,
                    Some(model) => {
                        let mut gains = model.gains_at(pos, val(P::S_SMOOTH));
                        let tilt = tilt_weights(val(P::S_TILT));
                        for k in 0..LAYERS {
                            gains[k] *= tilt[k] * level;
                        }
                        if start || switched {
                            sv.spectral.snap(gains);
                        }
                        SlotCtl::Spectral(SpectralParams {
                            freq: (note_hz / self.sr).min(0.45),
                            gains,
                            pure: val(P::S_PURE),
                            warp: val(P::S_WARP),
                            fm: val(P::S_FM),
                            ratio: val(P::S_RATIO),
                            spread: val(P::SPREAD),
                            // Reach the new envelope point within one control period.
                            glide: 3.0 / CONTROL_PERIOD as f32,
                        })
                    }
                },
            };
            if self.voices[index].serial == self.serial {
                self.playhead[slot] = pos;
            }
        }
    }

    /// Render `out_l.len()` frames, replacing the buffers' contents.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let frames = out_l.len().min(out_r.len());
        out_l[..frames].fill(0.0);
        out_r[..frames].fill(0.0);
        let mut done = 0;
        while done < frames {
            if self.control_in == 0 {
                self.refresh_globals();
                for i in 0..VOICES {
                    if self.voices[i].active {
                        self.refresh_voice(i, false);
                    }
                }
                self.control_in = CONTROL_PERIOD;
            }
            let n = self.control_in.min(frames - done);
            self.render(&mut out_l[done..done + n], &mut out_r[done..done + n]);
            self.control_in -= n;
            done += n;
        }
    }

    fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let (place, mix) = (self.place, self.mix);
        let (filt_on, g, k, a1, mode) = (self.filt_on, self.filt_g, self.filt_k, self.filt_a1, self.filt_mode);
        let content = &self.content;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            for i in 0..out_l.len() {
                let mut s = [(0.0f32, 0.0f32); SLOTS];
                for slot in 0..SLOTS {
                    let sv = &mut v.slots[slot];
                    let (l, r) = match (&sv.ctl, &content[slot]) {
                        (SlotCtl::Grain(p), SlotContent { source: Some(src), .. }) => sv.grain.tick(&src.data, p),
                        (SlotCtl::Spectral(p), SlotContent { model: Some(m), .. }) => sv.spectral.tick(m, p),
                        _ => (0.0, 0.0),
                    };
                    // Mid/side placement: width scales the side, balance pans the mid.
                    let (mid, side) = (0.5 * (l + r), 0.5 * (l - r) * place[slot].side);
                    s[slot] = (mid * place[slot].mid_l + side, mid * place[slot].mid_r - side);
                }
                let mut l = mix[0] * s[0].0 + mix[1] * s[1].0 + mix[2] * s[0].0 * s[1].0;
                let mut r = mix[0] * s[0].1 + mix[1] * s[1].1 + mix[2] * s[0].1 * s[1].1;
                let amp = v.env.process() * v.vel * VOICE_GAIN;
                l *= amp;
                r *= amp;
                if filt_on {
                    let (fl, fr) = (v.filter[0].tick(l, g, k, a1), v.filter[1].tick(r, g, k, a1));
                    (l, r) = match mode {
                        1 => (fl.1, fr.1),
                        2 => (fl.2, fr.2),
                        _ => (fl.0, fr.0),
                    };
                }
                out_l[i] += l;
                out_r[i] += r;
            }
            if !v.env.is_active() {
                v.active = false;
            }
        }
        for i in 0..out_l.len() {
            self.volume += (self.volume_target - self.volume) * 0.002;
            out_l[i] = soft_clip(out_l[i] * self.volume);
            out_r[i] = soft_clip(out_r[i] * self.volume);
        }
    }
}

fn oldest(voices: &[Voice], pred: impl Fn(&Voice) -> bool) -> Option<usize> {
    voices.iter().enumerate().filter(|(_, v)| pred(v)).min_by_key(|(_, v)| v.serial).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze_blocking;

    fn engine() -> Engine {
        let mut e = Engine::new(48_000.0);
        for slot in 0..SLOTS {
            let source = Arc::new(Source::builtin(slot as u32));
            let model = analyze_blocking(source.clone());
            let root_hz = model.root_hz;
            e.set_content(slot, SlotContent { source: Some(source), model: Some(Arc::new(model)), root_hz });
        }
        e
    }

    fn set(e: &mut Engine, id: u32, v: f32) {
        e.set_param(P::index_of(id).unwrap(), v);
    }

    fn render(e: &mut Engine, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        let mut i = 0;
        while i < frames {
            let n = block.min(frames - i);
            e.process(&mut l[i..i + n], &mut r[i..i + n]);
            i += n;
        }
        (l, r)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Fundamental by autocorrelation over a plausible lag range.
    fn pitch_hz(x: &[f32], sr: f32) -> f32 {
        let (lo, hi) = ((sr / 2000.0) as usize, (sr / 40.0) as usize);
        let n = x.len() - hi;
        let corr = |lag: usize| -> f32 { (0..n).map(|i| x[i] * x[i + lag]).sum() };
        let best = (lo..hi).map(corr).fold(0.0f32, f32::max);
        let lag = (lo + 1..hi - 1).find(|&l| corr(l) > 0.9 * best && corr(l) >= corr(l - 1) && corr(l) >= corr(l + 1));
        sr / lag.unwrap() as f32
    }

    #[test]
    fn default_patch_sounds_and_goes_silent_after_release() {
        let mut e = engine();
        e.note_on(60, 0.8);
        let (l, r) = render(&mut e, 24_000, 128);
        assert!(rms(&l) > 0.01 && rms(&r) > 0.01, "rms {} {}", rms(&l), rms(&r));
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0));
        e.note_off(60);
        render(&mut e, 48_000, 128);
        assert_eq!(e.active_voices(), 0);
        let (l, _) = render(&mut e, 4800, 128);
        assert!(l.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn each_engine_plays_the_note_that_was_asked_for() {
        // Slot A alone (grain, key-tracked audio-rate) then slot B alone
        // (spectral): both must land on the played note.
        for (fade, name) in [(0.0f32, "grain"), (1.0, "spectral")] {
            for note in [48u8, 57, 69] {
                let mut e = engine();
                set(&mut e, P::FADE, fade);
                set(&mut e, P::SLOT_A + P::SPREAD, 0.0);
                set(&mut e, P::SLOT_B + P::SPREAD, 0.0);
                e.note_on(note, 1.0);
                let (l, _) = render(&mut e, 24_000, 128);
                let want = midi_to_freq(note as f32);
                let got = pitch_hz(&l[4800..], 48_000.0);
                let cents = 1200.0 * (got / want).log2();
                assert!(cents.abs() < 15.0, "{name} note {note}: {got} Hz, wanted {want} Hz");
            }
        }
    }

    #[test]
    fn output_does_not_depend_on_block_size() {
        let run = |block: usize| {
            let mut e = engine();
            set(&mut e, P::SLOT_A + P::SCAN, 0.5);
            set(&mut e, P::SLOT_A + P::G_SCATTER, 0.3);
            set(&mut e, P::SLOT_B + P::SCAN, 1.0);
            e.note_on(55, 0.7);
            e.note_on(62, 0.9);
            render(&mut e, 6000, block)
        };
        let a = run(128);
        assert_eq!(a, run(1));
        assert_eq!(a, run(480));
    }

    #[test]
    fn either_engine_runs_in_either_slot() {
        for (ea, eb) in [(0.0f32, 0.0f32), (1.0, 1.0), (1.0, 0.0)] {
            for fade in [0.0f32, 1.0] {
                let mut e = engine();
                set(&mut e, P::SLOT_A + P::ENGINE, ea);
                set(&mut e, P::SLOT_B + P::ENGINE, eb);
                set(&mut e, P::FADE, fade);
                e.note_on(60, 1.0);
                let (l, _) = render(&mut e, 9600, 128);
                assert!(rms(&l[2400..]) > 0.01, "engines {ea}/{eb} fade {fade} silent");
            }
        }
    }

    #[test]
    fn width_is_mid_side_and_balance_pans() {
        let side_ratio = |e: &mut Engine| {
            e.note_on(60, 1.0);
            let (l, r) = render(e, 24_000, 128);
            let mid: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a + b).collect();
            let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
            rms(&side) / rms(&mid)
        };
        // Width 0 collapses a slot to mono, whatever the engine spreads.
        for slot in [P::SLOT_A, P::SLOT_B] {
            let mut e = engine();
            set(&mut e, P::FADE, if slot == P::SLOT_A { 0.0 } else { 1.0 });
            set(&mut e, slot + P::SPREAD, 1.0);
            set(&mut e, slot + P::WIDTH, 0.0);
            assert!(side_ratio(&mut e) < 1e-4);
            let mut e = engine();
            set(&mut e, P::FADE, if slot == P::SLOT_A { 0.0 } else { 1.0 });
            set(&mut e, slot + P::SPREAD, 1.0);
            set(&mut e, slot + P::WIDTH, 2.0);
            assert!(side_ratio(&mut e) > 0.05);
        }
        // The defaults put slot A nearer the centre than slot B.
        let mut a = engine();
        set(&mut a, P::FADE, 0.0);
        let mut b = engine();
        set(&mut b, P::FADE, 1.0);
        assert!(side_ratio(&mut a) < side_ratio(&mut b));
        // Balance hard left leaves the right channel with side only.
        let mut e = engine();
        set(&mut e, P::FADE, 0.0);
        set(&mut e, P::SLOT_A + P::WIDTH, 0.0);
        set(&mut e, P::SLOT_A + P::BALANCE, -1.0);
        e.note_on(60, 1.0);
        let (l, r) = render(&mut e, 9600, 128);
        assert!(rms(&l) > 0.01 && rms(&r) < 1e-4 * rms(&l).max(1.0));
    }

    #[test]
    fn ring_mode_differs_from_fade_and_needs_both_slots() {
        let run = |mode: f32, level_b: f32| {
            let mut e = engine();
            set(&mut e, P::MIX_MODE, mode);
            set(&mut e, P::SLOT_B + P::LEVEL, level_b);
            e.note_on(60, 1.0);
            render(&mut e, 9600, 128).0
        };
        let (fade, ring) = (run(0.0, 0.8), run(1.0, 0.8));
        assert!(rms(&ring[2400..]) > 0.003, "ring silent: {}", rms(&ring[2400..]));
        assert!(fade.iter().zip(&ring).any(|(a, b)| (a - b).abs() > 0.01));
        // A product: with slot B at zero the centre of the ring is silent.
        assert!(rms(&run(1.0, 0.0)[2400..]) < 1e-6);
    }

    #[test]
    fn filter_modes_shape_the_spectrum() {
        let run = |mode: f32, cutoff: f32| {
            let mut e = engine();
            set(&mut e, P::F_MODE, mode);
            set(&mut e, P::F_CUTOFF, cutoff);
            e.note_on(48, 1.0);
            let (l, _) = render(&mut e, 24_000, 128);
            assert!(l.iter().all(|v| v.is_finite()));
            rms(&l[4800..])
        };
        let open = run(0.0, 20_000.0);
        assert!(run(0.0, 60.0) < 0.5 * open, "low-pass did not attenuate");
        assert!(run(2.0, 12_000.0) < 0.5 * open, "high-pass did not attenuate");
    }

    #[test]
    fn stealing_never_exceeds_the_pool_and_stays_bounded() {
        let mut e = engine();
        for n in 0..40u8 {
            e.note_on(36 + n, 1.0);
            let (l, r) = render(&mut e, 480, 128);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0));
        }
        assert_eq!(e.active_voices(), VOICES);
        e.reset();
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        for hi in [false, true] {
            let mut e = engine();
            for (i, d) in PARAM_DEFS.iter().enumerate() {
                e.set_param(i, if hi { d.max } else { d.min } as f32);
            }
            set(&mut e, P::VOLUME, 1.0);
            set(&mut e, P::FADE, 0.5);
            set(&mut e, P::SLOT_A + P::LEVEL, 1.0);
            set(&mut e, P::SLOT_B + P::LEVEL, 1.0);
            for n in [24u8, 60, 108] {
                e.note_on(n, 1.0);
            }
            let (l, r) = render(&mut e, 24_000, 128);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0), "hi={hi}");
        }
    }

    #[test]
    fn a_slot_without_content_is_silent_not_broken() {
        let mut e = Engine::new(48_000.0);
        e.note_on(60, 1.0);
        let (l, _) = render(&mut e, 4800, 128);
        assert!(l.iter().all(|v| *v == 0.0));
        // Spectral selected but the model has not arrived yet.
        let source = Arc::new(Source::builtin(0));
        e.set_content(1, SlotContent { source: Some(source), model: None, root_hz: 0.0 });
        let (l, _) = render(&mut e, 4800, 128);
        assert!(l.iter().all(|v| *v == 0.0));
    }
}
