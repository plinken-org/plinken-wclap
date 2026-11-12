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
/// Time the read position takes to follow the Position control. A knob
/// arrives as a few dozen jumps a second; reading the source from a new
/// place on each one is heard as a rattle, so the position glides.
const POSITION_GLIDE_SECONDS: f32 = 0.06;
/// Time the spectral layer gains take to follow the envelopes.
const LAYER_GLIDE_SECONDS: f32 = 0.004;
/// Output ceiling of the peak limiter (about -1 dBFS).
const LIMIT_CEILING: f32 = 0.89;
/// Time the limiter takes to let go after a peak.
const LIMIT_RELEASE_SECONDS: f32 = 0.12;
/// How long the limiter holds its gain after a peak before letting go:
/// longer than one cycle of the lowest notes, so the gain does not creep
/// back up between the crests of a bass note and flatten each one anew.
const LIMIT_HOLD_SECONDS: f32 = 0.025;
/// Reference pitch for grain rate with key tracking off (C3).
const FREE_RATE_HZ: f32 = 130.812_78;
/// Make-up gain for the ring product of two signals well below full scale.
const RING_GAIN: f32 = 4.0;
/// Headroom for a chord before the output soft clip.
const VOICE_GAIN: f32 = 0.8;

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
    /// Where this voice's share of the slot sits in the stereo field.
    place: Placement,
}

/// Where successive notes sit around their slot's balance, as a fraction
/// of the way to the far side: alternating left and right, so a chord
/// fans out across the field instead of stacking on one point.
const NOTE_PANS: [f32; 8] = [-0.35, 0.35, -0.7, 0.7, -0.15, 0.15, -0.55, 0.55];
const CENTRE: Placement = Placement { mid_l: 1.0, mid_r: 1.0, side: 1.0 };

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
    /// This note's place around the slot balance (one of [`NOTE_PANS`]).
    pan: f32,
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
    /// Largest output sample since the UI last asked.
    out_peak: f32,
    /// The Position control per slot, after its glide.
    position: [f32; SLOTS],
    /// Current gain of the output limiter (1 = not limiting).
    limit_gain: f32,
    /// Samples left before the limiter may start to release.
    limit_hold: u32,
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
            pan: 0.0,
            env: Envelope::new(sr),
            slots: core::array::from_fn(|s| SlotVoice {
                grain: GrainVoice::new(0x9e37_79b9u32.wrapping_mul((i * SLOTS + s + 1) as u32)),
                spectral: SpectralVoice::new(),
                scan: 0.0,
                ctl: SlotCtl::Off,
                kind: None,
                place: CENTRE,
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
            mix: [0.0; 3],
            filt_on: false,
            filt_g: 0.0,
            filt_k: 2.0,
            filt_a1: 0.0,
            filt_mode: 0,
            volume: 0.0,
            volume_target: 0.0,
            playhead: [0.0; SLOTS],
            out_peak: 0.0,
            position: [0.0; SLOTS],
            limit_gain: 1.0,
            limit_hold: 0,
        };
        for slot in 0..SLOTS {
            e.position[slot] = e.slot_value(slot, P::POSITION);
        }
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

    /// Output peak since the last call (for the UI's level meter).
    pub fn take_peak(&mut self) -> f32 {
        core::mem::take(&mut self.out_peak)
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
        v.pan = NOTE_PANS[(serial % NOTE_PANS.len() as u64) as usize];
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
        let follow = 1.0 - (-(CONTROL_PERIOD as f32) / (POSITION_GLIDE_SECONDS * self.sr)).exp();
        for slot in 0..SLOTS {
            let target = self.slot_value(slot, P::POSITION);
            self.position[slot] += (target - self.position[slot]) * follow;
        }
    }

    /// Recompute one voice's per-slot control values. `start` is true on
    /// the note's first refresh (no scan advance, no gain glide).
    fn refresh_voice(&mut self, index: usize, start: bool) {
        let note = self.voices[index].note as f32;
        let note_pan = self.voices[index].pan;
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
            // Mid/side placement. Width scales everything stereo the slot
            // makes (0 = one point), Balance says where that point is, and
            // Spread fans the notes out around it.
            let width = val(P::WIDTH);
            let fan = note_pan * val(P::SPREAD) * width.min(1.0);
            let theta = ((val(P::BALANCE) + fan).clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
            let place = Placement { mid_l: SQRT_2 * theta.cos(), mid_r: SQRT_2 * theta.sin(), side: width };
            let note_hz = midi_to_freq(note + val(P::TUNE));
            let scan_per_sample = val(P::SCAN) as f64 / (src.seconds() as f64 * self.sr as f64);

            let sv = &mut self.voices[index].slots[slot];
            sv.place = place;
            if !start {
                sv.scan += scan_per_sample * CONTROL_PERIOD as f64;
                sv.scan -= sv.scan.floor();
            }
            let pos = (self.position[slot] + sv.scan as f32).rem_euclid(1.0);
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
                        gain: level * overlap_gain(size, shape, scatter) * src.makeup_at(pos),
                        period: match content.model.as_ref() {
                            Some(m) if m.pitched => src.sr / m.root_hz,
                            _ => 0.0,
                        },
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
                            glide: 1.0 - (-1.0 / (LAYER_GLIDE_SECONDS * self.sr)).exp(),
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
        let mix = self.mix;
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
                    let place = sv.place;
                    let (mid, side) = (0.5 * (l + r), 0.5 * (l - r) * place.side);
                    s[slot] = (mid * place.mid_l + side, mid * place.mid_r - side);
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
        // Peak limiter: a resonant filter or a big chord can exceed full
        // scale by a wide margin. No lookahead, so no latency: the gain
        // drops at once to whatever keeps the loudest channel at the
        // ceiling (both channels together, so the image does not move),
        // holds there for a cycle of the lowest notes, then recovers
        // slowly — over a held note it settles to a steady gain instead of
        // reshaping every cycle.
        let release = 1.0 - (-1.0 / (LIMIT_RELEASE_SECONDS * self.sr)).exp();
        let hold = (LIMIT_HOLD_SECONDS * self.sr) as u32;
        for i in 0..out_l.len() {
            self.volume += (self.volume_target - self.volume) * 0.002;
            let (l, r) = (out_l[i] * self.volume, out_r[i] * self.volume);
            let peak = l.abs().max(r.abs());
            // The meter shows the level asked for, before limiting: above
            // the ceiling it reads as "the limiter is working".
            self.out_peak = self.out_peak.max(peak);
            let want = if peak > LIMIT_CEILING { LIMIT_CEILING / peak } else { 1.0 };
            if want < self.limit_gain {
                self.limit_gain = want;
                self.limit_hold = hold;
            } else if self.limit_hold > 0 {
                self.limit_hold -= 1;
            } else {
                self.limit_gain += (want - self.limit_gain) * release;
            }
            // The clip after it never engages below 1.0; it only guards
            // against a non-finite or runaway value.
            out_l[i] = soft_clip(l * self.limit_gain);
            out_r[i] = soft_clip(r * self.limit_gain);
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
        // Width 0 collapses a centred slot to one point, whatever the
        // engine spreads and wherever the note would have been fanned to.
        for slot in [P::SLOT_A, P::SLOT_B] {
            for width in [0.0f32, 2.0] {
                let mut e = engine();
                set(&mut e, P::FADE, if slot == P::SLOT_A { 0.0 } else { 1.0 });
                set(&mut e, slot + P::BALANCE, 0.0);
                set(&mut e, slot + P::SPREAD, 1.0);
                set(&mut e, slot + P::WIDTH, width);
                let ratio = side_ratio(&mut e);
                if width == 0.0 {
                    assert!(ratio < 1e-4, "width 0 left side energy {ratio}");
                } else {
                    assert!(ratio > 0.05, "width 2 gave no stereo ({ratio})");
                }
            }
        }
        // Balance hard left leaves the right channel empty.
        let mut e = engine();
        set(&mut e, P::FADE, 0.0);
        set(&mut e, P::SLOT_A + P::WIDTH, 0.0);
        set(&mut e, P::SLOT_A + P::BALANCE, -1.0);
        e.note_on(60, 1.0);
        let (l, r) = render(&mut e, 9600, 128);
        assert!(rms(&l) > 0.01 && rms(&r) < 1e-4 * rms(&l).max(1.0));
    }

    #[test]
    fn the_default_patch_opens_the_field_left_and_right() {
        let play = |fade: f32| {
            let mut e = engine();
            set(&mut e, P::FADE, fade);
            e.note_on(60, 1.0);
            let (l, r) = render(&mut e, 24_000, 128);
            (rms(&l[4800..]), rms(&r[4800..]), l, r)
        };
        // Slot A alone sits left, slot B alone right, by a clear margin.
        let (al, ar, ..) = play(0.0);
        assert!(al > 1.5 * ar, "slot A: left {al}, right {ar}");
        let (bl, br, ..) = play(1.0);
        assert!(br > 1.5 * bl, "slot B: left {bl}, right {br}");
        // Together they are wide: the side signal is a good part of the mid.
        let (.., l, r) = play(0.5);
        let mid: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a + b).collect();
        let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(rms(&side) > 0.4 * rms(&mid), "side {} against mid {}", rms(&side), rms(&mid));
        // And it folds down to mono without disappearing.
        assert!(rms(&mid) > 0.02);
    }

    #[test]
    fn spread_fans_the_notes_of_a_chord_across_the_field() {
        // One centred slot, no stereo from the engine itself except Spread.
        let chord = |spread: f32| {
            let mut e = engine();
            set(&mut e, P::FADE, 1.0);
            set(&mut e, P::SLOT_B + P::BALANCE, 0.0);
            set(&mut e, P::SLOT_B + P::SPREAD, spread);
            // Per-note levels left and right, one note at a time.
            (0..4)
                .map(|i| {
                    e.note_on(48 + 5 * i, 1.0);
                    let (l, r) = render(&mut e, 9600, 128);
                    e.note_off(48 + 5 * i);
                    render(&mut e, 48_000, 128);
                    rms(&l[2400..]) / rms(&r[2400..])
                })
                .collect::<Vec<f32>>()
        };
        let fanned = chord(1.0);
        // Successive notes land on opposite sides, some of them far out.
        for pair in fanned.windows(2) {
            assert!((pair[0] - 1.0) * (pair[1] - 1.0) < 0.0, "two notes in a row on one side: {fanned:?}");
        }
        assert!(fanned.iter().any(|r| *r > 1.4) && fanned.iter().any(|r| *r < 1.0 / 1.4), "{fanned:?}");
        for ratio in chord(0.0) {
            assert!((ratio - 1.0).abs() < 0.02, "spread 0 still pans: {ratio}");
        }
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

    /// A music-like source: level and brightness change along its length.
    fn varied_engine() -> Engine {
        let sr = 48_000.0f32;
        let n = (sr * 6.0) as usize;
        let data: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                let swell = 0.15 + 0.85 * (0.5 - 0.5 * (core::f32::consts::TAU * t / 1.3).cos());
                let bright = 0.5 - 0.5 * (core::f32::consts::TAU * t / 2.1).cos();
                let ph = core::f32::consts::TAU * 196.0 * t;
                swell * (1..=24).map(|h| (ph * h as f32).sin() / (h as f32).powf(2.0 - 1.5 * bright)).sum::<f32>()
            })
            .collect();
        let mut e = Engine::new(48_000.0);
        for slot in 0..SLOTS {
            let source = Arc::new(Source::from_mono(sr, data.clone()).unwrap());
            let model = analyze_blocking(source.clone());
            let root_hz = model.root_hz;
            e.set_content(slot, SlotContent { source: Some(source), model: Some(Arc::new(model)), root_hz });
        }
        e
    }

    /// Turning the Position knob arrives as a stream of small jumps (one per
    /// UI message, ~60 a second) while the read point crosses a source whose
    /// level and timbre keep changing. The sound must glide through it: the
    /// level of one cycle of the note may not differ from the next by more
    /// than a fifth of the average. (Before the position glide and phase-
    /// locked grains this was 30 % for Spectral and 70–100 % for Grain.)
    #[test]
    fn sweeping_position_is_smooth_from_cycle_to_cycle() {
        for (engine_kind, name) in [(1.0f32, "spectral"), (0.0, "grain")] {
            let mut e = varied_engine();
            set(&mut e, P::FADE, 0.0);
            set(&mut e, P::SLOT_A + P::ENGINE, engine_kind);
            set(&mut e, P::SLOT_A + P::SPREAD, 0.0);
            set(&mut e, P::SLOT_A + P::POSITION, 0.1);
            e.note_on(55, 1.0);
            render(&mut e, 4800, 128);
            // One second, 60 knob messages, across three quarters of the source.
            let mut swept = Vec::new();
            for step in 0..60 {
                set(&mut e, P::SLOT_A + P::POSITION, 0.1 + 0.75 * (step + 1) as f32 / 60.0);
                swept.extend(render(&mut e, 800, 128).0);
            }
            assert!(swept.iter().all(|v| v.is_finite()));
            // G3 is 245 samples a cycle at 48 kHz.
            let levels: Vec<f32> = swept.chunks_exact(245).map(rms).collect();
            let mean = levels.iter().sum::<f32>() / levels.len() as f32;
            let jump = levels.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            assert!(mean > 0.02, "{name}: too quiet to judge ({mean})");
            assert!(jump < 0.2 * mean, "{name}: level jumps {:.0} % between cycles", 100.0 * jump / mean);
        }
    }

    #[test]
    fn position_follows_its_control_gradually() {
        let mut e = engine();
        set(&mut e, P::FADE, 0.0);
        e.note_on(60, 1.0);
        render(&mut e, 4800, 128);
        let before = e.playhead(0);
        set(&mut e, P::SLOT_A + P::POSITION, 0.9);
        render(&mut e, 128, 128);
        let soon = e.playhead(0);
        assert!(soon > before && soon < 0.3, "after one block the read point is at {soon}");
        render(&mut e, 24_000, 128);
        assert!((e.playhead(0) - 0.9).abs() < 0.01, "after half a second it is at {}", e.playhead(0));
    }

    #[test]
    fn the_limiter_holds_a_screaming_filter_under_the_ceiling() {
        // Full resonance with the cutoff on a partial, a chord, everything up.
        let mut e = engine();
        set(&mut e, P::VOLUME, 1.0);
        set(&mut e, P::F_RES, 1.0);
        set(&mut e, P::F_CUTOFF, 523.25);
        set(&mut e, P::SLOT_A + P::LEVEL, 1.0);
        set(&mut e, P::SLOT_B + P::LEVEL, 1.0);
        for n in [36u8, 48, 60, 64, 67, 72] {
            e.note_on(n, 1.0);
        }
        let (l, r) = render(&mut e, 48_000, 128);
        let asked = e.take_peak();
        assert!(asked > 1.5, "the patch should overshoot without a limiter (asked for {asked})");
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |p, v| p.max(v.abs()));
        assert!(peak <= LIMIT_CEILING + 1e-4, "peak {peak} above the ceiling");
        // Limited, not squashed to nothing, and not clipped. Clipping this
        // patch at the same ceiling would hold a large share of the samples
        // flat; the limiter lets only brief crests touch it (under 1 % of
        // the sustained part, never for as long as a millisecond). Those
        // remain because there is no lookahead: when the notes of a chord
        // beat upwards the gain follows the rise instead of anticipating it.
        let sustained = &l[24_000..];
        assert!(rms(sustained) > 0.1);
        let pinned = |x: &[f32]| x.iter().filter(|v| v.abs() > 0.999 * LIMIT_CEILING).count();
        let clipped: Vec<f32> = sustained.iter().map(|v| v / e.limit_gain).map(|v| v.clamp(-LIMIT_CEILING, LIMIT_CEILING)).collect();
        assert!(pinned(sustained) * 100 < sustained.len(), "{} of {} samples at the ceiling", pinned(sustained), sustained.len());
        assert!(pinned(&clipped) > 5 * pinned(sustained), "no better than clipping: {} vs {}", pinned(&clipped), pinned(sustained));
        let (mut longest, mut run) = (0, 0);
        for v in sustained {
            run = if v.abs() > 0.999 * LIMIT_CEILING { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        assert!(longest < 48, "{longest} samples in a row at the ceiling");
    }

    #[test]
    fn the_limiter_leaves_normal_playing_alone() {
        let mut e = engine();
        e.note_on(60, 0.8);
        let (l, _) = render(&mut e, 24_000, 128);
        assert!(l.iter().fold(0.0f32, |p, v| p.max(v.abs())) < LIMIT_CEILING);
        assert_eq!(e.limit_gain, 1.0);
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
