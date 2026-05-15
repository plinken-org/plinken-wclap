//! Synome — Plinken WCLAP synthesizer plugin.
//!
//! Phase A scaffold: the CLAP entry-point ABI is wired through the shared
//! `wclap-plugin` crate; this module renders silence. The DSP — voice pool,
//! BLEP oscillators, ADSR, filter — gets ported in subsequent phases from
//! the existing private repo at
//! `/Volumes/Music/TECH41/gitroot/plinken-synome/plugin/src/lib/rust/synth/`.

#![no_std]

use wclap_plugin::{init_plugin, silence, Plugin, PluginDef, ProcessCtx, ProcessStatus};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.synome\0",
    name: b"Synome\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.0.1\0",
    description: b"Polyphonic synthesizer with anti-aliased BLEP oscillators.\0",
    features: &[b"instrument\0", b"synthesizer\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
};

struct Synome;

impl Plugin for Synome {
    fn new() -> Self {
        Synome
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        silence(ctx);
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<Synome>(&PLUGIN_DEF);
}
