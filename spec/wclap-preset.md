# WCLAP Preset — an NKS-aligned, CLAP-native preset format

Status: **draft spec** (2026-07-07).

A preset format for WCLAP plugins that is **as close to NKS (Native Kontrol
Standard) as possible**, so that a Plinken/plinken-org library and a third-party
NKS library (e.g. u-he Diva's official NKS presets) can be **indexed and queried
the same way**. The canonical on-disk/on-CDN artifact **is an NKSF file**
(`.nksf` / `.nksfx`); this spec pins how a WCLAP plugin produces and consumes one
using CLAP-native mechanisms (`clap.state`, `clap.params`, `clap.remote-controls`).

Non-goal: Komplete Kontrol / Maschine **hardware** recognition. That needs NI's
partner program (registered plugin IDs, `.nicnt`, `komplete.db3`, `NI Resources`
artwork). We produce NKSF *files*; we do not register with NI's desktop software.

---

## 1. The canonical artifact: NKSF

NKSF is a RIFF container so third-party tooling and our own indexer parse one path.

- Magic `RIFF` · u32 LE size · form type **`NIKS`**.
- Sub-chunks in order: **`NISI`** · **`NICA`** · **`PLID`** · **`PCHK`**.
- Each: 4-byte FourCC · u32 LE size · body · pad to even.
- `NISI`/`NICA`/`PLID` bodies: `u32` version (`1`) then a **MessagePack** map.
- `PCHK` body: **raw** plugin state.
- `.nksf` = instrument (`deviceType:"INST"`), `.nksfx` = effect (`deviceType:"FX"`).

### NISI — summary (queryable metadata)
| key | type | notes |
|---|---|---|
| `name` | string | preset display name |
| `author` | string | |
| `vendor` | string | maker / library |
| `comment` | string | |
| `deviceType` | string | `INST` \| `FX` \| `MIDI` |
| `bankchain` | array<string> | ≤3 levels, e.g. `["Diva","Bass",""]` |
| `types` | array<array<string>> | category paths, e.g. `[["Bass","Analog"]]` |
| `modes` | array<string> | e.g. `["Arpeggiated"]` |
| `uuid` | string | stable preset id |

### NICA — controller assignments (parameter pages)
Key **`ni8`**: array of pages; each page = 8 control slots:
```
{ id, name, section, autoname: bool, vflag: bool }
```
Maps 1:1 onto CLAP **`clap.remote-controls`** (8 params/page). Empty slot: `name:""`.

### PLID — plugin id (how the loader resolves the plugin)
`{ "VST.magic": <i32> }` and/or `{ "VST3.uid": [<i32>×4] }`. WCLAP additions
(back-compatible extra keys, ignored by NI tooling):
```
"CLAP.id": "com.plinken.organ",   // reverse-DNS CLAP id → resolve a WCLAP build
"WCLAP.assetId": "<registry id>"  // optional fast path to the .wclap.tar.gz
```
Resolution order for playback: `CLAP.id` → `WCLAP.assetId` → `VST3.uid`/`VST.magic`.

### PCHK — plugin state (raw)
For a WCLAP plugin this is the **`clap.state`** blob our scaffold already emits:
```
u32 "PLST" · u32 version · u32 count · count×(u32 param_id, f64 value)
[ + optional save_extra_state() tail — e.g. sampler PCM refs ]
```
For an imported third-party preset it is that vendor's opaque chunk (only usable
when that plugin is actually hosted).

### Previews & artwork (library layout, outside the RIFF)
- Audio preview: `.ogg` named `<preset>.ogg` in a `.previews/` sibling dir.
  Effects have none. We **render previews offline through the WCLAP host** — we
  host the plugin, so no third-party VST hosting is needed.
- Bank artwork: plain image blobs referenced from the bank index. We do **not**
  emit NI's `NI Resources/image/...` exact-px files (HW-only).

---

## 2. What a WCLAP plugin must implement

The host writes/reads NKSF; the plugin only needs standard CLAP extensions.

| CLAP ext | Role in NKSF | Scaffold status |
|---|---|---|
| `clap.state` | save/load → **PCHK** | **implemented** (`crates/wclap-plugin`) |
| `clap.params` | param ids/ranges the state references | **implemented** |
| `clap.remote-controls` | 8/page → **NICA `ni8`** | **new** (add to scaffold + both hosts) |
| `clap.preset-load` | apply a preset by reference | **new** (optional but preferred) |

`Plugin::save_extra_state()` / `load_extra_state()` (`crates/wclap-plugin/src/lib.rs`)
already carry non-param state past the PLST block — banks-with-samples ride it.

Host work lands in **both** copies in lockstep: `plinken-org/crates/wclap-host`
and `Plinken/wclap-host`.

---

## 3. Packaging factory presets in `.wclap.tar.gz`

`scripts/bundle-wclap.mjs` gains an optional `presets/` tree; `build-shelf.mjs`
surfaces it in `shelf.json`.

```
module.wasm
plugin.json
ui/… widgets/…
presets/                      ← new
  <Bank>/<Preset>.nksf
  <Bank>/.previews/<Preset>.nksf.ogg
```

`plugin.json` (manifest_version 1) gains an optional field:
```json
"presets": { "dir": "presets", "count": 24, "format": "nksf" }
```
Absent field = no factory presets (fully back-compatible). The host indexes the
tree by parsing each NKSF's NISI/PLID exactly as it would a user upload — one
code path for factory, user, and imported-third-party presets.

---

## 4. Why NKSF-native (query parity with Diva et al.)

Because the stored artifact *is* NKSF, indexing our own plugins and an ingested
u-he Diva NKS library is the same parse. A query like "all Diva analog basses"
or "all arpeggiated instrument presets across every synth" is one filter over
NISI fields (`vendor`, `bankchain`, `types`, `modes`, `deviceType`) regardless of
who authored the preset. Our own WCLAP plugins emit real `.nksf` so they are
first-class in that same catalog.

See `Plinken/docs/sound-library-nks.md` for the online library (API + R2 + D1),
the D1 index schema, and the ingest/query pipeline.
