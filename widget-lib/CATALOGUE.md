# Catalogue

Public showcase route at `/widgets` on `apps/site`. Displays every
widget kind in `@plinken/widget-lib` live, driven by the same
`MockConnection` the `/designer` route uses. Doubles as visual
reference for plugin authors and as visual-regression coverage when
widgets get added or restyled.

## Page layout

Single SvelteKit route at `apps/site/src/routes/widgets/+page.svelte`.

```
+--------------------------------------------------+
| Theme: [dark v]   Accent: [■]   Preview: [▶]    |
+--------------------------------------------------+
| [ knob ]  [ fader ]  [ meter ]  [ toggle ]  …   |
| [ switch ] [ dropdown ] [ xy-pad ] [ button ] … |
+--------------------------------------------------+
```

Top bar: theme dropdown (lists registered themes), accent swatch, a
`Preview` toggle wired to the mock ticker. Body: responsive grid,
one card per widget kind.

Each card:

- live widget instance bound to a per-card mock endpoint
  (`catalogue.knob`, `catalogue.fader`, …), animated by the preview
  ticker so knobs sweep, meters peak, spectra scroll
- canonical attribute snippet (copy-paste-able, matches the example
  in `DESIGNER.md`)
- one-line description (mirrors the bullet from `README.md` —
  knob: “gain/cutoff/Q/freq across most plugins”)
- per-card accent override demonstrating the one-off path without
  affecting siblings

## Mock data

Shared `MockConnection` instance for the whole page, one mock
endpoint per card using the per-kind defaults from
`DESIGNER.md`'s table. The card reads its widget's attributes to
synthesise `meta` just like the designer canvas does — same code
path, different host.

## Theme tokens

Every widget styles itself through this fixed set of CSS custom
properties. New widgets **must** route their colours through these
names; hardcoded values fail the catalogue's theme-switcher test
(snapshot under light + dark theme should both look right).

**Surface:**

| Token                | Use                                          |
|----------------------|----------------------------------------------|
| `--plk-bg`           | background fill behind widgets               |
| `--plk-bg-deep`      | recessed surface (track grooves, panel inset)|
| `--plk-border`       | chrome lines, widget outlines                |
| `--plk-border-soft`  | secondary lines, separators                  |

**Text:**

| Token                | Use                                          |
|----------------------|----------------------------------------------|
| `--plk-text`         | primary readout text + active thumb fill     |
| `--plk-text-dim`     | labels, inactive states                      |

**Accent:**

| Token                | Use                                          |
|----------------------|----------------------------------------------|
| `--plk-accent`       | primary highlight (active track, drag thumb) |
| `--plk-accent-deep`  | pressed / active border                      |
| `--plk-accent-warn`  | peak / clip / GR overshoot (red-ish)         |
| `--plk-accent-alt`   | secondary highlight (xy-pad y-axis, switch state) |

**Typographic:**

| Token                | Use                                          |
|----------------------|----------------------------------------------|
| `--plk-font-mono`    | readouts, numeric values                     |
| `--plk-font-display` | labels, widget chrome text                   |

**Sizing (optional, widget-local defaults override):**

| Token                | Use                                          |
|----------------------|----------------------------------------------|
| `--plk-radius`       | corner radius for boxed widgets              |
| `--plk-thumb-size`   | fader / knob handle dimensions               |

Default theme lives at `:root`. Alternate themes are blocks scoped
to `[data-theme="..."]` on `<body>` (or the catalogue page's root
element), setting the same properties.

```css
:root {
  --plk-bg: #1a1820;
  --plk-bg-deep: #0e0c12;
  --plk-border: #3a3540;
  --plk-border-soft: #2a2530;
  --plk-text: #e8e4f0;
  --plk-text-dim: #8b8095;
  --plk-accent: #925db3;
  --plk-accent-deep: #6f478b;
  --plk-accent-warn: #d65a5a;
  --plk-accent-alt: #4ea3a5;
  --plk-font-mono: 'JetBrains Mono', ui-monospace, monospace;
  --plk-font-display: 'Inter', system-ui, sans-serif;
  --plk-radius: 2px;
  --plk-thumb-size: 20px;
}

[data-theme="light"] {
  --plk-bg: #f4f1f8;
  --plk-bg-deep: #e6e1ec;
  --plk-border: #c8bfd2;
  --plk-border-soft: #ddd5e4;
  --plk-text: #2a2530;
  --plk-text-dim: #6e6478;
  /* accents may carry over or shift — themes choose */
}
```

## Per-widget accent override

Every widget accepts an `accent="#hex"` attribute. The base class
maps it to inline `style="--plk-accent: #hex"`, scoping the
override to that one element. Useful for one-off highlights
(per-channel colour in a multi-track meter, distinct accent on a
bypass toggle) without forking a theme.

```html
<plinken-knob endpoint="cutoff" accent="#4ea3a5" …/>
```

## Authoring rule

When authoring a new widget:

1. Reach for these tokens before reaching for hex codes.
2. If a widget genuinely needs a colour not covered by an existing
   token, add the token to this doc in the same PR — don't
   one-off a hex value.
3. Add a card for the widget to the catalogue page in the same PR.
   The catalogue is the contract: if it isn't on the page, it
   isn't shipped.

## Sequencing

The page can ship before the library is full. With one widget
(`plinken-knob` from the designer spike), the catalogue is a
one-card demo that already exercises the theme switcher and the
attribute-snippet copy. Each new widget authored adds one card.
