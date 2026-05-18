# Designer

Web-based layout tool for Plinken plugin UIs. Lives at `/designer` on
`apps/site` (SvelteKit 5, runes). Drop widgets onto a canvas, position
and resize, type the endpoint name each widget binds to, save the
result as an `index.html` file that drops into a plugin's `ui/`
directory and rides the existing `bundle-wclap.mjs` pipeline into the
tarball.

## V1 scope

**In:**

- Palette of widget kinds from `@plinken/widget-lib`
- Drag from palette to canvas; reposition + resize on canvas
- Property panel for the selected widget (free-form `endpoint` string
  + display options: label, format, colour, kind-specific options)
- Mock wclap-host connection so widgets render and animate during
  layout (per-kind defaults — see table below)
- Save as `index.html` download; re-open by file-picker round-trip

**Out (deferred to V2+):**

- Patch upload / endpoint inventory
- Type-filtered endpoint dropdowns + autocomplete
- Validation of bindings against a real patch (“stale binding”
  warnings)
- Multi-file output (separate JS / CSS per design)
- Direct write to `plugins/<name>/ui/` (R2 store / HMR socket / API)
- Multi-select, copy/paste, undo/redo
- Snap-to-grid + alignment guides
- Skin / theme picker beyond `<plinken-background skin="…">`

## Layout

Three panels, single SvelteKit route:

```
+----------+------------------------+------------+
| Palette  |        Canvas          | Properties |
| (left)   |       (centre)         |  (right)   |
+----------+------------------------+------------+
```

- **Palette** — widget kinds with thumbnails. Click or drag to add to
  the canvas at the cursor / canvas centre.
- **Canvas** — fixed-size container (`<div class="plinken-ui">`,
  configurable W×H). Chrome lives in a `<plinken-background>` widget
  filling the container; placed widgets are siblings, absolute-
  positioned via CSS `transform: translate(x, y)` and explicit
  `width` / `height`. Each widget internally renders an `<svg
  viewBox>` so resizing is free. Click selects (8 resize handles +
  outline); drag body to move, drag handle to resize. Esc clears,
  Delete removes.
- **Properties** — bound to selection. Fields: see the attribute
  table below; values write straight back to attributes on the
  selected DOM node.

## Widget composition

Widgets are `HTMLElement` custom elements extending `PlinkenWidget`,
each rendering its own `<svg viewBox>` internally. Positioned via
absolute CSS, **not** as `<g>` groups inside one master SVG — custom
elements don't drop into the SVG namespace cleanly, and the
HTMLElement route keeps `PlinkenWidget extends HTMLElement` honest.

Resize is free: `preserveAspectRatio="none"` for stretch widgets
(background, faders, meters, spectrum, waveform), aspect-locked for
round widgets (knob, xy-pad).

## Binding model

Each widget exposes one attribute: `endpoint="paramName"`. The
property panel writes a free-form string; **no validation against any
patch happens in V1**. Typos surface at plugin load time when
`requestStatusUpdate()` returns no matching parameter — the widget
logs and skips its `onMeta` render.

Dynamic binding (V2): accept a built `.wclap.tar.gz`, instantiate it
through `wclap-host-js`, pull the endpoint inventory via
`requestStatusUpdate()` (same call the runtime widgets use), and
layer autocomplete + type-filtering + stale-binding warnings on top.
The on-disk file format does not change — V2 is intelligence, not a
schema break.

## Widget attributes

Three groups of attributes per widget:

**Binding (required):**

| Attr       | Value                                                  |
|------------|--------------------------------------------------------|
| `endpoint` | patch parameter name (free-form string in V1)          |

**Cmajor-parity (optional, mirror patch annotations):**

| Attr   | Cmajor annotation | Notes                                       |
|--------|-------------------|---------------------------------------------|
| `min`  | `min:`            | numeric lower bound                         |
| `max`  | `max:`            | numeric upper bound                         |
| `init` | `init:`           | default value                               |
| `step` | `step:`           | quantisation step                           |
| `unit` | `unit:`           | display label (`"dB"`, `"Hz"`, `"ms"`, …)  |
| `text` | `text:`           | pipe-separated enum values for switch/dropdown (`"Sine\|Saw\|Square\|Noise"`) |

**UI-only (no patch analog):**

| Attr      | Value                                                       |
|-----------|-------------------------------------------------------------|
| `scaling` | `"log"` \| `"lin"` (default `"lin"`) — knob/fader response curve |
| `format`  | display override, e.g. `"{v:.1f} kHz"` when patch says Hz   |
| `label`   | text shown by the widget (defaults to endpoint name)        |
| `x`, `y`  | pixel position on the canvas                                |
| `w`, `h`  | pixel size                                                  |

**Conflict rule:** at runtime, `meta` from the patch wins. The
Cmajor-parity attributes fill gaps only when the patch doesn't
supply the corresponding field. That keeps the patch authoritative
and the HTML in sync without edits when annotations change. The
UI-only attributes are always honoured — they have no patch source
to compete with.

**Canonical example:**

```html
<plinken-knob endpoint="cutoff"
              min="20" max="20000" init="1000" step="1"
              unit="Hz" scaling="log"
              x="40" y="60" w="56" h="56" label="CUTOFF"/>
```

During design (no patch loaded): the mock conn builds `meta` from
these attributes — widget renders with `0 – 20 kHz` range, log
response, Hz readout. After the plugin wires up against a patch
that declares `cutoff [[ min: 20, max: 20000, init: 1000, unit:
"Hz" ]]`: patch `meta` supersedes, nothing visible changes.

## Mock connection

The designer ships a `MockConnection` implementing the methods
`PlinkenWidget` calls:

```js
requestStatusUpdate()           // → { parameters: […] } built from placed widgets
addParameterListener(ep, cb)    // registers cb; ticker emits fake values
removeParameterListener(ep, cb)
requestParameterValue(ep)       // → fires cb with init
sendParameterGestureStart(ep)   // no-op (logged)
sendParameterGestureEnd(ep)
sendEventOrValue(ep, value)
```

The synthesised `parameters` array reads each placed widget's
Cmajor-parity attributes first; anything missing falls back to
per-kind defaults:

| Widget   | min  | max  | init | unit | step |
|----------|------|------|------|------|------|
| knob     |  0   |  1   | 0.5  | —    | —    |
| fader    | -60  |  0   | -12  | dB   | 0.1  |
| toggle   |  0   |  1   |  0   | —    | 1    |
| switch   |  0   |  3   |  0   | —    | 1    |
| dropdown |  0   |  3   |  0   | —    | 1    |
| button   |  0   |  1   |  0   | —    | 1    |
| meter    | -60  |  0   | -60  | dB   | —    |
| spectrum |  0   |  1   |  0   | —    | —    |
| waveform | -1   |  1   |  0   | —    | —    |
| xy-pad   |  0   |  1   | 0.5  | —    | —    |
| led      |  0   |  1   |  0   | —    | 1    |
| keyboard |  0   | 127  |  60  | —    | 1    |

A `Preview` toggle on the canvas drives the mock's value ticker so
meters / spectra / waveforms animate during layout.

## Output format

Single `index.html`, written to `plugins/<name>/ui/index.html`:

```html
<!doctype html>
<meta charset="utf-8">
<link rel="stylesheet" href="../widget-lib/widget-lib.css">
<script type="module" src="../widget-lib/index.mjs"></script>

<div class="plinken-ui" data-w="480" data-h="320">
  <plinken-background skin="organ"/>
  <plinken-knob   endpoint="cutoff" min="20" max="20000" init="1000" step="1"
                  unit="Hz" scaling="log"
                  x="40" y="60" w="56" h="56" label="CUTOFF"/>
  <plinken-fader  endpoint="vol"    min="-60" max="0" init="-12"
                  unit="dB"
                  x="120" y="40" w="20" h="200" label="VOL"/>
  <plinken-toggle endpoint="bypass" x="200" y="60" w="40" h="20" label="BYPASS"/>
</div>

<script type="module">
  import { mountAll } from '../widget-lib/index.mjs';
  mountAll();   // reads x/y/w/h, applies transforms, calls setConnection(conn) on each widget
</script>
```

`mountAll()` reads `x` / `y` / `w` / `h` attributes off each placed
element, applies them as CSS `transform: translate(…)` + `width` /
`height`, acquires the host connection from the iframe parent
(existing wclap-host flow), and walks the tree calling
`setConnection(conn)` on each `PlinkenWidget` instance.

## Save

V1: `Save` button triggers a download of `index.html`; user drops it
into `plugins/<name>/ui/`. `Open` is a `<input type="file">` pick
that parses the same file back into canvas state. No state
persistence between sessions otherwise.

V2 candidates: R2-backed project store, HMR socket that writes
directly to the workspace, multi-design library per account.

## Bundler integration (follow-up, not designer scope)

When the first plugin (organ / piano / auto-panner) starts using
`widget-lib`, `scripts/bundle-wclap.mjs` needs a small change to
ship `widget-lib/` into the tarball under `widget-lib/` — same
pattern as the existing `widgets/` walk, skipping `package.json` and
`node_modules/`. Flag this on the first plugin rewrite PR; the
designer work itself doesn't touch the bundler.

## Sequencing

Smallest spike that proves the pipeline end-to-end:

1. `widget-lib/widget-base.mjs` (✓ already merged).
2. One concrete widget (`plinken-knob`) extending `PlinkenWidget`,
   rendering an SVG, honouring `min`/`max`/`init`/`unit`/`scaling`
   attributes as patch-meta fallbacks.
3. `apps/site/src/routes/designer/+page.svelte` — palette with one
   entry, canvas, drag-place-resize, property panel writing back to
   the attribute set.
4. `MockConnection` in the designer module, reading widgets'
   attributes to synthesise `meta`.
5. Save → download `index.html`; verify the file loads in a plain
   browser tab with a mock conn and the knob renders + animates.
6. Add remaining widgets one at a time as they're authored.
