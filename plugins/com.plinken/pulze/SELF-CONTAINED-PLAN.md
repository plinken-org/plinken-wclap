# Pulze → proper self-contained plugin

## Why
plinken-org plugins run in ANY host (other DAWs, the standalone wclap-host,
Plinken). The current design draws the 4×4 pads in Plinken's app UI
(`PulzePadsPanel`) and streams sample PCM to the plugin over an app-only
side channel. In any other host the pads are missing → the plugin is broken.
A plugin must own its pads and its samples.

## Fundamental constraint
A wclap (wasm) plugin is sandboxed — it cannot read the host's files/OPFS.
So the plugin's **UI** must be the thing that ingests audio (HTML5 file
drop → `FileReader`/`arrayBuffer` → `decodeAudioData`), then hand the decoded
PCM to the plugin over the UI→plugin message channel. Portable sampler
plugins do exactly this.

## Persistence — HOST sample-library interface (supersedes PCM-in-state)
The plugin does NOT embed PCM in its state. Instead the HOST exposes a
content-addressed sample store the plugin calls, and only a small manifest
lives in the project.

WASM host interface the plugin imports (WIT-shaped; realised as wclap `env.*`
host imports, one impl per host — standalone tool, runner, Plinken):
```
interface sample-library {
  list-libraries: func() -> list<library-info>
  open-sample:  func(id: string) -> result<sample-handle, error>
  read-sample:  func(handle, offset: u64, len: u32) -> result<list<u8>, error>
  write-sample: func(id: string, data: list<u8>) -> result<(), error>
  save-manifest: func(id: string, data: list<u8>) -> result<(), error>
  load-manifest: func(id: string) -> result<list<u8>, error>
}
```
- **Upload** (drop a file): plugin UI decodes → hashes → `write-sample(sha, pcm)`;
  host stores content-addressed; pad references `sample_id = "sha256:…"`.
- **Persist**: plugin `save-manifest(instanceId, json)`; host writes it into the
  project. On load, `load-manifest` → for each pad's `sample_id`,
  `open-sample`/`read-sample` to stream the PCM back.
- **Dedup**: same sample across pads/kits = one stored blob (hash key).

Manifest JSON in the project (small; no PCM):
```
{ "library_version":1,
  "kits":[ { "name":"909 Tight",
    "pads":[ {"note":36,"name":"Kick","sample_id":"sha256:abc123",
              "gain":-2.5,"pitch":0,"choke_group":1} ] } ],
  "samples":{ "sha256:abc123":
    {"name":"kick.wav","format":"wav","frames":44100,"channels":1,"sample_rate":44100} } }
```

Pad params (level/tune/pan/filter/mute-group) may still ride PLST params, or
move into the manifest — manifest is cleaner (kit-scoped). TBD in impl.

## Design
Plugin UI (`ui/index.html`, self-contained, no app dependency):
- Bank row A/B/C/D + 4×4 pad grid, Akai note labels, per-pad sample name.
- HTML5 file drag-drop onto a pad → read bytes → `decodeAudioData` (in the
  webview's own AudioContext, offline is fine) → encode PLSP → `send` to the
  plugin (UI→plugin; reuse the existing message pipe the host forwards).
- Click a pad = audition (send a note to the plugin, or just visual — the
  host feeds notes).
- Per-pad controls (level/tune/pan…) as params via `transport.sendSet`.
- The MIDI light stays.

Plugin (`src/lib.rs`):
- `on_message` already reassembles PLSP → `samples[pad]`. Keep it — now the
  bytes come from the plugin's OWN UI instead of the app.
- Add `save_state`/`load_state` (or extend PLST) to serialize `samples[]`
  PCM so the sample survives save/load in ANY host.

Plinken app cleanup (separate, after the plugin works):
- Remove `PulzePadsPanel` + Pulze-specific `PluginSampleService` wiring.
- Plinken hosts Pulze's UI like any other plugin. (Optional nicety later:
  let a Plinken asset-browser drag hand file bytes to the plugin UI, but the
  plugin never depends on it — OS file drop is the portable path.)

## Concrete steps (execution order)
1. **wclap-plugin scaffold — add a custom-state hook** (the enabler):
   - `Plugin::save_extra_state(&self) -> Vec<u8> { Vec::new() }` (default).
   - `Plugin::load_extra_state(&mut self, bytes: &[u8]) {}` (default).
   - `state_save` (lib.rs:1690): after the PLST param block, append the
     plugin's extra bytes. `state_load` (1738): after reading the params,
     pass the remaining stream bytes to `load_extra_state`.
   - New vtable slots + thunks in `init_plugin::<P>`. Existing plugins
     unaffected (defaults are empty).
2. **Pulze plugin — samples in state**:
   - `save_extra_state`: serialize `samples[]` — per non-empty pad:
     `[slot u16][sample_rate u32][channels u8][frames u32][L f32le…][R…]`.
   - `load_extra_state`: rebuild `samples[pad] = Some(Arc::new(SampleData))`.
   - `on_message` already installs from PLSP — keep as-is (now fed by the
     plugin's OWN UI).
3. **Pulze UI — the pads live here**:
   - 4×4 grid + A/B/C/D banks + per-pad label/sample-name + MIDI light.
   - File drop/upload on a pad → `arrayBuffer` → `AudioContext.decodeAudioData`
     → encode PLSP (slot = bank*16+pad) → `window.parent.postMessage(buf)`.
     (iframe-bridge forwards any ArrayBuffer to the plugin → `on_message`.)
   - Per-pad params (level/tune/pan/…) via `transport.sendSet`.
   - Widen window (already 760×240 in publish script).
4. **Remove the app hack** (Plinken, after the plugin works):
   - Drop `PulzePadsPanel` + Pulze-specific `PluginSampleService` paths.
   - Plinken hosts Pulze's UI like any plugin.

## Verify
Load Pulze in `apps/wclap-host` (standalone): pads visible, drop a wav on a
pad, play the note → hear it, save/reload state → still there. That's the
definition of done — no Plinken involved.
