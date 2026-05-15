//! Vocal Limiter — Plinken WCLAP audio-effect plugin.
//!
//! MVP scaffold: stereo-linked lookahead limiter with hardcoded defaults
//! (ceiling −1 dBFS, attack 5 ms, release 50 ms). Drive + automation come
//! once `clap.params` lands in `wclap-plugin`.

use fundsp::audiounit::AudioUnit;
use fundsp::prelude32::limiter_stereo;
use wclap_plugin::{init_plugin, silence, Plugin, PluginDef, ProcessCtx, ProcessStatus, StereoIo};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.vocal-limiter\0",
    name: b"Vocal Limiter\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.0.1\0",
    description: b"Lookahead peak limiter tuned for vocals.\0",
    features: &[b"audio-effect\0", b"limiter\0", b"mastering\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
};

// Defaults — promoted to clap.params once the extension lands.
const CEILING_DB: f32 = -1.0;
const ATTACK_S: f32 = 0.005;
const RELEASE_S: f32 = 0.050;

/// 10^(dB / 20) — linear gain factor from a decibel value.
fn db_to_amp(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

struct VocalLimiter {
    // Boxed-trait so we don't have to spell out fundsp's nested generic
    // `An<Limiter<U2>>` type at every storage site. Per-tick vtable cost
    // is negligible against the limiter's own envelope math.
    limiter: Box<dyn AudioUnit>,
    ceiling_gain: f32,
}

impl Plugin for VocalLimiter {
    fn new() -> Self {
        Self {
            limiter: Box::new(limiter_stereo(ATTACK_S, RELEASE_S)),
            ceiling_gain: db_to_amp(CEILING_DB),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.limiter.set_sample_rate(sample_rate);
    }

    fn reset(&mut self) {
        self.limiter.reset();
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let Some(io) = ctx.stereo_io() else {
            silence(ctx);
            return ProcessStatus::Continue;
        };
        let StereoIo {
            input_l,
            input_r,
            output_l,
            output_r,
        } = io;
        let frames = input_l.len();
        let mut buf_in = [0.0_f32; 2];
        let mut buf_out = [0.0_f32; 2];
        for f in 0..frames {
            buf_in[0] = input_l[f];
            buf_in[1] = input_r[f];
            self.limiter.tick(&buf_in, &mut buf_out);
            output_l[f] = buf_out[0] * self.ceiling_gain;
            output_r[f] = buf_out[1] * self.ceiling_gain;
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<VocalLimiter>(&PLUGIN_DEF);
}
