//! Synome — Plinken WCLAP synthesizer plugin.
//!
//! Moog-style polysynth: 2 oscillators (saw/pulse morph, FM, hard sync),
//! Moog ladder filter, 3 ADSRs, LFO, mod FX + delay/reverb, 16-voice
//! pool with stealing, glide/legato/mono. DSP primitives come from the
//! shared `plinken-dsp` crate; notes arrive through the CLAP note port
//! via the `wclap-plugin` event-queue hooks.

mod params;
mod synth;

use params::{PARAM_COUNT, PARAM_DEFS};
use plinken_sample_core::{AssembleResult, SampleAssembler, SampleData};
use std::sync::Arc;
use synth::Synome;
use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.synome\0",
    name: b"Synome\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.2\0",
    description: b"Moog-style polysynth \xe2\x80\x94 2 osc (FM, hard sync), ladder filter, 3 ADSRs, LFO, FX.\0",
    features: &[b"instrument\0", b"synthesizer\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

struct SynomePlugin {
    /// DSP engine — allocated in `activate` once the sample rate is known.
    synth: Option<Synome>,
    /// Authoritative param values, kept valid before AND after activate so
    /// state replay ahead of `activate` (hosts do this) survives, and so
    /// `get_param`/state save never need the DSP side.
    values: [f64; PARAM_COUNT],
    /// PLSP chunk reassembly for the instrument's sample slot (0).
    assembler: SampleAssembler,
    /// Sample kept plugin-side so a (re)activate can re-install it into
    /// the freshly built synth.
    sample: Option<Arc<SampleData>>,
}

impl SynomePlugin {
    fn clamped(id: u32, value: f64) -> Option<f64> {
        let def = PARAM_DEFS.get(id as usize)?;
        Some(value.clamp(def.min, def.max))
    }
}

impl Plugin for SynomePlugin {
    fn new() -> Self {
        SynomePlugin {
            synth: None,
            values: core::array::from_fn(|i| PARAM_DEFS[i].default),
            assembler: SampleAssembler::new(),
            sample: None,
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let mut s = Synome::new(sample_rate as f32);
        for (i, v) in self.values.iter().enumerate() {
            s.set_param_value(i, *v as f32);
        }
        s.set_sample(self.sample.clone());
        self.synth = Some(s);
    }

    fn reset(&mut self) {
        if let Some(s) = &mut self.synth {
            s.reset();
        }
    }

    fn note_on(&mut self, _time: u32, _channel: i16, key: i16, velocity: f64) {
        let Some(s) = &mut self.synth else { return };
        if !(0..=127).contains(&key) {
            return;
        }
        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
        s.note_on(key as u8, vel.max(1));
    }

    fn note_off(&mut self, _time: u32, _channel: i16, key: i16, _velocity: f64) {
        let Some(s) = &mut self.synth else { return };
        if (0..=127).contains(&key) {
            s.note_off(key as u8);
        }
    }

    fn note_choke(&mut self, _time: u32, _channel: i16, key: i16) {
        let Some(s) = &mut self.synth else { return };
        if key < 0 {
            s.all_notes_off();
        } else if key <= 127 {
            s.note_off(key as u8);
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(v) = Self::clamped(id, value) else { return };
        self.values[id as usize] = v;
        if let Some(s) = &mut self.synth {
            s.set_param_value(id as usize, v as f32);
        }
    }

    fn get_param(&self, id: u32) -> f64 {
        self.values.get(id as usize).copied().unwrap_or(0.0)
    }

    fn params() -> &'static [ParamDef] {
        &PARAM_DEFS
    }

    fn on_message(&mut self, bytes: &[u8]) -> bool {
        match self.assembler.push(bytes) {
            AssembleResult::Complete { slot: 0, sample } => {
                let arc = Arc::new(sample);
                self.sample = Some(arc.clone());
                if let Some(s) = &mut self.synth {
                    s.set_sample(Some(arc));
                }
                true
            }
            AssembleResult::Cleared { slot: 0 } => {
                self.sample = None;
                if let Some(s) = &mut self.synth {
                    s.set_sample(None);
                }
                true
            }
            // Synome has exactly one sample slot; other slots are ignored
            // but still consumed (they're PLSP traffic, not param CBOR).
            AssembleResult::Complete { .. }
            | AssembleResult::Cleared { .. }
            | AssembleResult::Progress { .. }
            | AssembleResult::Error => true,
            AssembleResult::NotMine => false,
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let Some(s) = &mut self.synth else {
            silence(ctx);
            return ProcessStatus::Continue;
        };
        let Some(out) = ctx.stereo_out() else {
            silence(ctx);
            return ProcessStatus::Continue;
        };
        s.process(out.output_l, out.output_r);
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<SynomePlugin>(&PLUGIN_DEF);
}
