//! Moog-style Synthesizer Engine for Synome
//!
//! Features:
//! - 2 oscillators with saw/pulse morphing and hard sync
//! - Moog ladder filter (2/4 pole, LP/BP/HP)
//! - 3 ADSR envelopes (filter, amp, mod)
//! - LFO with 5 waveforms
//! - FM synthesis between oscillators
//! - Polyphonic with voice stealing
//! - Glide/portamento
//! - Effects: chorus, phaser, flanger, delay, reverb
//!
//! The DSP primitives (oscillator, filter, envelope, LFO, noise,
//! effects) live in the shared `plinken-dsp` crate; this module owns
//! what's Synome-specific: the voice, voice management, and the
//! param-driven render loop.
//!
//! Ported near-verbatim from the private monorepo's
//! `plugins/Synome/src/synth.rs` (the clean rewrite); the wclap host
//! drives params through `set_param_value` instead of a snapshot copy.

use crate::params::{Param, PARAM_COUNT};
use plinken_dsp::{
    fast_tanh, midi_to_freq, soft_clip, Delay, Envelope, Lfo, ModulationFx, MoogFilter, Noise,
    OscMode, Oscillator, Reverb,
};
use plinken_sample_core::{LoopMode, SampleData, SampleVoice, VoiceParams};
use std::sync::Arc;

pub const MAX_VOICES: usize = 16;

/// Single synthesizer voice
pub struct Voice {
    pub osc1: Oscillator,
    pub osc2: Oscillator,
    /// Per-osc sample readers for OscMode::Sample — position/loop state
    /// only; env/pan/gain shaping stays with the synth voice.
    pub sample_osc1: SampleVoice,
    pub sample_osc2: SampleVoice,
    pub filter: MoogFilter,
    pub amp_env: Envelope,
    pub filter_env: Envelope,
    pub mod_env: Envelope,
    pub lfo: Lfo,
    pub note: u8,
    pub velocity: f32,
    pub target_freq: f32,
    pub current_freq: f32,
    pub glide_rate: f32,
    pub active: bool,
    sample_rate: f32,
}

impl Voice {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            osc1: Oscillator::new(sample_rate),
            osc2: Oscillator::new(sample_rate),
            sample_osc1: SampleVoice::new(),
            sample_osc2: SampleVoice::new(),
            filter: MoogFilter::new(sample_rate),
            amp_env: Envelope::new(sample_rate),
            filter_env: Envelope::new(sample_rate),
            mod_env: Envelope::new(sample_rate),
            lfo: Lfo::new(sample_rate),
            note: 0,
            velocity: 0.0,
            target_freq: 440.0,
            current_freq: 440.0,
            glide_rate: 1.0,
            active: false,
            sample_rate,
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, legato: bool, glide_ms: f32) {
        self.note = note;
        self.velocity = velocity;
        self.target_freq = midi_to_freq(note as f32);

        if legato && self.active {
            // Legato: don't retrigger envelopes, just glide
        } else {
            self.amp_env.trigger();
            self.filter_env.trigger();
            self.mod_env.trigger();
            if !self.active {
                self.current_freq = self.target_freq;
            }
        }

        // Calculate glide rate
        if glide_ms > 0.0 {
            let glide_samples = glide_ms * self.sample_rate / 1000.0;
            self.glide_rate = 1.0 / glide_samples.max(1.0);
        } else {
            self.glide_rate = 1.0;
            self.current_freq = self.target_freq;
        }

        self.active = true;
    }

    pub fn note_off(&mut self) {
        self.amp_env.release();
        self.filter_env.release();
        self.mod_env.release();
    }

    pub fn is_active(&self) -> bool {
        self.active && self.amp_env.is_active()
    }

    pub fn is_releasing(&self) -> bool {
        self.amp_env.is_releasing()
    }

    pub fn is_playing_note(&self, note: u8) -> bool {
        self.active && self.note == note && !self.amp_env.is_releasing()
    }
}

/// Main Synome synthesizer
pub struct Synome {
    voices: Vec<Voice>,
    delay: Delay,
    reverb: Reverb,
    mod_fx: ModulationFx,
    noise: Noise,
    global_lfo: Lfo,
    sample_rate: f32,
    params: [f32; PARAM_COUNT],
    /// Instrument-wide sample for OscMode::Sample (slot 0 of the PLSP
    /// delivery). Voices read it through per-voice SampleVoice positions.
    sample_slot: Option<Arc<SampleData>>,
}

impl Synome {
    pub fn new(sample_rate: f32) -> Self {
        let voices = (0..MAX_VOICES).map(|_| Voice::new(sample_rate)).collect();

        Self {
            voices,
            delay: Delay::new(sample_rate, 4.0),
            reverb: Reverb::new(sample_rate),
            mod_fx: ModulationFx::new(sample_rate),
            noise: Noise::default(),
            global_lfo: Lfo::new(sample_rate),
            sample_rate,
            params: crate::params::default_values(),
            sample_slot: None,
        }
    }

    /// Install / clear the instrument's sample (OscMode::Sample source).
    /// Sounding voices keep their current Arc until they end.
    pub fn set_sample(&mut self, sample: Option<Arc<SampleData>>) {
        self.sample_slot = sample;
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        for voice in &mut self.voices {
            *voice = Voice::new(sample_rate);
        }
        self.delay = Delay::new(sample_rate, 4.0);
        self.reverb = Reverb::new(sample_rate);
        self.mod_fx = ModulationFx::new(sample_rate);
        self.global_lfo = Lfo::new(sample_rate);
    }

    /// Write one param slot. The value must already be clamped to the
    /// declared range (lib.rs clamps against PARAM_DEFS before calling).
    pub fn set_param_value(&mut self, index: usize, value: f32) {
        if index < PARAM_COUNT {
            self.params[index] = value;
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let vel = velocity as f32 / 127.0;
        let mono = self.params[Param::Mono as usize] > 0.5;
        let legato = self.params[Param::Legato as usize] > 0.5;
        let glide = self.params[Param::Glide as usize];
        let voice_count_idx = self.params[Param::VoiceCount as usize].round() as usize;
        let max_voices = [4, 8, 12, 16][voice_count_idx.min(3)];

        if mono {
            let legato_hold = legato && self.voices[0].active;
            self.voices[0].note_on(note, vel, legato, glide);
            if !legato_hold {
                self.trigger_sample_oscs(0, note);
            }
            if self.params[Param::LfoRetrig as usize] > 0.5 {
                self.voices[0].lfo.reset();
            }
        } else {
            let voice_idx = self.find_voice(note, max_voices);
            self.voices[voice_idx].note_on(note, vel, false, glide);
            self.trigger_sample_oscs(voice_idx, note);
            if self.params[Param::LfoRetrig as usize] > 0.5 {
                self.voices[voice_idx].lfo.reset();
            }
        }
    }

    /// (Re)start a voice's sample readers whenever a sample is loaded —
    /// cheap enough to do unconditionally, so switching an osc to Sample
    /// mode mid-note just works. Envelope/pan/gain of the reader are
    /// bypassed (`tick_pitched`); the synth's own env path shapes it.
    fn trigger_sample_oscs(&mut self, voice_idx: usize, note: u8) {
        let Some(sample) = self.sample_slot.clone() else { return };
        let params = VoiceParams {
            loop_mode: LoopMode::LoopContinuous,
            ..Default::default()
        };
        let v = &mut self.voices[voice_idx];
        v.sample_osc1
            .trigger(sample.clone(), note, 127, &params, self.sample_rate, 0);
        v.sample_osc2
            .trigger(sample, note, 127, &params, self.sample_rate, 0);
    }

    pub fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.is_playing_note(note) {
                voice.note_off();
            }
        }
    }

    #[allow(dead_code)]
    pub fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
            voice.amp_env.reset();
        }
    }

    fn find_voice(&self, note: u8, max_voices: usize) -> usize {
        // Retrigger same note
        for (i, v) in self.voices.iter().enumerate().take(max_voices) {
            if v.is_playing_note(note) {
                return i;
            }
        }

        // Find idle voice
        for (i, v) in self.voices.iter().enumerate().take(max_voices) {
            if !v.is_active() {
                return i;
            }
        }

        // Steal oldest releasing voice
        let mut oldest_idx = 0;
        let mut oldest_samples = 0u32;
        for (i, v) in self.voices.iter().enumerate().take(max_voices) {
            if v.is_releasing() && v.amp_env.stage_samples() > oldest_samples {
                oldest_idx = i;
                oldest_samples = v.amp_env.stage_samples();
            }
        }
        if oldest_samples > 0 {
            return oldest_idx;
        }

        // Steal first voice
        0
    }

    pub fn process(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let p = &self.params;

        // Update global LFO
        self.global_lfo.set_rate(p[Param::LfoRate as usize]);
        self.global_lfo.set_shape(p[Param::LfoShape as usize].round() as i32);
        self.global_lfo.set_delay(p[Param::LfoDelay as usize]);

        let volume = p[Param::Volume as usize];
        let pan = p[Param::Pan as usize];
        let drive = p[Param::MasterDrive as usize];

        // Sample-oscillator setup (constant per block): mode per osc, the
        // frequency the sample plays 1:1 at, and the source/host rate
        // ratio. `osc freq / root freq * rate ratio` is the per-sample
        // playback step — all pitch modulation (coarse/fine/LFO/env/tune)
        // is already inside the osc frequency, so a sample osc glides and
        // vibratos exactly like a BLEP osc.
        let osc1_mode = OscMode::from_index(p[Param::Osc1Mode as usize].round() as u8);
        let osc2_mode = OscMode::from_index(p[Param::Osc2Mode as usize].round() as u8);
        let sample_root_freq = midi_to_freq(p[Param::SampleRootKey as usize].round());
        let sample_rate_ratio = self
            .sample_slot
            .as_ref()
            .map(|s| s.sample_rate as f64 / self.sample_rate as f64);

        for i in 0..output_l.len() {
            let lfo_val = self.global_lfo.process();
            let mut sample = 0.0f32;

            for voice in &mut self.voices {
                if !voice.is_active() {
                    voice.active = false;
                    continue;
                }

                // Update envelope parameters
                voice.amp_env.set_adsr(
                    p[Param::AmpAttack as usize],
                    p[Param::AmpDecay as usize],
                    p[Param::AmpSustain as usize],
                    p[Param::AmpRelease as usize],
                );
                voice.filter_env.set_adsr(
                    p[Param::FilterAttack as usize],
                    p[Param::FilterDecay as usize],
                    p[Param::FilterSustain as usize],
                    p[Param::FilterRelease as usize],
                );
                voice.mod_env.set_adsr(
                    p[Param::ModAttack as usize],
                    p[Param::ModDecay as usize],
                    p[Param::ModSustain as usize],
                    p[Param::ModRelease as usize],
                );

                let amp_env = voice.amp_env.process();
                let filter_env = voice.filter_env.process();
                let mod_env = voice.mod_env.process();

                // Use global or per-voice LFO
                voice.lfo.set_rate(p[Param::LfoRate as usize]);
                voice.lfo.set_shape(p[Param::LfoShape as usize].round() as i32);
                let voice_lfo = voice.lfo.process();
                let lfo = if p[Param::LfoRetrig as usize] > 0.5 { voice_lfo } else { lfo_val };

                // Glide
                if (voice.current_freq - voice.target_freq).abs() > 0.1 {
                    let diff = voice.target_freq - voice.current_freq;
                    voice.current_freq += diff * voice.glide_rate;
                }

                // Calculate base frequency with master tune
                let tune_cents = p[Param::MasterTune as usize];
                let base_freq = voice.current_freq * (2.0f32).powf(tune_cents / 1200.0);

                // OSC1 frequency with modulation
                let osc1_coarse = p[Param::Osc1Coarse as usize];
                let osc1_fine = p[Param::Osc1Fine as usize];
                let mut osc1_pitch_mod = 0.0;
                if p[Param::Osc1ModLfo as usize] > 0.5 {
                    osc1_pitch_mod += lfo * p[Param::PitchModAmount as usize];
                }
                if p[Param::Osc1ModEnv as usize] > 0.5 {
                    osc1_pitch_mod += mod_env * p[Param::PitchModAmount as usize];
                }
                let osc1_freq = base_freq * (2.0f32).powf((osc1_coarse + osc1_fine / 100.0 + osc1_pitch_mod) / 12.0);
                voice.osc1.set_freq(osc1_freq);

                // OSC2 frequency with modulation
                let osc2_coarse = p[Param::Osc2Coarse as usize];
                let osc2_fine = p[Param::Osc2Fine as usize];
                let mut osc2_pitch_mod = 0.0;
                if p[Param::Osc2ModLfo as usize] > 0.5 {
                    osc2_pitch_mod += lfo * p[Param::PitchModAmount as usize];
                }
                if p[Param::Osc2ModEnv as usize] > 0.5 {
                    osc2_pitch_mod += mod_env * p[Param::PitchModAmount as usize];
                }
                let osc2_freq = base_freq * (2.0f32).powf((osc2_coarse + osc2_fine / 100.0 + osc2_pitch_mod) / 12.0);
                voice.osc2.set_freq(osc2_freq);

                // Process OSC2 first (for FM and sync)
                let osc2_shape = p[Param::Osc2Shape as usize];
                let (osc2_out, osc2_wrapped) = if osc2_mode == OscMode::Sample {
                    let out = match sample_rate_ratio {
                        Some(rr) => {
                            let ratio = ((osc2_freq / sample_root_freq) as f64 * rr).max(0.0);
                            voice.sample_osc2.tick_pitched(ratio)
                        }
                        None => 0.0,
                    };
                    // A sample reader has no phase wrap → never a sync source.
                    (out, false)
                } else {
                    voice.osc2.process(osc2_shape, 0.0)
                };

                // FM amount with modulation
                let mut fm_amount = p[Param::Osc1FmAmount as usize];
                if p[Param::Osc1FmEnv as usize] > 0.5 {
                    fm_amount *= mod_env;
                }
                if p[Param::Osc1FmLfo as usize] > 0.5 {
                    fm_amount *= lfo * 0.5 + 0.5;
                }

                // Process OSC1 with FM
                let osc1_shape = p[Param::Osc1Shape as usize];
                let osc1_out = if osc1_mode == OscMode::Sample {
                    match sample_rate_ratio {
                        Some(rr) => {
                            // FM against a sample reader = playback-rate
                            // modulation by the modulator's signal value.
                            let ratio = ((osc1_freq / sample_root_freq) as f64
                                * rr
                                * (1.0 + (osc2_out * fm_amount) as f64))
                                .max(0.0);
                            voice.sample_osc1.tick_pitched(ratio)
                        }
                        None => 0.0,
                    }
                } else {
                    let (o, _) = voice.osc1.process(osc1_shape, osc2_out * fm_amount);
                    o
                };

                // Hard sync: OSC2 resets OSC1 (a sample osc1 rewinds).
                if p[Param::Osc2Sync as usize] > 0.5 && osc2_wrapped {
                    if osc1_mode == OscMode::Sample {
                        voice.sample_osc1.restart();
                    } else {
                        voice.osc1.sync();
                    }
                }

                // Mixer
                let mix_osc1 = p[Param::MixOsc1 as usize];
                let mix_osc2 = p[Param::MixOsc2 as usize];
                let mix_noise = p[Param::MixNoise as usize];
                let noise_type = p[Param::MixNoiseType as usize].round() as i32;

                let noise_sample = if noise_type == 0 {
                    self.noise.white()
                } else {
                    self.noise.pink()
                };

                let mixed = osc1_out * mix_osc1 + osc2_out * mix_osc2 + noise_sample * mix_noise;

                // Filter with modulation
                let filter_base = p[Param::FilterFreq as usize];
                let filter_env_on = p[Param::FilterEnvOn as usize] > 0.5;
                let filter_lfo_on = p[Param::FilterLfoOn as usize] > 0.5;
                let filter_env_amt = p[Param::FilterEnvAmount as usize];
                let filter_lfo_amt = p[Param::FilterLfoAmount as usize];
                let filter_keytrack = p[Param::FilterKeytrack as usize];

                let mut cutoff = filter_base;
                if filter_env_on {
                    cutoff *= (2.0f32).powf(filter_env * filter_env_amt * 4.0);
                }
                if filter_lfo_on {
                    cutoff *= (2.0f32).powf(lfo * filter_lfo_amt * 2.0);
                }
                // Keytracking
                let key_offset = (voice.note as f32 - 60.0) / 12.0;
                cutoff *= (2.0f32).powf(key_offset * filter_keytrack);
                cutoff = cutoff.clamp(20.0, 20000.0);

                voice.filter.set_mode(p[Param::FilterMode as usize].round() as i32);
                voice.filter.set_poles(p[Param::FilterPole as usize].round() as i32);

                let filtered = voice.filter.process(mixed, cutoff, p[Param::FilterRes as usize]);

                // Apply amp envelope and velocity
                sample += filtered * amp_env * voice.velocity;
            }

            // Voice scaling
            sample *= 0.25;

            // Drive/saturation
            if drive > 0.01 {
                let gain = 1.0 + drive * 10.0;
                sample = fast_tanh(sample * gain) / fast_tanh(gain);
            }

            // Effects
            let modfx_type = p[Param::ModFxType as usize].round() as i32;
            if modfx_type > 0 {
                let modfx_rate = p[Param::ModFxRate as usize];
                let modfx_depth = p[Param::ModFxDepth as usize];
                let modfx_mix = p[Param::ModFxMix as usize];
                sample = self.mod_fx.process(sample, modfx_type, modfx_rate, modfx_depth, modfx_mix);
            }

            let reverb_type = p[Param::ReverbType as usize].round() as i32;
            if reverb_type > 0 {
                let fx_time = p[Param::FxTime as usize];
                let fx_amount = p[Param::FxAmount as usize];
                let fx_feedback = p[Param::FxFeedback as usize];
                let fx_damping = p[Param::FxDamping as usize];

                match reverb_type {
                    1 => {
                        sample = self.delay.process(sample, fx_time, fx_feedback, fx_amount);
                    }
                    2 | 3 => {
                        let decay = if reverb_type == 2 { 0.7 } else { 0.85 };
                        sample = self.reverb.process(sample, decay, fx_damping, fx_amount);
                    }
                    _ => {}
                }
            }

            // Volume and panning
            sample *= volume;
            sample = soft_clip(sample);

            let pan_l = ((1.0 - pan) * 0.5).sqrt();
            let pan_r = ((1.0 + pan) * 0.5).sqrt();

            output_l[i] = sample * pan_l;
            output_r[i] = sample * pan_r;
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
            voice.osc1.reset();
            voice.osc2.reset();
            voice.filter.reset();
        }
        self.delay.clear();
        self.reverb.clear();
        self.mod_fx.clear();
        self.global_lfo.reset();
    }
}

impl Default for Synome {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_produces_audio_and_note_off_decays() {
        let mut s = Synome::new(48000.0);
        s.note_on(60, 100);
        let mut l = [0.0f32; 2048];
        let mut r = [0.0f32; 2048];
        s.process(&mut l, &mut r);
        let peak = l.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.01, "audible after note_on, peak={peak}");

        // Release and run past the default 0.3 s amp release: output decays
        // to (near) silence.
        s.note_off(60);
        let mut tail_peak = 0.0f32;
        for _ in 0..12 {
            s.process(&mut l, &mut r);
            tail_peak = l.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        }
        assert!(tail_peak < 0.001, "silent after release, peak={tail_peak}");
    }

    #[test]
    fn sample_osc_plays_and_repitches() {
        let mut s = Synome::new(48000.0);
        // A 480-frame 100 Hz-ish ramp at 48 kHz.
        let n = 4800;
        s.set_sample(Some(Arc::new(SampleData {
            sample_rate: 48000,
            channels: 1,
            frame_count: n,
            left: (0..n).map(|i| ((i % 480) as f32 / 240.0) - 1.0).collect(),
            right: vec![],
        })));
        s.set_param_value(Param::Osc1Mode as usize, 4.0); // Sample
        s.set_param_value(Param::SampleRootKey as usize, 60.0);
        s.set_param_value(Param::MixOsc1 as usize, 1.0);

        // At the root key the reader steps 1:1 — audible output.
        s.note_on(60, 100);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        s.process(&mut l, &mut r);
        let peak = l.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.01, "sample osc audible at root, peak={peak}");

        // An octave up consumes the (looped) sample twice as fast —
        // compare positions indirectly by rendering the same length and
        // checking the outputs differ (different read rates).
        s.all_notes_off();
        s.note_on(72, 100);
        let mut l2 = [0.0f32; 512];
        let mut r2 = [0.0f32; 512];
        s.process(&mut l2, &mut r2);
        let peak2 = l2.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak2 > 0.01, "sample osc audible an octave up");
        assert_ne!(l[100], l2[100], "different pitches read differently");
    }

    #[test]
    fn sample_mode_without_sample_is_silent_not_broken() {
        let mut s = Synome::new(48000.0);
        s.set_param_value(Param::Osc1Mode as usize, 4.0);
        s.set_param_value(Param::MixOsc1 as usize, 1.0);
        s.set_param_value(Param::MixOsc2 as usize, 0.0);
        s.set_param_value(Param::MixNoise as usize, 0.0);
        s.note_on(60, 100);
        let mut l = [0.0f32; 256];
        let mut r = [0.0f32; 256];
        s.process(&mut l, &mut r);
        assert!(l.iter().all(|x| x.abs() < 1e-6), "no sample → silence");
    }

    #[test]
    fn polyphony_and_voice_steal_do_not_panic() {
        let mut s = Synome::new(48000.0);
        for n in 0..32u8 {
            s.note_on(40 + n, 100);
        }
        let mut l = [0.0f32; 256];
        let mut r = [0.0f32; 256];
        s.process(&mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
    }
}
