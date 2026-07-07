//! Pulze — Plinken WCLAP drum machine (MPC-style pad instrument).
//!
//! Dynamic pads in 4×4 banks on the classic Akai MPC note layout (bank
//! selector above the grid mirrors the hardware, so an MPC controller
//! plays it directly). Each pad is a sample player (plinken-sample-core
//! voices) with level / tune / pan / AD envelope / per-pad Moog filter and
//! MPC-style mute groups (closed hat chokes open hat). Samples arrive from
//! the host as PLSP chunks over the webview byte channel (`on_message` →
//! `SampleAssembler`); pad topology persists as ordinary params through
//! the scaffold's PLST state dump, sample/asset references persist
//! app-side and are re-sent on load. The pad model mirrors the MPC program
//! pad structure so Akai `.xpm`/`.pgm` import maps field-for-field.

mod pads;

use pads::{pad_value_index, PadParam, PADS, PARAM_DEFS};
use plinken_dsp::{soft_clip, MoogFilter};
use plinken_sample_core::{
    AssembleResult, LoopMode, SampleAssembler, SampleData, SampleVoice, VoiceParams,
};
use std::sync::Arc;
use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.pulze\0",
    name: b"Pulze\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.1\0",
    description: b"MPC-style drum machine \xe2\x80\x94 dynamic pads, Akai note layout, sample playback.\0",
    features: &[b"instrument\0", b"drum-machine\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

const MAX_VOICES: usize = 32;

struct PulzePlugin {
    /// Flat param values, indexed like `PARAM_DEFS` (single source of
    /// truth for state save/load and the UI snapshot).
    values: Vec<f64>,
    /// Per-pad sample content, delivered via PLSP chunks. Index = pad.
    samples: Vec<Option<Arc<SampleData>>>,
    voices: Vec<SampleVoice>,
    /// Pad index each voice is sounding (parallel to `voices`).
    voice_pad: Vec<u32>,
    /// Monotonic trigger stamp for oldest-voice stealing.
    next_age: u64,
    /// Per-pad stereo Moog filters, allocated at activate (sample rate).
    filters: Vec<(MoogFilter, MoogFilter)>,
    assembler: SampleAssembler,
    sample_rate: f32,
    /// Per-frame pad accumulation scratch (pad L/R sums before the pad
    /// filter). Allocated once; only touched pads are cleared per frame.
    pad_accum: Vec<(f32, f32)>,
    touched: Vec<u32>,
}

impl PulzePlugin {
    #[inline]
    fn value(&self, pad: usize, p: PadParam) -> f64 {
        self.values[pad_value_index(pad, p)]
    }

    fn pad_count(&self) -> usize {
        (self.values[1].round() as usize).clamp(1, PADS)
    }

    fn trigger_pad(&mut self, pad: usize, key: i16, velocity: f64) {
        let Some(sample) = self.samples[pad].clone() else {
            return;
        };
        let one_shot = self.value(pad, PadParam::OneShot) > 0.5;
        let decay = self.value(pad, PadParam::Decay) as f32;
        let params = VoiceParams {
            root_key: (self.value(pad, PadParam::RootKey).round() as i64).clamp(0, 127) as u8,
            tune_cents: (self.value(pad, PadParam::Tune) * 100.0
                + self.value(pad, PadParam::FineTune)) as f32,
            gain: self.value(pad, PadParam::Level) as f32,
            pan: self.value(pad, PadParam::Pan) as f32,
            loop_mode: if one_shot { LoopMode::OneShot } else { LoopMode::NoLoop },
            loop_start: None,
            loop_end: None,
            attack: self.value(pad, PadParam::Attack) as f32,
            // AD shape: no sustain plateau; the same time constant closes
            // held (NoLoop) pads on release.
            decay,
            sustain: 0.0,
            release: decay,
        };
        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8;

        // Mute group: choke every sounding voice whose pad shares this
        // pad's group (the new voice is triggered after, so it survives).
        let group = self.value(pad, PadParam::MuteGroup).round() as u32;
        if group != 0 {
            for i in 0..self.voices.len() {
                let vp = self.voice_pad[i] as usize;
                if self.voices[i].is_active()
                    && vp < PADS
                    && self.values[pad_value_index(vp, PadParam::MuteGroup)].round() as u32
                        == group
                {
                    self.voices[i].choke();
                }
            }
        }

        let idx = self.find_voice(key as u8);
        let age = self.next_age;
        self.next_age += 1;
        self.voices[idx].trigger(sample, key as u8, vel, &params, self.sample_rate, age);
        self.voice_pad[idx] = pad as u32;
    }

    /// retrigger same note → idle → oldest releasing → oldest playing.
    fn find_voice(&self, note: u8) -> usize {
        if let Some(i) = self.voices.iter().position(|v| v.is_playing_note(note)) {
            return i;
        }
        if let Some(i) = self.voices.iter().position(|v| !v.is_active()) {
            return i;
        }
        let oldest = |pred: &dyn Fn(&SampleVoice) -> bool| {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, v)| pred(v))
                .min_by_key(|(_, v)| v.age())
                .map(|(i, _)| i)
        };
        if let Some(i) = oldest(&|v: &SampleVoice| v.is_releasing()) {
            return i;
        }
        oldest(&|v: &SampleVoice| v.is_active()).unwrap_or(0)
    }
}

impl Plugin for PulzePlugin {
    fn new() -> Self {
        PulzePlugin {
            values: PARAM_DEFS.iter().map(|d| d.default).collect(),
            samples: vec![None; PADS],
            voices: (0..MAX_VOICES).map(|_| SampleVoice::new()).collect(),
            voice_pad: vec![0; MAX_VOICES],
            next_age: 1,
            filters: Vec::new(),
            assembler: SampleAssembler::new(),
            sample_rate: 48000.0,
            pad_accum: vec![(0.0, 0.0); PADS],
            touched: Vec::with_capacity(MAX_VOICES),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.filters = (0..PADS)
            .map(|_| {
                (
                    MoogFilter::new(self.sample_rate),
                    MoogFilter::new(self.sample_rate),
                )
            })
            .collect();
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.kill();
        }
        for (l, r) in &mut self.filters {
            l.reset();
            r.reset();
        }
    }

    fn note_on(&mut self, _time: u32, _channel: i16, key: i16, velocity: f64) {
        if let Some(pad) = pads::pad_for_note(key, self.pad_count()) {
            self.trigger_pad(pad, key, velocity);
        }
    }

    fn note_off(&mut self, _time: u32, _channel: i16, key: i16, _velocity: f64) {
        // One-shot voices ignore release inside SampleVoice; gated (NoLoop)
        // pads enter their AD release here.
        if !(0..=127).contains(&key) {
            return;
        }
        for v in &mut self.voices {
            if v.is_playing_note(key as u8) {
                v.release();
            }
        }
    }

    fn note_choke(&mut self, _time: u32, _channel: i16, key: i16) {
        if key < 0 {
            for v in &mut self.voices {
                v.choke();
            }
        } else {
            for v in &mut self.voices {
                if v.is_active() && v.note() == key as u8 {
                    v.choke();
                }
            }
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(idx) = pads::param_index(id) else { return };
        let def = &PARAM_DEFS[idx];
        self.values[idx] = value.clamp(def.min, def.max);
    }

    fn get_param(&self, id: u32) -> f64 {
        pads::param_index(id).map_or(0.0, |i| self.values[i])
    }

    fn params() -> &'static [ParamDef] {
        &PARAM_DEFS
    }

    fn on_message(&mut self, bytes: &[u8]) -> bool {
        match self.assembler.push(bytes) {
            AssembleResult::Complete { slot, sample } => {
                let pad = slot as usize;
                if pad < PADS {
                    self.samples[pad] = Some(Arc::new(sample));
                }
                true
            }
            AssembleResult::Cleared { slot } => {
                let pad = slot as usize;
                if pad < PADS {
                    // Voices still holding the old Arc play it out; the
                    // slot just stops accepting new triggers.
                    self.samples[pad] = None;
                }
                true
            }
            AssembleResult::Progress { .. } | AssembleResult::Error => true,
            AssembleResult::NotMine => false,
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let Some(out) = ctx.stereo_out() else {
            silence(ctx);
            return ProcessStatus::Continue;
        };
        let master = self.values[0] as f32;
        let frames = out.output_l.len().min(out.output_r.len());

        for i in 0..frames {
            // Group per-voice output by pad so pads with a filter get
            // filtered as one signal (not per voice).
            self.touched.clear();
            let (accum, touched, voices, voice_pad) = (
                &mut self.pad_accum,
                &mut self.touched,
                &mut self.voices,
                &self.voice_pad,
            );
            for (vi, v) in voices.iter_mut().enumerate() {
                if v.is_active() {
                    let (l, r) = v.render(self.sample_rate);
                    let pad = voice_pad[vi];
                    let slot = &mut accum[pad as usize];
                    if slot.0 == 0.0 && slot.1 == 0.0 && !touched.contains(&pad) {
                        touched.push(pad);
                    }
                    slot.0 += l;
                    slot.1 += r;
                }
            }

            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            for &pad in self.touched.iter() {
                let (mut l, mut r) = self.pad_accum[pad as usize];
                self.pad_accum[pad as usize] = (0.0, 0.0);
                let p = pad as usize;
                let ftype = self.values[pad_value_index(p, PadParam::FilterType)].round() as i32;
                if ftype > 0 {
                    if let Some((fl, fr)) = self.filters.get_mut(p) {
                        let cutoff = self.values[pad_value_index(p, PadParam::Cutoff)] as f32;
                        let res = self.values[pad_value_index(p, PadParam::Resonance)] as f32;
                        // MoogFilter modes: 0 LP, 1 BP, 2 HP — param value
                        // 1..3 maps to mode 0..2 (0 = filter off).
                        fl.set_mode(ftype - 1);
                        fr.set_mode(ftype - 1);
                        l = fl.process(l, cutoff, res);
                        r = fr.process(r, cutoff, res);
                    }
                }
                sum_l += l;
                sum_r += r;
            }

            out.output_l[i] = soft_clip(sum_l * master);
            out.output_r[i] = soft_clip(sum_r * master);
        }
        // Zero any tail the host asked for beyond our min() guard.
        for i in frames..out.output_l.len() {
            out.output_l[i] = 0.0;
        }
        for i in frames..out.output_r.len() {
            out.output_r[i] = 0.0;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<PulzePlugin>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kick_sample() -> Vec<u8> {
        // Encode a tiny PLSP transfer for pad 1 (note 36): 64 frames of a
        // decaying pulse.
        let frames = 64usize;
        let pcm: Vec<f32> = (0..frames)
            .map(|i| (1.0 - i as f32 / frames as f32) * if i % 2 == 0 { 0.9 } else { -0.9 })
            .collect();
        encode_plsp(1, 48000, 1, frames as u32, 0, &pcm)
    }

    fn encode_plsp(
        slot: u32,
        sr: u32,
        ch: u32,
        total: u32,
        start: u32,
        left: &[f32],
    ) -> Vec<u8> {
        let payload_len = 32 + left.len() * 4;
        let mut out = vec![0xa1, 0x63, b's', b'm', b'p', 0x5a];
        out.extend_from_slice(&(payload_len as u32).to_be_bytes());
        for v in [0x504C_5350u32, 1, slot, sr, ch, total, start, left.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in left {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn make_plugin() -> PulzePlugin {
        let mut p = PulzePlugin::new();
        p.activate(48000.0, 128);
        p
    }

    #[test]
    fn note_36_triggers_pad_a02_after_sample_delivery() {
        let mut p = make_plugin();
        assert!(p.on_message(&kick_sample()));
        p.note_on(0, 0, 36, 1.0);
        // Render manually through the voices (no ProcessCtx off-wasm):
        let mut peak = 0.0f32;
        for _ in 0..64 {
            for v in p.voices.iter_mut() {
                if v.is_active() {
                    let (l, _) = v.render(48000.0);
                    peak = peak.max(l.abs());
                }
            }
        }
        assert!(peak > 0.01, "pad sounds after delivery, peak={peak}");
    }

    #[test]
    fn note_without_sample_is_silent_and_safe() {
        let mut p = make_plugin();
        p.note_on(0, 0, 36, 1.0);
        assert!(p.voices.iter().all(|v| !v.is_active()));
    }

    #[test]
    fn unmapped_note_is_ignored() {
        let mut p = make_plugin();
        assert!(p.on_message(&kick_sample()));
        p.note_on(0, 0, 127, 1.0); // no pad maps note 127
        assert!(p.voices.iter().all(|v| !v.is_active()));
    }

    #[test]
    fn mute_group_chokes_sibling_pad() {
        let mut p = make_plugin();
        // Deliver samples to pads 2 (closed hat, note 42) and 6 (open hat,
        // note 46), both in mute group 1.
        let hat: Vec<f32> = vec![0.5; 4800];
        let msg2 = encode_plsp(2, 48000, 1, 4800, 0, &hat);
        let msg6 = encode_plsp(6, 48000, 1, 4800, 0, &hat);
        assert!(p.on_message(&msg2));
        assert!(p.on_message(&msg6));
        p.set_param(pads::PAD_ID_BASE + 2 * pads::PAD_ID_STRIDE + PadParam::MuteGroup as u32, 1.0);
        p.set_param(pads::PAD_ID_BASE + 6 * pads::PAD_ID_STRIDE + PadParam::MuteGroup as u32, 1.0);

        p.note_on(0, 0, 46, 1.0); // open hat rings
        let open_idx = p
            .voices
            .iter()
            .position(|v| v.is_active())
            .expect("open hat voice");
        p.note_on(0, 0, 42, 1.0); // closed hat chokes it
        assert!(
            p.voices[open_idx].is_releasing(),
            "open-hat voice must be choked by the closed hat"
        );
        // The new closed-hat voice itself is playing, not choked.
        let playing = p.voices.iter().filter(|v| v.is_playing_note(42)).count();
        assert_eq!(playing, 1);
    }

    #[test]
    fn pad_count_gates_bank_b() {
        let mut p = make_plugin();
        let note_b01 = pads::PAD_NOTES[16];
        let msg = encode_plsp(16, 48000, 1, 64, 0, &vec![0.5; 64]);
        assert!(p.on_message(&msg));
        p.note_on(0, 0, note_b01 as i16, 1.0); // PadCount default 16 → gated
        assert!(p.voices.iter().all(|v| !v.is_active()));
        p.set_param(1, 32.0);
        p.note_on(0, 0, note_b01 as i16, 1.0);
        assert!(p.voices.iter().any(|v| v.is_active()));
    }

    #[test]
    fn state_roundtrip_values_by_id() {
        let mut p = make_plugin();
        let id = pads::PAD_ID_BASE + 5 * pads::PAD_ID_STRIDE + PadParam::Level as u32;
        p.set_param(id, 0.25);
        assert!((p.get_param(id) - 0.25).abs() < 1e-9);
        // Clamps to range.
        p.set_param(id, 9.0);
        assert!((p.get_param(id) - 1.0).abs() < 1e-9);
    }
}
