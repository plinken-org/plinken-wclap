// Two jobs:
//
// 1. wasm linker flags — `wclap-host-js` scans the plugin's exports for a
//    WebAssembly.Table to use as the function-table source, then grows it at
//    runtime to install host trampolines. rust-lld's wasm linker defaults to
//    a fixed-size table (max == initial), so we ask for it to be both
//    exported *and* growable.
//
// 2. Codegen — Pulze declares 770 params (2 globals + 64 pads × 12) whose
//    names/modules are distinct `&'static [u8]` NUL-terminated literals, and
//    a 128-entry note→pad table in the classic Akai MPC layout. Both are
//    data, not logic, so they're generated into OUT_DIR/params_gen.rs and
//    `include!`d — see `PAD_PARAM` offsets in src/pads.rs for the id scheme.

use std::fmt::Write as _;

/// Classic Akai MPC bank-A pad→note map (pads A01..A16, bottom row first).
const BANK_A_NOTES: [u8; 16] = [
    37, 36, 42, 82, // A01..A04
    40, 38, 46, 44, // A05..A08
    48, 47, 45, 43, // A09..A12
    49, 55, 51, 53, // A13..A16
];

const PADS: usize = 64;
const PAD_PARAMS: usize = 12;
const PAD_ID_BASE: u32 = 0x0100;
const PAD_ID_STRIDE: u32 = 0x10;

fn pad_notes() -> [u8; PADS] {
    let mut notes = [0u8; PADS];
    notes[..16].copy_from_slice(&BANK_A_NOTES);
    // Banks B–D: placeholder chromatic continuation (ascending from 50,
    // skipping every note bank A uses) until an authoritative MPC bank
    // B–D reference is transcribed — a data-only edit here.
    let used: Vec<u8> = BANK_A_NOTES.to_vec();
    let mut candidate = 50u8;
    for slot in notes.iter_mut().take(PADS).skip(16) {
        while used.contains(&candidate) {
            candidate += 1;
        }
        *slot = candidate;
        candidate += 1;
    }
    notes
}

fn pad_label(pad: usize) -> String {
    let bank = (b'A' + (pad / 16) as u8) as char;
    format!("{}{:02}", bank, pad % 16 + 1)
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-cdylib-link-arg=--export-table");
        println!("cargo:rustc-cdylib-link-arg=--growable-table");
    }
    println!("cargo:rerun-if-changed=build.rs");

    let notes = pad_notes();
    let mut out = String::new();

    // note → pad index (0xFF = unmapped).
    let mut note_to_pad = [0xFFu8; 128];
    for (pad, &n) in notes.iter().enumerate() {
        note_to_pad[n as usize] = pad as u8;
    }
    writeln!(out, "/// Trigger note per pad (classic Akai MPC layout).").unwrap();
    writeln!(out, "pub static PAD_NOTES: [u8; {PADS}] = {notes:?};").unwrap();
    writeln!(out, "/// MIDI note → pad index; 0xFF = unmapped.").unwrap();
    writeln!(out, "pub static NOTE_TO_PAD: [u8; 128] = {note_to_pad:?};").unwrap();

    // Param table. Per-pad offsets (see src/pads.rs accessors):
    //   0 Level, 1 Tune, 2 FineTune, 3 Pan, 4 Attack, 5 Decay,
    //   6 FilterType, 7 Cutoff, 8 Resonance, 9 MuteGroup, 10 OneShot,
    //   11 RootKey  (12..=15 reserved)
    struct P {
        name: &'static str,
        min: f64,
        max: f64,
        default: f64,
        stepped: bool,
    }
    let pad_params = [
        // Unity by default — a dropped kick plays at 0 dB; the USER turns
        // it down, the plugin never pre-attenuates.
        P { name: "Level", min: 0.0, max: 1.0, default: 1.0, stepped: false },
        P { name: "Tune", min: -36.0, max: 36.0, default: 0.0, stepped: false },
        P { name: "Fine Tune", min: -100.0, max: 100.0, default: 0.0, stepped: false },
        P { name: "Pan", min: -1.0, max: 1.0, default: 0.0, stepped: false },
        P { name: "Attack", min: 0.001, max: 2.0, default: 0.001, stepped: false },
        P { name: "Decay", min: 0.01, max: 8.0, default: 4.0, stepped: false },
        P { name: "Filter Type", min: 0.0, max: 3.0, default: 0.0, stepped: true },
        P { name: "Cutoff", min: 20.0, max: 20000.0, default: 20000.0, stepped: false },
        P { name: "Resonance", min: 0.0, max: 1.0, default: 0.0, stepped: false },
        P { name: "Mute Group", min: 0.0, max: 32.0, default: 0.0, stepped: true },
        P { name: "One Shot", min: 0.0, max: 1.0, default: 1.0, stepped: true },
        P { name: "Root Key", min: 0.0, max: 127.0, default: 0.0, stepped: true },
    ];
    assert_eq!(pad_params.len(), PAD_PARAMS);

    let total = 2 + PADS * PAD_PARAMS;
    writeln!(out, "pub const PARAM_COUNT: usize = {total};").unwrap();
    writeln!(
        out,
        "pub static PARAM_DEFS: [wclap_plugin::ParamDef; {total}] = ["
    )
    .unwrap();
    let auto = "wclap_plugin::PARAM_IS_AUTOMATABLE";
    let stepped = "wclap_plugin::PARAM_IS_AUTOMATABLE | wclap_plugin::PARAM_IS_STEPPED";
    let hidden_stepped =
        "wclap_plugin::PARAM_IS_STEPPED | wclap_plugin::PARAM_IS_HIDDEN";
    writeln!(
        out,
        "    wclap_plugin::ParamDef {{ id: 0, flags: {auto}, name: b\"Master Level\\0\", module: b\"\\0\", min: 0f64, max: 1f64, default: 1f64 }},"
    )
    .unwrap();
    // PadCount is host/UI plumbing, not a knob: hidden + stepped, persists
    // the dynamic pad count through the ordinary PLST param dump.
    writeln!(
        out,
        "    wclap_plugin::ParamDef {{ id: 1, flags: {hidden_stepped}, name: b\"Pad Count\\0\", module: b\"\\0\", min: 1f64, max: {PADS}f64, default: 16f64 }},"
    )
    .unwrap();
    for pad in 0..PADS {
        let label = pad_label(pad);
        for (off, p) in pad_params.iter().enumerate() {
            let id = PAD_ID_BASE + pad as u32 * PAD_ID_STRIDE + off as u32;
            let flags = if p.stepped { stepped } else { auto };
            // Root Key defaults to the pad's own trigger note.
            let default = if p.name == "Root Key" {
                notes[pad] as f64
            } else {
                p.default
            };
            writeln!(
                out,
                "    wclap_plugin::ParamDef {{ id: {id}, flags: {flags}, name: b\"Pad {label} {}\\0\", module: b\"Pads/{label}\\0\", min: {}f64, max: {}f64, default: {}f64 }},",
                p.name, p.min, p.max, default
            )
            .unwrap();
        }
    }
    writeln!(out, "];").unwrap();

    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("params_gen.rs");
    std::fs::write(dest, out).unwrap();
}
