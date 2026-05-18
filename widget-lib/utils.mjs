// @ts-nocheck
// Shared widget helpers: range mapping (linear + log), clamping, quantising,
// and value formatting. Imported from individual widget files; no DOM access
// here so the same module works inside a plugin tarball or the designer page.

export function clamp(v, min, max) {
  return v < min ? min : v > max ? max : v;
}

// Snap to nearest multiple of step (relative to min, when given).
// step <= 0 returns v unchanged.
export function quantize(v, step, min = 0) {
  if (!step || step <= 0) return v;
  return min + Math.round((v - min) / step) * step;
}

// Map t in [0,1] to a value in [min,max] (linear or log).
// log scaling requires min > 0 and max > 0 (caller's responsibility).
export function fromUnit(t, min, max, scaling = 'lin') {
  if (scaling === 'log') {
    const lmin = Math.log(min);
    return Math.exp(lmin + (Math.log(max) - lmin) * t);
  }
  return min + (max - min) * t;
}

// Inverse of fromUnit — value in [min,max] to t in [0,1].
export function toUnit(v, min, max, scaling = 'lin') {
  if (scaling === 'log') {
    const lmin = Math.log(min);
    return (Math.log(v) - lmin) / (Math.log(max) - lmin);
  }
  return (v - min) / (max - min);
}

// Format a value for display. Supports a tiny subset of patterns:
//   "{v:.1f} kHz"   → "12.3 kHz"
//   "{v:.0f}%"      → "42%"
//   no format       → pretty default (3 sig figs, then unit)
//
// Designed to cover the common Cmajor unit/format cases; widgets that
// need richer formatting can call this twice or roll their own.
export function formatValue(v, unit = '', format = '') {
  if (!Number.isFinite(v)) return '—';
  if (format) {
    return format.replace(/\{v(?::\.(\d+)f)?\}/g, (_, digits) => {
      return digits != null ? v.toFixed(+digits) : prettyNumber(v);
    });
  }
  const s = prettyNumber(v);
  return unit ? `${s} ${unit}` : s;
}

function prettyNumber(v) {
  const a = Math.abs(v);
  if (a >= 1000) return v.toFixed(0);
  if (a >= 100)  return v.toFixed(1);
  if (a >= 10)   return v.toFixed(2);
  return v.toFixed(3);
}

// Resolve a widget's effective meta: patch meta wins, attributes fill gaps.
// Returns a plain object with min/max/init/step/unit/text fields.
export function resolveMeta(meta, el, defaults = {}) {
  const get = (k, parse = (x) => x) => {
    if (meta && meta[k] != null) return meta[k];
    const a = el.getAttribute(k);
    if (a != null) return parse(a);
    return defaults[k];
  };
  return {
    min:  get('min',  parseFloat),
    max:  get('max',  parseFloat),
    init: get('init', parseFloat),
    step: get('step', parseFloat),
    unit: get('unit'),
    text: get('text'),
  };
}

// Per-kind fallback table from DESIGNER.md § Mock connection.
export const KIND_DEFAULTS = {
  knob:     { min: 0,   max: 1,   init: 0.5, unit: '',   step: undefined },
  fader:    { min: -60, max: 0,   init: -12, unit: 'dB', step: 0.1 },
  toggle:   { min: 0,   max: 1,   init: 0,   unit: '',   step: 1 },
  switch:   { min: 0,   max: 3,   init: 0,   unit: '',   step: 1 },
  dropdown: { min: 0,   max: 3,   init: 0,   unit: '',   step: 1 },
  button:   { min: 0,   max: 1,   init: 0,   unit: '',   step: 1 },
  meter:    { min: -60, max: 0,   init: -60, unit: 'dB', step: undefined },
  spectrum: { min: 0,   max: 1,   init: 0,   unit: '',   step: undefined },
  waveform: { min: -1,  max: 1,   init: 0,   unit: '',   step: undefined },
  'xy-pad': { min: 0,   max: 1,   init: 0.5, unit: '',   step: undefined },
  led:      { min: 0,   max: 1,   init: 0,   unit: '',   step: 1 },
  keyboard: { min: 0,   max: 127, init: 60,  unit: '',   step: 1 },
};
