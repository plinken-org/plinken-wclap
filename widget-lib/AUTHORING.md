# Authoring a widget

The rules below are normative. If a widget breaks one, the catalogue
page will look wrong and the designer's mock conn will misbehave —
both easy to notice on review.

## File layout

One widget per file at `widget-lib/<kind>.mjs`. The file:

1. Imports `PlinkenWidget` from `./widget-base.mjs`
2. Declares a subclass
3. Calls `customElements.define('plinken-<kind>', …)` at module top

No widget code outside that file. Shared helpers (range-mapping, log
scaling, format utilities) live in `widget-lib/utils.mjs`.

## Tag naming

- Custom element name: `plinken-<kind>` (kebab-case, mandatory hyphen)
- Class name: `Plinken<Kind>` (PascalCase) — `PlinkenKnob`, `PlinkenXyPad`
- File name: `<kind>.mjs` (lowercase, no prefix)

## Lifecycle

Extend `PlinkenWidget`. The base class handles:

- Pulling the endpoint name from the `endpoint` attribute
- One-shot `requestStatusUpdate()` to fetch `meta`
- Subscribing to live values via `addParameterListener`
- Tearing down on `disconnectedCallback`

The subclass overrides:

- `onMeta(meta)` — called once with the endpoint's annotations
  (`min` / `max` / `init` / `step` / `unit` / `text`). Build the
  initial DOM here; this is the render entry point. Treat the patch
  `meta` as authoritative; only fall back to widget attributes for
  fields the patch doesn't supply.
- `onValue(v)` — called on every live update. Cheap path: write to
  CSS variables / SVG `transform` / `textContent`, no DOM allocation.

For UI → DSP writes, call `this.write(value)` for drag updates and
`this.write(value, true)` for one-shot gestures (reset on double-click,
default value snap). Drag start/end bracketing is a known gap in
the base class — a future patch adds `beginGesture()` /
`endGesture()`; until then, widgets that drag continuously can
emit a single `write(v, true)` on pointerup as a one-frame gesture.

Do not override the constructor unless you need to set up state
that predates `setConnection()`. The base class is happy with the
default constructor.

## Data feeds beyond parameters

Most widgets (knob, fader, toggle, switch, …) only need the scalar
parameter channel the base class wires up by default. Visualisers
(spectrum, waveform, meter) and input widgets that handle event /
MIDI streams need more. Two handles cover everything; pick the one
that matches how the data flows.

### Inside-out — widget subscribes via `conn`

When a patch exposes the feed as a Cmajor stream or event endpoint,
the widget pulls it in `onMeta` using the base class helper:

```js
class PlinkenSpectrum extends PlinkenWidget {
  onMeta(meta) {
    // … render the canvas + axes from `meta` …
    const bandsEndpoint = this.getAttribute('bands-endpoint');
    this.subscribeEndpoint(bandsEndpoint, (arr) => {
      this.pushBands(arr);    // drives the same render path as outside-in
    });
  }

  pushBands(arr) { /* smoothing + draw */ }
}
```

`subscribeEndpoint(endpoint, cb)` wraps Cmajor's
`addEndpointListener` / `removeEndpointListener` pair and registers
the teardown so it fires from `disconnectedCallback` alongside the
parameter listener. The widget never touches lifecycle directly.

### Outside-in — host pushes via a public method

When the data arrives over a transport the widget can't subscribe to
itself (a custom message channel, a Web Audio `AnalyserNode`, a unit
test, the designer's `MockConnection`), expose a public method on
the widget and have the plugin's `ui/index.html` script call it:

```js
// In the widget:
class PlinkenSpectrum extends PlinkenWidget {
  pushBands(arr) { /* smoothing + draw */ }
}

// In the plugin's ui/index.html:
const spec = document.querySelector('plinken-spectrum');
transport.onSpectrum((arr) => spec.pushBands(arr));
```

Public methods are just DOM API — no base-class plumbing needed.
The widget stays decoupled from any specific transport, so the same
element can be driven from any source that produces the right
shape of data (handy for the catalogue's mock conn and the
designer's preview).

### Escape hatch — `this.conn`

`this.conn` (a getter on the base class) returns the full Cmajor
`PatchConnection`. Use it only when neither helper above fits — for
example, a widget that needs to send MIDI input events
(`sendMIDIInputEvent`) or query a sibling endpoint's metadata. If a
new pattern shows up twice, promote it to a helper on the base
class rather than copy-pasting `conn` access.

### Rule of thumb

| Data flow                          | Use                          |
|------------------------------------|------------------------------|
| Scalar param, owned by the widget  | base class default           |
| Stream / event endpoint            | `subscribeEndpoint(ep, cb)`  |
| Custom transport, host-driven      | public method on the widget  |
| Anything else                      | `this.conn` escape hatch     |

## Attributes

Full schema in `DESIGNER.md`. Two rules:

1. Cmajor-parity attributes (`min`, `max`, `init`, `step`, `unit`,
   `text`) are *fallbacks*. Read them only for fields missing from
   the patch `meta`. Never overwrite a patch-supplied value.
2. UI-only attributes (`label`, `format`, `accent`, `scaling`,
   `x`/`y`/`w`/`h`) are always honoured.

For live updates from the designer's property panel: the canvas
destroys and re-creates the widget element on attribute change in
V1. Don't rely on `attributeChangedCallback` until the base class
adds `observedAttributes` (V2).

## Shadow DOM

Use open shadow DOM. Attach in `onMeta`, not the constructor (so
the element can be moved around the canvas before `setConnection`
lands without throwing).

```js
onMeta(meta) {
  const shadow = this.attachShadow({ mode: 'open' });
  shadow.innerHTML = `<style>…</style><svg viewBox=…>…</svg>`;
  …
}
```

CSS custom properties penetrate shadow boundaries, so theme tokens
still work. The shadow keeps the internal SVG out of the page's
`querySelector` namespace, which the designer relies on (it
selects placed widgets by `plinken-*` tag, never by inner SVG).

## SVG conventions

One `<svg>` per widget, sized to fill the host element:

```html
<svg viewBox="0 0 100 100" width="100%" height="100%" preserveAspectRatio="…"></svg>
```

- `preserveAspectRatio="xMidYMid meet"` (default) for aspect-locked
  widgets: knob, xy-pad, led, keyboard
- `preserveAspectRatio="none"` for stretch widgets: fader, meter,
  spectrum, waveform, background, dropdown, switch, button, toggle,
  label

Pick a `viewBox` that gives integer coordinates for the widget's
natural drawing units (e.g. `0 0 100 100` for a knob, `0 0 20 200`
for a vertical fader). Resizing happens at the SVG level, so
internal coordinates never need to change.

No embedded raster images; everything is vector. Icons that
look bitmap-y (LEDs, indicators) are circles with gradients, not
`<image href=...>`.

## Styling

All colours route through theme tokens (full list in `CATALOGUE.md`):

```css
.track     { fill: var(--plk-bg-deep); }
.fill      { fill: var(--plk-accent); }
.fill.warn { fill: var(--plk-accent-warn); }
.label     { fill: var(--plk-text-dim); }
```

No hex codes in widget CSS. If a needed colour isn't covered by an
existing token, add the token to `CATALOGUE.md` in the same PR
rather than one-offing.

## Fonts

Two webfonts ship inside `widget-lib`, declared via `@font-face` in
`widget-lib.css` and loaded once per plugin UI:

- **JetBrains Mono** Regular — the mono token (`--plk-font-mono`)
- **Inter** Regular — the display token (`--plk-font-display`)

Both are OFL 1.1, downloaded from Google Fonts, subset to Latin
(`U+0000-00FF` + common typographic punctuation), and live as woff2
in `widget-lib/fonts/`. Combined ~45 KB per plugin tarball.
`font-display: swap` is set on both, so widgets paint with the
system fallback (`ui-monospace` / `system-ui`) until the woff2
arrives, then re-render — no FOIT.

Tokens (defined in `widget-lib.css`):

```css
--plk-font-mono:    'JetBrains Mono', ui-monospace, 'Cascadia Mono',
                    'IBM Plex Mono', Menlo, Consolas, monospace;
--plk-font-display: 'Inter', system-ui, -apple-system, 'Segoe UI',
                    Roboto, sans-serif;
```

The bundled name comes first in each stack; the system fallback
covers the paint-before-load window and any environment where the
woff2 can't be served (offline catalogue snapshot, etc.).

Usage rule:

- `--plk-font-mono` — numeric readouts (“1000 Hz”, “-12.3 dB”),
  parameter values, anything that needs tabular figures. Always
  pair with `font-variant-numeric: tabular-nums` so digits don't
  jitter as values change width.
- `--plk-font-display` — labels (“CUTOFF”, “BYPASS”), button text,
  enum labels, anything that's a name rather than a value.

Labels conventionally render uppercase + letter-spaced + small (the
organ/limiter look). The widget chooses; rule of thumb:

```css
.label {
  font-family: var(--plk-font-display);
  font-size: 0.55rem;
  letter-spacing: 0.14em;
  text-transform: uppercase;
}
```

**Adding a weight or script:** drop the subset woff2 into
`widget-lib/fonts/` (same naming pattern) and add an `@font-face`
block to `widget-lib.css`. Stay subset to keep the per-plugin
tarball lean — avoid shipping full-range files. License compliance
(OFL 1.1) requires the source's `*-OFL.txt` to remain in
`widget-lib/fonts/`.

## Pointer + touch

One unified flow using Pointer Events; never mix in `mousedown` /
`touchstart`:

```js
el.addEventListener('pointerdown', e => {
  e.preventDefault();
  el.setPointerCapture(e.pointerId);
  /* begin drag, write(v, true) if you want a single-frame gesture */
});
el.addEventListener('pointermove', e => { /* if captured: update */ });
el.addEventListener('pointerup',   e => { /* end drag */ });
el.addEventListener('pointercancel', e => { /* same as up */ });
```

Wheel for fine-tune is opt-in per widget, with
`{ passive: false }` so `preventDefault()` works on trackpads.
Double-click resets to `meta.init` (or attribute fallback).

## Accessibility

Each widget sets the right `role` on its interactive root and
updates `aria-*` from meta + value:

| Widget   | role             | aria fields                                    |
|----------|------------------|------------------------------------------------|
| knob     | `slider`         | `aria-valuemin`/`max`/`now`/`valuetext` (with unit) |
| fader    | `slider`         | same                                           |
| toggle   | `switch`         | `aria-checked`                                 |
| button   | `button`         | —                                              |
| switch   | `radiogroup`     | child `radio` per option, `aria-checked`       |
| dropdown | native `<select>`| inherit                                        |
| xy-pad   | `application`    | custom; expose `aria-valuetext` summary        |
| meter    | `meter`          | `aria-valuemin`/`max`/`now`                    |
| keyboard | `application`    | per-key `role="button"` with `aria-pressed`    |
| label    | none             | —                                              |
| led      | `status`         | `aria-label` describing state                  |

Keyboard support is opt-in per widget. Sliders should accept
arrow keys (±1 step) and Page Up/Down (±10 step); switches should
accept Space + Enter to toggle.

## Catalogue entry

Every new widget lands a card on `/widgets` in the same PR (see
`CATALOGUE.md`). The card is the contract: it proves the widget
renders, animates against the mock conn, honours the theme switch,
and exposes a copy-paste attribute snippet. A widget without a
card is not shipped.

## Checklist (drop into the PR description)

- [ ] File at `widget-lib/<kind>.mjs`, `customElements.define` at top
- [ ] Extends `PlinkenWidget`, overrides `onMeta` + `onValue`
- [ ] Patch `meta` is authoritative; attributes fill gaps only
- [ ] Open shadow DOM, attached in `onMeta`
- [ ] One `<svg viewBox>`, correct `preserveAspectRatio` for kind
- [ ] Colours only through `--plk-*` tokens (no hex)
- [ ] Readouts use `--plk-font-mono` + `tabular-nums`
- [ ] Pointer Events only (no `mouse*` / `touch*`)
- [ ] `role` + `aria-*` per the table above
- [ ] Card added to `/widgets` route
- [ ] Snippet in card matches the schema in `DESIGNER.md`
