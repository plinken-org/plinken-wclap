//! Pulze — Plinken WCLAP drum machine (MPC-style pad instrument).
//!
//! Phase A scaffold: the CLAP entry-point ABI is wired through the shared
//! `wclap-plugin` crate; this module renders silence. Planned shape:
//! a **dynamic** pad set (pads are added as needed, grouped in 4×4 banks
//! like the MPC's A/B/C/D) using the **classic Akai MPC note layout** —
//! bank A pads 1–16 map to MIDI notes
//! 37 36 42 82 / 40 38 46 44 / 48 47 45 43 / 49 55 51 53.
//! Each pad is a synthesized drum voice (kick / snare / hats / percussion)
//! first, later up to 4 sample layers per pad mirroring the MPC program
//! pad structure (velocity ranges, level/tune/pan/decay/filter, mute
//! groups) so Akai `.xpm`/`.pgm` patch import is a field-for-field map.
//! The UI puts the bank selector above the 4×4 grid like the hardware, and
//! bank note offsets match the MPC's — an Akai controller over MIDI plays
//! it directly. Notes reach the plugin through the CLAP note port declared
//! below once `wclap-plugin` exposes the process event queue (same
//! dependency as Synome's DSP phase).

#![no_std]

use wclap_plugin::{init_plugin, silence, Plugin, PluginDef, ProcessCtx, ProcessStatus};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.pulze\0",
    name: b"Pulze\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.0.1\0",
    description: b"MPC-style drum machine \xe2\x80\x94 dynamic pads, Akai note layout, synthesized kits.\0",
    features: &[b"instrument\0", b"drum-machine\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: None,
};

struct Pulze;

impl Plugin for Pulze {
    fn new() -> Self {
        Pulze
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        silence(ctx);
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<Pulze>(&PLUGIN_DEF);
}
