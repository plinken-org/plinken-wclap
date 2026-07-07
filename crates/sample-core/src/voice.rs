//! Pitched sample voices + polyphonic pool.
//!
//! Ported from the private monorepo's `plugins/Sampler/src/voice.rs`, with
//! the linear release fade replaced by a real `plinken_dsp::Envelope` ADSR,
//! a click-free `choke()` (forced ~3 ms release for drum mute groups), and
//! `tick_pitched()` for driving a voice as a synth oscillator.

use crate::sample::SampleData;
use plinken_dsp::Envelope;
use std::sync::Arc;

/// Voice scaling factor to prevent clipping with many voices.
/// 1/4 = -12 dB headroom.
pub const VOICE_SCALE: f32 = 0.25;

/// Forced-release time used by [`SampleVoice::choke`], in seconds. Long
/// enough to avoid a click, short enough to read as an immediate cut.
const CHOKE_RELEASE_S: f32 = 0.003;

/// Semitone-based playback-rate multiplier (2.0 = up one octave).
#[inline]
pub fn pitch_ratio(note: u8, root_key: u8) -> f64 {
    let semitone_diff = note as f64 - root_key as f64;
    2.0_f64.powf(semitone_diff / 12.0)
}

/// Playback behaviour at the end of the sample / on note-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Play once; note-off enters the release phase.
    #[default]
    NoLoop,
    /// Loop between loop points (or whole sample) forever while held.
    LoopContinuous,
    /// Loop while held; on release play through to the end.
    LoopSustain,
    /// Ignore note-off; play to the end (drum pads).
    OneShot,
}

/// Per-trigger playback parameters, resolved by the caller (a Pulze pad or
/// a Synome voice) before the trigger.
#[derive(Debug, Clone, Copy)]
pub struct VoiceParams {
    pub root_key: u8,
    pub tune_cents: f32,
    /// Linear gain applied on top of velocity.
    pub gain: f32,
    /// -1.0 (left) .. 1.0 (right).
    pub pan: f32,
    pub loop_mode: LoopMode,
    pub loop_start: Option<u32>,
    pub loop_end: Option<u32>,
    /// ADSR in seconds / 0..1 sustain.
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for VoiceParams {
    fn default() -> Self {
        Self {
            root_key: 60,
            tune_cents: 0.0,
            gain: 1.0,
            pan: 0.0,
            loop_mode: LoopMode::NoLoop,
            loop_start: None,
            loop_end: None,
            attack: 0.001,
            decay: 0.1,
            sustain: 1.0,
            release: 0.05,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceState {
    Idle,
    Playing,
    Release,
}

/// A single sample-playback voice.
pub struct SampleVoice {
    note: u8,
    velocity_amp: f32,
    state: VoiceState,
    sample: Option<Arc<SampleData>>,
    /// Playback position in source frames (fractional).
    position: f64,
    pitch_ratio: f64,
    env: Envelope,
    gain: f32,
    pan_l: f32,
    pan_r: f32,
    loop_mode: LoopMode,
    loop_start: Option<u32>,
    loop_end: Option<u32>,
    /// Monotonic trigger stamp for oldest-voice stealing.
    age: u64,
}

impl SampleVoice {
    pub fn new() -> Self {
        Self {
            note: 0,
            velocity_amp: 0.0,
            state: VoiceState::Idle,
            sample: None,
            position: 0.0,
            pitch_ratio: 1.0,
            env: Envelope::new(48000.0),
            gain: 1.0,
            pan_l: core::f32::consts::FRAC_1_SQRT_2,
            pan_r: core::f32::consts::FRAC_1_SQRT_2,
            loop_mode: LoopMode::NoLoop,
            loop_start: None,
            loop_end: None,
            age: 0,
        }
    }

    /// Start playing `sample` at `note`, resolved against `params`.
    pub fn trigger(
        &mut self,
        sample: Arc<SampleData>,
        note: u8,
        velocity: u8,
        params: &VoiceParams,
        host_sample_rate: f32,
        age: u64,
    ) {
        self.note = note;
        self.velocity_amp = velocity as f32 / 127.0;
        self.state = VoiceState::Playing;
        self.position = 0.0;
        let base_pitch = pitch_ratio(note, params.root_key);
        let tune_ratio = 2.0_f64.powf(params.tune_cents as f64 / 1200.0);
        self.pitch_ratio = base_pitch * tune_ratio;
        self.sample = Some(sample);
        self.gain = params.gain;
        // Equal-power pan.
        let pan = params.pan.clamp(-1.0, 1.0);
        self.pan_l = ((1.0 - pan) * 0.5).sqrt();
        self.pan_r = ((1.0 + pan) * 0.5).sqrt();
        self.loop_mode = params.loop_mode;
        self.loop_start = params.loop_start;
        self.loop_end = params.loop_end;
        self.env = Envelope::new(host_sample_rate);
        self.env
            .set_adsr(params.attack, params.decay, params.sustain, params.release);
        self.env.trigger();
        self.age = age;
    }

    /// Note-off: enter the envelope's release phase (ignored by OneShot).
    pub fn release(&mut self) {
        if self.state == VoiceState::Playing && self.loop_mode != LoopMode::OneShot {
            self.state = VoiceState::Release;
            self.env.release();
        }
    }

    /// Click-free immediate cut (drum mute groups, voice steal): force a
    /// very short release regardless of the configured one.
    pub fn choke(&mut self) {
        if self.state != VoiceState::Idle {
            self.state = VoiceState::Release;
            self.env.set_adsr(0.001, 0.001, 1.0, CHOKE_RELEASE_S);
            self.env.release();
        }
    }

    /// Hard stop. Clicks — prefer [`SampleVoice::choke`].
    pub fn kill(&mut self) {
        self.state = VoiceState::Idle;
        self.sample = None;
        self.position = 0.0;
    }

    pub fn is_active(&self) -> bool {
        self.state != VoiceState::Idle
    }

    pub fn is_releasing(&self) -> bool {
        self.state == VoiceState::Release
    }

    pub fn is_playing_note(&self, note: u8) -> bool {
        self.is_active() && self.note == note && self.state == VoiceState::Playing
    }

    pub fn note(&self) -> u8 {
        self.note
    }

    /// Trigger stamp (for oldest-voice stealing).
    pub fn age(&self) -> u64 {
        self.age
    }

    /// Advance `position` by one host frame's worth of source frames,
    /// honouring the loop mode. Returns false when playback ended.
    #[inline]
    fn advance(&mut self, step: f64, sample_len: f64) -> bool {
        self.position += step;
        match self.loop_mode {
            LoopMode::NoLoop | LoopMode::OneShot => {
                if self.position >= sample_len {
                    return false;
                }
            }
            LoopMode::LoopContinuous => {
                let (start, end) = self.loop_points(sample_len);
                if self.position >= end {
                    self.position = start + (self.position - end);
                }
            }
            LoopMode::LoopSustain => {
                if self.state == VoiceState::Playing {
                    let (start, end) = self.loop_points(sample_len);
                    if self.position >= end {
                        self.position = start + (self.position - end);
                    }
                } else if self.position >= sample_len {
                    return false;
                }
            }
        }
        true
    }

    #[inline]
    fn loop_points(&self, sample_len: f64) -> (f64, f64) {
        match (self.loop_start, self.loop_end) {
            (Some(s), Some(e)) if (s as f64) < (e as f64).min(sample_len) => {
                (s as f64, (e as f64).min(sample_len))
            }
            _ => (0.0, sample_len),
        }
    }

    /// Render one stereo frame at the voice's own pitch ratio.
    pub fn render(&mut self, host_sample_rate: f32) -> (f32, f32) {
        if self.state == VoiceState::Idle {
            return (0.0, 0.0);
        }
        let Some(sample) = self.sample.clone() else {
            self.state = VoiceState::Idle;
            return (0.0, 0.0);
        };
        let sample_len = sample.frame_count as f64;
        if sample_len <= 0.0 {
            self.state = VoiceState::Idle;
            return (0.0, 0.0);
        }

        let left = sample.get_sample_interpolated(self.position, 0);
        let right = if sample.channels == 2 {
            sample.get_sample_interpolated(self.position, 1)
        } else {
            left
        };

        let rate_ratio = sample.sample_rate as f64 / host_sample_rate as f64;
        if !self.advance(self.pitch_ratio * rate_ratio, sample_len) {
            self.state = VoiceState::Idle;
        }

        let env = self.env.process();
        if !self.env.is_active() {
            self.state = VoiceState::Idle;
        }

        let gain = self.velocity_amp * self.gain * env * VOICE_SCALE;
        (left * gain * self.pan_l, right * gain * self.pan_r)
    }

    /// Mono tick for the sample-as-oscillator path: the position advances
    /// by `ratio_override` (already combining note pitch, modulation, and
    /// source/host rate ratio — computed per-sample by the synth), and the
    /// voice's own envelope/pan/gain are BYPASSED — the synth's amp env,
    /// filter and mixer own the shaping. Returns (L+R)/2 pre-filter.
    pub fn tick_pitched(&mut self, ratio_override: f64) -> f32 {
        if self.state == VoiceState::Idle {
            return 0.0;
        }
        let Some(sample) = self.sample.clone() else {
            self.state = VoiceState::Idle;
            return 0.0;
        };
        let sample_len = sample.frame_count as f64;
        if sample_len <= 0.0 {
            self.state = VoiceState::Idle;
            return 0.0;
        }

        let left = sample.get_sample_interpolated(self.position, 0);
        let out = if sample.channels == 2 {
            (left + sample.get_sample_interpolated(self.position, 1)) * 0.5
        } else {
            left
        };

        if !self.advance(ratio_override, sample_len) {
            self.state = VoiceState::Idle;
        }
        out
    }
}

impl Default for SampleVoice {
    fn default() -> Self {
        Self::new()
    }
}

/// Polyphonic pool of sample voices.
pub struct VoicePool {
    voices: Vec<SampleVoice>,
    /// Caller-supplied tag per voice (Pulze stores the pad index here so
    /// mute groups can find sibling voices). 0 when untagged.
    tags: Vec<u32>,
    next_age: u64,
}

impl VoicePool {
    pub fn new(max_voices: usize) -> Self {
        let n = max_voices.max(1);
        Self {
            voices: (0..n).map(|_| SampleVoice::new()).collect(),
            tags: vec![0; n],
            next_age: 1,
        }
    }

    /// Trigger `note`; returns the voice index used. `tag` travels with
    /// the voice (e.g. Pulze pad index) for [`VoicePool::choke_tag`].
    pub fn note_on(
        &mut self,
        sample: Arc<SampleData>,
        note: u8,
        velocity: u8,
        params: &VoiceParams,
        host_sample_rate: f32,
        tag: u32,
    ) -> usize {
        let idx = self.find_voice(note);
        let age = self.next_age;
        self.next_age += 1;
        self.voices[idx].trigger(sample, note, velocity, params, host_sample_rate, age);
        self.tags[idx] = tag;
        idx
    }

    pub fn note_off(&mut self, note: u8) {
        for v in &mut self.voices {
            if v.is_playing_note(note) {
                v.release();
            }
        }
    }

    /// Choke every active voice carrying `tag` (mute groups). Skips
    /// `except_idx` so a pad doesn't choke the voice it just triggered.
    pub fn choke_tag(&mut self, tag: u32, except_idx: usize) {
        for (i, v) in self.voices.iter_mut().enumerate() {
            if i != except_idx && v.is_active() && self.tags[i] == tag {
                v.choke();
            }
        }
    }

    pub fn choke_note(&mut self, note: u8) {
        for v in &mut self.voices {
            if v.is_active() && v.note() == note {
                v.choke();
            }
        }
    }

    /// Choke everything (fast fade) — the click-free all-notes-off.
    pub fn choke_all(&mut self) {
        for v in &mut self.voices {
            v.choke();
        }
    }

    /// Hard-stop everything immediately.
    pub fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            v.kill();
        }
    }

    /// retrigger same note → idle → oldest releasing → oldest playing.
    fn find_voice(&self, note: u8) -> usize {
        if let Some(idx) = self.voices.iter().position(|v| v.is_playing_note(note)) {
            return idx;
        }
        if let Some(idx) = self.voices.iter().position(|v| !v.is_active()) {
            return idx;
        }
        let oldest = |pred: &dyn Fn(&SampleVoice) -> bool| {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, v)| pred(v))
                .min_by_key(|(_, v)| v.age())
                .map(|(i, _)| i)
        };
        if let Some(idx) = oldest(&|v: &SampleVoice| v.is_releasing()) {
            return idx;
        }
        oldest(&|v: &SampleVoice| v.is_active()).unwrap_or(0)
    }

    /// Render one stereo frame from all active voices.
    pub fn render(&mut self, host_sample_rate: f32) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for v in &mut self.voices {
            if v.is_active() {
                let (l, r) = v.render(host_sample_rate);
                left += l;
                right += r;
            }
        }
        (left, right)
    }

    /// Render one frame per active voice, delivering each voice's output
    /// with its tag to `sink(tag, l, r)` instead of summing — callers that
    /// need per-tag post-processing (e.g. Pulze's per-pad filters) group
    /// and sum themselves.
    pub fn render_each(&mut self, host_sample_rate: f32, mut sink: impl FnMut(u32, f32, f32)) {
        for (i, v) in self.voices.iter_mut().enumerate() {
            if v.is_active() {
                let (l, r) = v.render(host_sample_rate);
                sink(self.tags[i], l, r);
            }
        }
    }

    pub fn active_count(&self) -> usize {
        self.voices.iter().filter(|v| v.is_active()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_shot_sample() -> Arc<SampleData> {
        // 1 kHz-ish content: a short ramp so interpolation is observable.
        let n = 4800;
        Arc::new(SampleData {
            sample_rate: 48000,
            channels: 1,
            frame_count: n,
            left: (0..n).map(|i| ((i % 100) as f32 / 50.0) - 1.0).collect(),
            right: vec![],
        })
    }

    #[test]
    fn pitch_ratio_octaves() {
        assert!((pitch_ratio(60, 60) - 1.0).abs() < 1e-9);
        assert!((pitch_ratio(72, 60) - 2.0).abs() < 1e-9);
        assert!((pitch_ratio(48, 60) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn trigger_renders_and_one_shot_ignores_release() {
        let mut pool = VoicePool::new(8);
        let params = VoiceParams {
            loop_mode: LoopMode::OneShot,
            ..Default::default()
        };
        pool.note_on(one_shot_sample(), 60, 127, &params, 48000.0, 0);
        // Release must be ignored for one-shots.
        pool.note_off(60);
        let mut peak = 0.0f32;
        for _ in 0..256 {
            let (l, _) = pool.render(48000.0);
            peak = peak.max(l.abs());
        }
        assert!(peak > 0.01, "one-shot keeps sounding, peak={peak}");
        // Runs to the end and goes idle.
        for _ in 0..8192 {
            pool.render(48000.0);
        }
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn choke_fades_to_silence_quickly_without_click() {
        let mut pool = VoicePool::new(8);
        let params = VoiceParams {
            loop_mode: LoopMode::LoopContinuous,
            ..Default::default()
        };
        let idx = pool.note_on(one_shot_sample(), 60, 127, &params, 48000.0, 7);
        // Warm up.
        for _ in 0..64 {
            pool.render(48000.0);
        }
        pool.choke_tag(7, usize::MAX);
        let _ = idx;
        // ~3 ms at 48k = 144 frames; give it 4x that then require silence.
        let mut last = 0.0f32;
        let mut max_jump = 0.0f32;
        for _ in 0..600 {
            let (l, _) = pool.render(48000.0);
            max_jump = max_jump.max((l - last).abs());
            last = l;
        }
        assert_eq!(pool.active_count(), 0, "choked voice goes idle");
        // No hard-cut click: per-frame jump stays well below full scale.
        assert!(max_jump < 0.5, "no click on choke, max_jump={max_jump}");
    }

    #[test]
    fn loop_continuous_wraps_at_loop_points() {
        let mut v = SampleVoice::new();
        let params = VoiceParams {
            loop_mode: LoopMode::LoopContinuous,
            loop_start: Some(100),
            loop_end: Some(200),
            ..Default::default()
        };
        v.trigger(one_shot_sample(), 60, 127, &params, 48000.0, 1);
        for _ in 0..1000 {
            v.render(48000.0);
        }
        assert!(v.is_active(), "continuous loop never ends while held");
    }

    #[test]
    fn stealing_prefers_idle_then_oldest_releasing() {
        let mut pool = VoicePool::new(2);
        let params = VoiceParams::default();
        pool.note_on(one_shot_sample(), 60, 100, &params, 48000.0, 0);
        pool.note_on(one_shot_sample(), 61, 100, &params, 48000.0, 0);
        pool.note_off(60); // 60 releasing (older)
        pool.note_off(61); // 61 releasing (newer)
        let idx = pool.note_on(one_shot_sample(), 62, 100, &params, 48000.0, 0);
        // Voice 0 (note 60) was the oldest releasing — it gets stolen.
        assert_eq!(idx, 0);
    }

    #[test]
    fn tick_pitched_reads_through_sample() {
        let mut v = SampleVoice::new();
        v.trigger(one_shot_sample(), 60, 127, &VoiceParams::default(), 48000.0, 1);
        let a = v.tick_pitched(1.0);
        let b = v.tick_pitched(1.0);
        // Ramp content: consecutive unit-step reads differ.
        assert!(a != b || (a == 0.0 && b != 0.0) || (a != 0.0));
        // Double-speed read ends twice as fast.
        let mut v2 = SampleVoice::new();
        v2.trigger(one_shot_sample(), 60, 127, &VoiceParams::default(), 48000.0, 2);
        let mut n = 0usize;
        while v2.is_active() && n < 100_000 {
            v2.tick_pitched(2.0);
            n += 1;
        }
        assert!((n as i64 - 2400).abs() <= 2, "4800 frames at 2x = ~2400 ticks, got {n}");
    }
}
