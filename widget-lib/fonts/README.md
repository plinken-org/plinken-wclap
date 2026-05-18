# Bundled fonts

Latin subsets of two OFL-licensed fonts, downloaded from Google Fonts
and self-hosted so plugin UIs render consistently across OS, work
offline, and avoid the privacy / CSP footprint of linking to
`fonts.googleapis.com` from inside a plugin iframe.

| File                                | Source                                      | License |
|-------------------------------------|---------------------------------------------|---------|
| `Inter-Regular.latin.woff2`         | https://github.com/rsms/inter               | OFL 1.1 — see `Inter-OFL.txt`         |
| `JetBrainsMono-Regular.latin.woff2` | https://github.com/JetBrains/JetBrainsMono  | OFL 1.1 — see `JetBrainsMono-OFL.txt` |

Both are the Regular (weight 400) face only, subset to the Latin
unicode range (`U+0000-00FF` + common typographic punctuation). Total
on-disk: ~45 KB combined.

Loaded via `@font-face` in `../widget-lib.css` with `font-display:
swap`, so widgets paint with the system fallback (`ui-monospace` /
`system-ui`) until the woff2 arrives, then re-render with the
bundled glyph.

## Adding weights / glyphs

If a future widget needs a bold weight or a non-Latin script, drop the
relevant subset woff2 here (same naming pattern) and add an
`@font-face` block to `widget-lib.css`. Stay subset to keep the
per-plugin tarball small — avoid shipping full-range files.

## License compliance

OFL 1.1 lets us redistribute the fonts (including modified subsets)
provided the license text travels with them. The two `*-OFL.txt`
files in this directory satisfy that requirement and must remain
present in every plugin tarball that bundles `widget-lib/`.
