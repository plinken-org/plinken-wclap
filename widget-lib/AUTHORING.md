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

Two tokens, both system-only — no webfont loading from the widget
library itself (keeps the bundled tarball small and avoids cross-
origin font fetches inside the plugin iframe):

```css
:root {
  --plk-font-mono:    ui-monospace, 'JetBrains Mono', 'Cascadia Mono', 'IBM Plex Mono', Menlo, Consolas, monospace;
  --plk-font-display: system-ui, -apple-system, 'Inter', 'Segoe UI', Roboto, sans-serif;
}
```

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

If `apps/site` wants to use a webfont for the broader page, that
lands in the site's CSS, not in `widget-lib`. The widget tokens
inherit through cascading and pick up the webfont automatically.

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
