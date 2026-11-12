//! FORMA — Plinken WCLAP synthesizer.
//!
//! Two slots, A and B. Each slot plays its own source sample through one
//! of two engines: **Grain** (key-tracked audio-rate granular synthesis)
//! or **Spectral** (the source re-synthesised as four static sounds with
//! envelopes — see `analysis.rs`). The slots are crossfaded or
//! ring-modulated and placed in the stereo field in mid/side terms.
//!
//! Sources arrive from the plugin's own UI as PLSP chunks (the same
//! protocol Pulze and Synome use) and are saved inside the plugin state,
//! so a project reloads in any host without the original files.

// The NMF and layer maths index several small matrices by the same
// counter; iterator chains would hide which axis is which.
#![allow(clippy::needless_range_loop)]

pub mod analysis;
pub mod engine;
pub mod grain;
pub mod params;
pub mod spectral;

use analysis::{Analyzer, Source, SpectralModel, LAYERS};
use engine::{Engine, SlotContent, SLOTS};
use params::{index_of, PARAM_COUNT, PARAM_DEFS};
use plinken_sample_core::{AssembleResult, SampleAssembler};
use std::sync::Arc;
use wclap_plugin::{init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"com.plinken.forma\0",
    name: b"FORMA\0",
    vendor: b"Plinken\0",
    url: b"https://plinken.org\0",
    version: b"0.1.1\0",
    description: b"Two-slot synthesizer: granular and spectral-resynthesis engines built from your own samples.\0",
    features: &[b"instrument\0", b"synthesizer\0", b"sampler\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

/// Extra-state magic, little-endian "FRM1".
const STATE_MAGIC: [u8; 4] = *b"FRM1";
/// Points per curve in the UI overview.
const OVERVIEW_POINTS: usize = 128;
/// Seconds between playhead/progress messages to the UI.
const UI_TICK_SECONDS: f32 = 0.05;
/// The UI's request for the slot overviews (CBOR text "forma?").
const UI_REQUEST: &[u8] = b"\x66forma?";
const MSG_OVERVIEW: u8 = 1;
const MSG_TICK: u8 = 2;

struct Slot {
    source: Arc<Source>,
    /// False while the slot still plays its factory source.
    imported: bool,
    model: Option<Arc<SpectralModel>>,
    analyzer: Option<Analyzer>,
    root_hz: f32,
    /// The UI's picture of this slot is out of date.
    dirty: bool,
}

impl Slot {
    fn with_source(source: Source, imported: bool) -> Slot {
        let source = Arc::new(source);
        Slot { analyzer: Some(Analyzer::new(source.clone())), source, imported, model: None, root_hz: 0.0, dirty: true }
    }

    fn content(&self) -> SlotContent {
        SlotContent { source: Some(self.source.clone()), model: self.model.clone(), root_hz: self.root_hz }
    }
}

pub struct FormaPlugin {
    /// DSP engine — built in `activate` once the sample rate is known.
    engine: Option<Engine>,
    /// Authoritative param values, valid before and after `activate`
    /// (hosts replay state ahead of it).
    values: [f64; PARAM_COUNT],
    assembler: SampleAssembler,
    slots: [Slot; SLOTS],
    sample_rate: f32,
    ui_countdown: f32,
}

impl FormaPlugin {
    fn install(&mut self, slot: usize, source: Source, imported: bool) {
        self.slots[slot] = Slot::with_source(source, imported);
        if let Some(e) = &mut self.engine {
            e.set_content(slot, self.slots[slot].content());
        }
    }

    /// Advance the pending analyses by an amount of work that scales with
    /// the block length, so the cost per second of audio is constant.
    fn advance_analysis(&mut self, frames: usize) {
        let units = (frames / 128).max(1);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(an) = &mut slot.analyzer else { continue };
            let had_root = an.root_hz().is_some();
            let mut done = false;
            for _ in 0..units {
                if an.step() {
                    done = true;
                    break;
                }
            }
            let root = an.root_hz();
            let mut model = None;
            if done {
                model = an.take_model().map(Arc::new);
                slot.model = model.clone();
                slot.analyzer = None;
                slot.dirty = true;
            }
            if let Some(hz) = root {
                slot.root_hz = hz;
                if model.is_some() || !had_root {
                    if let Some(e) = &mut self.engine {
                        e.set_analysis(i, hz, model);
                    }
                }
            }
        }
    }

    /// Finish every pending analysis now (offline callers and tests).
    pub fn finish_analysis(&mut self) {
        while self.slots.iter().any(|s| s.analyzer.is_some()) {
            self.advance_analysis(128);
        }
    }

    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        self.advance_analysis(out_l.len());
        match &mut self.engine {
            Some(e) => e.process(out_l, out_r),
            None => {
                out_l.fill(0.0);
                out_r.fill(0.0);
            }
        }
    }

    /// `{"forma": bytes}` — the one plugin→UI message shape FORMA adds.
    fn wrap(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 10);
        out.extend_from_slice(&[0xa1, 0x65, b'f', b'o', b'r', b'm', b'a', 0x59]);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Picture of a slot for the UI: status, root, length, the source's
    /// level curve and the four layer envelopes.
    fn overview(&self, index: usize) -> Vec<u8> {
        let slot = &self.slots[index];
        let mut p = Vec::with_capacity(16 + OVERVIEW_POINTS * (1 + LAYERS));
        let progress = slot.analyzer.as_ref().map(|a| a.progress()).unwrap_or(1.0);
        let pitched = slot.model.as_ref().is_some_and(|m| m.pitched);
        p.push(MSG_OVERVIEW);
        p.push(index as u8);
        p.push(if slot.model.is_some() { 2 } else { 1 });
        p.push((progress * 100.0) as u8);
        p.push(pitched as u8 | (slot.imported as u8) << 1);
        p.extend_from_slice(&slot.root_hz.to_le_bytes());
        p.extend_from_slice(&slot.source.seconds().to_le_bytes());
        match &slot.model {
            Some(m) => {
                let t = m.frames;
                let at = |i: usize| i * t / OVERVIEW_POINTS;
                let span = |i: usize| at(i)..(at(i + 1)).max(at(i) + 1).min(t);
                for i in 0..OVERVIEW_POINTS {
                    let peak = m.peaks[span(i)].iter().fold(0.0f32, |a, v| a.max(*v));
                    p.push((peak.min(1.0) * 255.0) as u8);
                }
                let top = m.env.iter().fold(1e-9f32, |a, v| a.max(*v));
                for k in 0..LAYERS {
                    let env = &m.env[k * t..(k + 1) * t];
                    for i in 0..OVERVIEW_POINTS {
                        let v = env[span(i)].iter().fold(0.0f32, |a, v| a.max(*v));
                        p.push((v / top * 255.0) as u8);
                    }
                }
            }
            None => p.resize(p.len() + OVERVIEW_POINTS * (1 + LAYERS), 0),
        }
        Self::wrap(&p)
    }

    fn tick_message(&self) -> Vec<u8> {
        let mut p = vec![MSG_TICK];
        for i in 0..SLOTS {
            let pos = self.engine.as_ref().map(|e| e.playhead(i)).unwrap_or(0.0);
            p.push((pos.clamp(0.0, 1.0) * 255.0) as u8);
        }
        p.push(self.engine.as_ref().map(|e| e.active_voices()).unwrap_or(0) as u8);
        for slot in &self.slots {
            p.push(slot.analyzer.as_ref().map(|a| (a.progress() * 100.0) as u8).unwrap_or(100));
        }
        Self::wrap(&p)
    }
}

impl Plugin for FormaPlugin {
    fn new() -> Self {
        FormaPlugin {
            engine: None,
            values: core::array::from_fn(|i| PARAM_DEFS[i].default),
            assembler: SampleAssembler::new(),
            slots: core::array::from_fn(|i| Slot::with_source(Source::builtin(i as u32), false)),
            sample_rate: 48_000.0,
            ui_countdown: 0.0,
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        let mut e = Engine::new(self.sample_rate);
        for (i, v) in self.values.iter().enumerate() {
            e.set_param(i, *v as f32);
        }
        for (i, slot) in self.slots.iter().enumerate() {
            e.set_content(i, slot.content());
        }
        self.engine = Some(e);
    }

    fn reset(&mut self) {
        if let Some(e) = &mut self.engine {
            e.reset();
        }
    }

    fn note_on(&mut self, _time: u32, _channel: i16, key: i16, velocity: f64) {
        if let (Some(e), 0..=127) = (&mut self.engine, key) {
            e.note_on(key as u8, velocity as f32);
        }
    }

    fn note_off(&mut self, _time: u32, _channel: i16, key: i16, _velocity: f64) {
        if let (Some(e), 0..=127) = (&mut self.engine, key) {
            e.note_off(key as u8);
        }
    }

    fn note_choke(&mut self, _time: u32, _channel: i16, key: i16) {
        let Some(e) = &mut self.engine else { return };
        if key < 0 {
            e.all_notes_off();
        } else if key <= 127 {
            e.note_off(key as u8);
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(i) = index_of(id) else { return };
        let v = value.clamp(PARAM_DEFS[i].min, PARAM_DEFS[i].max);
        self.values[i] = v;
        if let Some(e) = &mut self.engine {
            e.set_param(i, v as f32);
        }
    }

    fn get_param(&self, id: u32) -> f64 {
        index_of(id).map(|i| self.values[i]).unwrap_or(0.0)
    }

    fn params() -> &'static [ParamDef] {
        &PARAM_DEFS
    }

    fn on_message(&mut self, bytes: &[u8]) -> bool {
        if bytes == UI_REQUEST {
            for slot in self.slots.iter_mut() {
                slot.dirty = true;
            }
            return true;
        }
        match self.assembler.push(bytes) {
            AssembleResult::Complete { slot, sample } if (slot as usize) < SLOTS => {
                // Too short to play: keep what the slot has.
                if let Some(source) = Source::from_sample(&sample) {
                    self.install(slot as usize, source, true);
                }
                true
            }
            AssembleResult::Cleared { slot } if (slot as usize) < SLOTS => {
                self.install(slot as usize, Source::builtin(slot), false);
                true
            }
            AssembleResult::NotMine => false,
            // PLSP traffic for other slots, progress and errors: consumed.
            _ => true,
        }
    }

    fn save_extra_state(&self) -> Vec<u8> {
        let mut out = STATE_MAGIC.to_vec();
        for slot in &self.slots {
            if !slot.imported {
                out.push(0);
                continue;
            }
            out.push(1);
            out.extend_from_slice(&slot.source.sr.to_le_bytes());
            out.extend_from_slice(&(slot.source.data.len() as u32).to_le_bytes());
            for v in &slot.source.data {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    fn load_extra_state(&mut self, bytes: &[u8]) {
        let Some(mut rest) = bytes.strip_prefix(&STATE_MAGIC) else { return };
        for i in 0..SLOTS {
            let Some((&has, tail)) = rest.split_first() else { return };
            rest = tail;
            if has == 0 {
                if self.slots[i].imported {
                    self.install(i, Source::builtin(i as u32), false);
                }
                continue;
            }
            if rest.len() < 8 {
                return;
            }
            let sr = f32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
            let frames = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
            rest = &rest[8..];
            let Some(byte_len) = frames.checked_mul(4).filter(|n| *n <= rest.len()) else { return };
            let data = rest[..byte_len].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            rest = &rest[byte_len..];
            if let Some(source) = Source::from_mono(sr, data) {
                self.install(i, source, true);
            }
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        match ctx.stereo_out() {
            Some(out) => self.render(out.output_l, out.output_r),
            None => {
                self.advance_analysis(frames);
                silence(ctx);
            }
        }
        for i in 0..SLOTS {
            if self.slots[i].dirty {
                self.slots[i].dirty = false;
                ctx.send_to_ui(&self.overview(i));
            }
        }
        self.ui_countdown -= frames as f32 / self.sample_rate;
        if self.ui_countdown <= 0.0 {
            self.ui_countdown = UI_TICK_SECONDS;
            ctx.send_to_ui(&self.tick_message());
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<FormaPlugin>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;
    use params as P;

    /// A PLSP message exactly as the UI encodes it (see ui/index.html).
    fn plsp(slot: u32, sr: u32, channels: u32, total: u32, start: u32, pcm: &[f32]) -> Vec<u8> {
        let frames = pcm.len() as u32 / channels;
        let payload = 32 + pcm.len() * 4;
        let mut m = vec![0xa1, 0x63, b's', b'm', b'p', 0x5a];
        m.extend_from_slice(&(payload as u32).to_be_bytes());
        for v in [0x504c_5350u32, 1, slot, sr, channels, total, start, frames] {
            m.extend_from_slice(&v.to_le_bytes());
        }
        for v in pcm {
            m.extend_from_slice(&v.to_le_bytes());
        }
        m
    }

    fn saw(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| 2.0 * ((hz * i as f32 / 44_100.0) % 1.0) - 1.0).collect()
    }

    fn active() -> FormaPlugin {
        let mut p = FormaPlugin::new();
        p.activate(48_000.0, 128);
        p
    }

    fn render(p: &mut FormaPlugin, frames: usize) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            p.render(cl, cr);
        }
        l
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn plays_out_of_the_box_while_the_analysis_runs_in_the_background() {
        let mut p = active();
        p.note_on(0, 0, 60, 0.8);
        // Slot A (grain) needs no analysis: sound from the first block.
        let first = render(&mut p, 2400);
        assert!(rms(&first) > 0.005, "silent at start: {}", rms(&first));
        assert!(p.slots.iter().any(|s| s.analyzer.is_some()), "analysis finished implausibly fast");
        // Each block does a bounded slice; it completes within seconds.
        render(&mut p, 48_000 * 6);
        assert!(p.slots.iter().all(|s| s.analyzer.is_none() && s.model.is_some()));
    }

    #[test]
    fn chunked_stereo_import_replaces_the_source_and_is_analysed() {
        let mut p = active();
        let mono = saw(196.0, 22_050);
        let stereo: Vec<f32> = mono.iter().chain(mono.iter()).copied().collect();
        // Planar stereo in two chunks.
        let half = 11_025;
        let chunk = |a: usize, b: usize| -> Vec<f32> {
            stereo[a..b].iter().chain(stereo[22_050 + a..22_050 + b].iter()).copied().collect()
        };
        assert!(p.on_message(&plsp(1, 44_100, 2, 22_050, 0, &chunk(0, half))));
        assert!(!p.slots[1].imported);
        assert!(p.on_message(&plsp(1, 44_100, 2, 22_050, half as u32, &chunk(half, 22_050))));
        assert!(p.slots[1].imported && p.slots[1].model.is_none());
        assert_eq!(p.slots[1].source.data.len(), 22_050);
        p.finish_analysis();
        let m = p.slots[1].model.as_ref().unwrap();
        assert!(m.pitched && (1200.0 * (m.root_hz / 196.0).log2()).abs() < 20.0, "root {}", m.root_hz);
        // The imported source is what slot B now plays.
        p.set_param(P::FADE, 1.0);
        p.note_on(0, 0, 55, 1.0);
        assert!(rms(&render(&mut p, 9600)) > 0.01);
        // Other messages are not swallowed.
        assert!(!p.on_message(b"\x65ready"));
        assert!(p.on_message(UI_REQUEST));
    }

    #[test]
    fn clearing_a_slot_restores_the_factory_source() {
        let mut p = active();
        p.on_message(&plsp(0, 44_100, 1, 4410, 0, &saw(220.0, 4410)));
        assert!(p.slots[0].imported);
        p.on_message(&plsp(0, 44_100, 1, 0, 0, &[]));
        assert!(!p.slots[0].imported);
        assert_eq!(p.slots[0].source.data.len(), Source::builtin(0).data.len());
    }

    #[test]
    fn state_round_trips_imported_sources_and_renders_identically() {
        let mut a = active();
        a.on_message(&plsp(0, 44_100, 1, 8820, 0, &saw(147.0, 8820)));
        a.finish_analysis();
        let blob = a.save_extra_state();

        let mut b = FormaPlugin::new();
        // Hosts restore state before activating.
        b.load_extra_state(&blob);
        b.activate(48_000.0, 128);
        b.finish_analysis();
        assert!(b.slots[0].imported && !b.slots[1].imported);
        assert_eq!(a.slots[0].source.data.len(), b.slots[0].source.data.len());

        for p in [&mut a, &mut b] {
            p.set_param(P::SLOT_A + P::ENGINE, 1.0);
            p.note_on(0, 0, 50, 1.0);
        }
        let (ra, rb) = (render(&mut a, 4800), render(&mut b, 4800));
        assert!(rms(&ra) > 0.01);
        let diff = ra.iter().zip(&rb).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(diff < 1e-3, "restored instance differs by {diff}");
    }

    #[test]
    fn truncated_or_foreign_state_is_ignored_safely() {
        let mut a = active();
        a.on_message(&plsp(0, 44_100, 1, 4410, 0, &saw(220.0, 4410)));
        let blob = a.save_extra_state();
        // The last byte is slot B's flag, so cut inside slot A's PCM.
        for cut in [0, 3, 4, 5, 9, 13, blob.len() - 2] {
            let mut b = active();
            b.load_extra_state(&blob[..cut]);
            assert!(!b.slots[0].imported, "cut {cut}");
        }
        let mut b = active();
        b.load_extra_state(b"NOPE\x01\x00\x00");
        assert!(!b.slots[0].imported);
    }

    #[test]
    fn params_clamp_and_ignore_unknown_ids() {
        let mut p = active();
        p.set_param(P::FADE, 7.0);
        assert_eq!(p.get_param(P::FADE), 1.0);
        p.set_param(P::SLOT_B + P::TUNE, -99.0);
        assert_eq!(p.get_param(P::SLOT_B + P::TUNE), -24.0);
        p.set_param(9999, 1.0);
        assert_eq!(p.get_param(9999), 0.0);
    }

    #[test]
    fn ui_messages_have_the_documented_shape() {
        let mut p = active();
        let o = p.overview(1);
        assert_eq!(&o[..8], &[0xa1, 0x65, b'f', b'o', b'r', b'm', b'a', 0x59]);
        let len = u16::from_be_bytes([o[8], o[9]]) as usize;
        assert_eq!(len, o.len() - 10);
        assert_eq!(len, 13 + OVERVIEW_POINTS * (1 + LAYERS));
        assert_eq!((o[10], o[11], o[12]), (MSG_OVERVIEW, 1, 1));
        p.finish_analysis();
        let o = p.overview(1);
        assert_eq!(o[12], 2);
        assert!(o[23..].iter().any(|v| *v > 0));
        let t = p.tick_message();
        assert_eq!(t[10], MSG_TICK);
        assert_eq!(t.len(), 10 + 6);
    }
}

#[cfg(test)]
mod cost {
    use super::*;
    use std::time::Instant;

    /// Not a pass/fail test: prints what one audio block costs. Run with
    /// `cargo test -p com-plinken-forma --release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn report_block_costs() {
        // Longest source the plugin keeps: the worst case for one step.
        let n = (analysis::MAX_SOURCE_SECONDS * 48_000.0) as usize;
        let data = (0..n).map(|i| (i as f32 * 0.013).sin() * (i as f32 * 0.000_7).cos()).collect();
        let mut an = Analyzer::new(Arc::new(Source::from_mono(48_000.0, data).unwrap()));
        let (mut worst, mut total, mut steps) = (0.0f64, 0.0f64, 0u32);
        loop {
            let t = Instant::now();
            let done = an.step();
            let dt = t.elapsed().as_secs_f64();
            worst = worst.max(dt);
            total += dt;
            steps += 1;
            if done {
                break;
            }
        }
        println!("analysis: {steps} steps, worst {:.0} us, total {:.0} ms", worst * 1e6, total * 1e3);

        let mut p = FormaPlugin::new();
        p.activate(48_000.0, 128);
        p.finish_analysis();
        p.set_param(params::SLOT_A + params::G_SIZE, 16.0);
        for k in 0..engine::VOICES as i16 {
            p.note_on(0, 0, 40 + 3 * k, 1.0);
        }
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        let t = Instant::now();
        let blocks = 2000;
        for _ in 0..blocks {
            p.render(&mut l, &mut r);
        }
        let per = t.elapsed().as_secs_f64() / blocks as f64;
        println!("render: {} voices, {:.0} us per 128-frame block ({:.1}% of real time)", engine::VOICES, per * 1e6, per / (128.0 / 48_000.0) * 100.0);
    }
}
