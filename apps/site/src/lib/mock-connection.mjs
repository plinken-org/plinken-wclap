// @ts-nocheck
// Mock PatchConnection for the /designer + /widgets routes.
// Implements the subset of the Cmajor PatchConnection API that
// PlinkenWidget calls — see DESIGNER.md § Mock connection.
//
// Synthesises `meta` for each endpoint by scanning the DOM under `root`
// for placed widgets and reading their Cmajor-parity attributes, falling
// back to the per-kind table from utils.mjs.

import { KIND_DEFAULTS } from '../../../../widget-lib/utils.mjs';

export class MockConnection {
  #root;
  #values = new Map();          // endpoint -> current value
  #meta = new Map();            // endpoint -> meta object
  #kinds = new Map();           // endpoint -> kind (knob/fader/...)
  #paramListeners = new Map();  // endpoint -> Set<cb>
  #eventListeners = new Map();  // endpoint -> Set<cb>
  #tickerId = null;
  #tickerStart = 0;

  constructor(root = document) {
    this.#root = root;
  }

  setRoot(root) {
    this.#root = root;
  }

  // ----- PatchConnection API -----

  async requestStatusUpdate() {
    this.#rescan();
    return {
      parameters: [...this.#meta.entries()].map(([endpointID, meta]) => ({
        endpointID,
        ...meta,
      })),
    };
  }

  addParameterListener(ep, cb) {
    if (!this.#paramListeners.has(ep)) this.#paramListeners.set(ep, new Set());
    this.#paramListeners.get(ep).add(cb);
  }

  removeParameterListener(ep, cb) {
    this.#paramListeners.get(ep)?.delete(cb);
  }

  requestParameterValue(ep) {
    const v = this.#values.get(ep);
    if (v !== undefined) queueMicrotask(() => this.#emit(ep, v));
  }

  sendParameterGestureStart(ep) { /* no-op in mock */ }
  sendParameterGestureEnd(ep)   { /* no-op in mock */ }

  sendEventOrValue(ep, value) {
    this.#values.set(ep, value);
    this.#emit(ep, value);
  }

  // For non-parameter feeds (spectrum bands, waveform frames, MIDI).
  addEndpointListener(ep, cb) {
    if (!this.#eventListeners.has(ep)) this.#eventListeners.set(ep, new Set());
    this.#eventListeners.get(ep).add(cb);
  }

  removeEndpointListener(ep, cb) {
    this.#eventListeners.get(ep)?.delete(cb);
  }

  // ----- Preview ticker -----

  startPreview() {
    if (this.#tickerId != null) return;
    this.#tickerStart = performance.now();
    this.#tickerId = setInterval(() => this.#tick(), 50);
  }

  stopPreview() {
    if (this.#tickerId == null) return;
    clearInterval(this.#tickerId);
    this.#tickerId = null;
    // Snap animatable endpoints back to init so the canvas looks calm.
    for (const [ep, meta] of this.#meta) {
      if (this.#isAnimatable(this.#kinds.get(ep))) {
        this.sendEventOrValue(ep, meta.init ?? 0);
      }
    }
  }

  get previewing() { return this.#tickerId != null; }

  // ----- internals -----

  #rescan() {
    this.#meta.clear();
    this.#kinds.clear();
    for (const el of this.#root.querySelectorAll('[endpoint]')) {
      const ep = el.getAttribute('endpoint');
      if (!ep) continue;
      const kind = el.tagName.toLowerCase().replace(/^plinken-/, '');
      const def = KIND_DEFAULTS[kind] ?? {};
      const meta = {
        min:  numAttr(el, 'min',  def.min),
        max:  numAttr(el, 'max',  def.max),
        init: numAttr(el, 'init', def.init),
        step: numAttr(el, 'step', def.step),
        unit: el.getAttribute('unit') ?? def.unit ?? '',
        text: el.getAttribute('text') ?? undefined,
      };
      this.#meta.set(ep, meta);
      this.#kinds.set(ep, kind);
      if (!this.#values.has(ep)) this.#values.set(ep, meta.init ?? 0);
    }
  }

  #emit(ep, value) {
    const listeners = this.#paramListeners.get(ep);
    if (!listeners) return;
    for (const cb of listeners) {
      try { cb(value); } catch (err) { console.error('listener', ep, err); }
    }
  }

  #tick() {
    const t = (performance.now() - this.#tickerStart) / 1000;
    for (const [ep, meta] of this.#meta) {
      const kind = this.#kinds.get(ep);
      if (!this.#isAnimatable(kind)) continue;
      const v = this.#animate(kind, t, ep, meta);
      this.#values.set(ep, v);
      this.#emit(ep, v);
    }
  }

  #isAnimatable(kind) {
    return kind === 'meter' || kind === 'spectrum' || kind === 'waveform';
  }

  #animate(kind, t, ep, meta) {
    // Per-endpoint phase offset so multiple meters don't lockstep.
    const phase = hash(ep) * 0.001;
    if (kind === 'meter') {
      // Quasi-musical envelope: slow swell + occasional peak.
      const swell = 0.5 + 0.45 * Math.sin(t * 1.7 + phase);
      const peak  = Math.max(0, Math.sin(t * 5.3 + phase) - 0.7) * 3;
      const u = Math.min(1, swell + peak * 0.2);
      return meta.min + (meta.max - meta.min) * u;
    }
    // spectrum / waveform: not used by initial widgets, but emit something.
    return meta.min + (meta.max - meta.min) * (0.5 + 0.5 * Math.sin(t + phase));
  }
}

function numAttr(el, name, fallback) {
  const a = el.getAttribute(name);
  if (a == null || a === '') return fallback;
  const v = parseFloat(a);
  return Number.isFinite(v) ? v : fallback;
}

function hash(s) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}
