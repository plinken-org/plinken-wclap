// @ts-nocheck
import { PlinkenWidget } from './widget-base.mjs';
import { clamp, quantize, fromUnit, toUnit, formatValue, resolveMeta, KIND_DEFAULTS } from './utils.mjs';

const SWEEP_START_DEG = 135;
const SWEEP_END_DEG = 405;
const SWEEP_RANGE_DEG = SWEEP_END_DEG - SWEEP_START_DEG;
const CX = 50;
const CY = 50;
const R_TRACK = 36;
const DRAG_PIXELS_FULL_RANGE = 200;

function polar(cx, cy, r, deg) {
  const rad = (deg - 90) * Math.PI / 180;
  return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)];
}

function arcPath(cx, cy, r, startDeg, endDeg) {
  const [x1, y1] = polar(cx, cy, r, startDeg);
  const [x2, y2] = polar(cx, cy, r, endDeg);
  const large = Math.abs(endDeg - startDeg) > 180 ? 1 : 0;
  const sweep = endDeg > startDeg ? 1 : 0;
  return `M ${x1.toFixed(3)} ${y1.toFixed(3)} A ${r} ${r} 0 ${large} ${sweep} ${x2.toFixed(3)} ${y2.toFixed(3)}`;
}

export class PlinkenKnob extends PlinkenWidget {
  #meta = null;
  #value = 0;
  #scaling = 'lin';
  #format = '';
  #labelText = '';
  #trackPath = null;
  #fillPath = null;
  #pointer = null;
  #readout = null;
  #svgRoot = null;
  #dragging = false;
  #dragStartY = 0;
  #dragStartT = 0;

  onMeta(meta) {
    const defaults = KIND_DEFAULTS.knob;
    const resolved = resolveMeta(meta, this, defaults);
    this.#meta = resolved;
    this.#scaling = (this.getAttribute('scaling') || 'lin').toLowerCase();
    this.#format = this.getAttribute('format') || '';
    this.#labelText = this.getAttribute('label') || this.getAttribute('endpoint') || '';

    const accent = this.getAttribute('accent');
    if (accent) this.style.setProperty('--plk-accent', accent);

    const shadow = this.attachShadow({ mode: 'open' });
    shadow.innerHTML = `
<style>
  :host {
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    outline: none;
    user-select: none;
    -webkit-user-select: none;
    touch-action: none;
    color: var(--plk-text);
  }
  :host(:focus-visible) .dial { stroke: var(--plk-accent-deep); }
  .wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    gap: 0.15rem;
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
    flex: 1 1 auto;
    min-height: 0;
  }
  .track { fill: none; stroke: var(--plk-bg-deep); stroke-width: 8; stroke-linecap: round; }
  .fill  { fill: none; stroke: var(--plk-accent); stroke-width: 8; stroke-linecap: round; }
  .dial  { fill: var(--plk-bg); stroke: var(--plk-border); stroke-width: 1; }
  .pointer { stroke: var(--plk-text); stroke-width: 2.5; stroke-linecap: round; }
  .label {
    font-family: var(--plk-font-display);
    font-size: 0.55rem;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--plk-text-dim);
    line-height: 1;
  }
  .readout {
    font-family: var(--plk-font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 0.65rem;
    color: var(--plk-text);
    line-height: 1;
  }
</style>
<div class="wrap">
  <svg viewBox="0 0 100 100" preserveAspectRatio="xMidYMid meet"
       role="slider" tabindex="-1" aria-orientation="vertical">
    <path class="track" d="${arcPath(CX, CY, R_TRACK, SWEEP_START_DEG, SWEEP_END_DEG)}"/>
    <path class="fill"  d=""/>
    <circle class="dial" cx="${CX}" cy="${CY}" r="${R_TRACK - 8}"/>
    <line class="pointer" x1="${CX}" y1="${CY - (R_TRACK - 14)}" x2="${CX}" y2="${CY - (R_TRACK - 4)}"
          transform="rotate(${SWEEP_START_DEG} ${CX} ${CY})"/>
  </svg>
  <div class="label">${escapeHtml(this.#labelText)}</div>
  <div class="readout">—</div>
</div>`;

    this.#svgRoot = shadow.querySelector('svg');
    this.#trackPath = shadow.querySelector('.track');
    this.#fillPath = shadow.querySelector('.fill');
    this.#pointer = shadow.querySelector('.pointer');
    this.#readout = shadow.querySelector('.readout');

    this.setAttribute('role', 'slider');
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');
    this.setAttribute('aria-valuemin', String(resolved.min));
    this.setAttribute('aria-valuemax', String(resolved.max));
    if (this.#labelText) this.setAttribute('aria-label', this.#labelText);

    this.#installPointer();
    this.#installKeyboard();
    this.#installWheel();
    this.#installDoubleClick();

    this.#applyValue(resolved.init ?? resolved.min);
  }

  onValue(v) {
    if (!this.#meta) return;
    this.#applyValue(v);
  }

  #applyValue(v) {
    const { min, max, unit } = this.#meta;
    const c = clamp(v, min, max);
    this.#value = c;
    const t = clamp(toUnit(c, min, max, this.#scaling), 0, 1);
    const endDeg = SWEEP_START_DEG + t * SWEEP_RANGE_DEG;
    this.#fillPath.setAttribute('d', t > 0 ? arcPath(CX, CY, R_TRACK, SWEEP_START_DEG, endDeg) : '');
    this.#pointer.setAttribute('transform', `rotate(${endDeg} ${CX} ${CY})`);
    const text = formatValue(c, unit, this.#format);
    this.#readout.textContent = text;
    this.setAttribute('aria-valuenow', String(c));
    this.setAttribute('aria-valuetext', text);
  }

  #effectiveStep() {
    const { step, min, max } = this.#meta;
    if (step && step > 0) return step;
    return (max - min) / 100;
  }

  #writeFromT(t, gesture = false) {
    const { min, max, step } = this.#meta;
    const clampedT = clamp(t, 0, 1);
    let v = fromUnit(clampedT, min, max, this.#scaling);
    v = quantize(v, step, min);
    v = clamp(v, min, max);
    this.#applyValue(v);
    this.write(v, gesture);
  }

  #installPointer() {
    const host = this;
    host.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 && e.pointerType === 'mouse') return;
      e.preventDefault();
      host.focus();
      host.setPointerCapture(e.pointerId);
      this.#dragging = true;
      this.#dragStartY = e.clientY;
      const { min, max } = this.#meta;
      this.#dragStartT = clamp(toUnit(this.#value, min, max, this.#scaling), 0, 1);
    });
    host.addEventListener('pointermove', (e) => {
      if (!this.#dragging) return;
      const dy = this.#dragStartY - e.clientY;
      const fine = e.shiftKey ? 0.2 : 1;
      const dt = (dy / DRAG_PIXELS_FULL_RANGE) * fine;
      this.#writeFromT(this.#dragStartT + dt, false);
    });
    const end = (e) => {
      if (!this.#dragging) return;
      this.#dragging = false;
      try { host.releasePointerCapture(e.pointerId); } catch {}
      // single-frame gesture wrap so the host groups the drag for undo
      this.write(this.#value, true);
    };
    host.addEventListener('pointerup', end);
    host.addEventListener('pointercancel', end);
  }

  #installKeyboard() {
    this.addEventListener('keydown', (e) => {
      const step = this.#effectiveStep();
      let delta = 0;
      switch (e.key) {
        case 'ArrowUp': case 'ArrowRight': delta = step; break;
        case 'ArrowDown': case 'ArrowLeft': delta = -step; break;
        case 'PageUp': delta = step * 10; break;
        case 'PageDown': delta = -step * 10; break;
        case 'Home': {
          e.preventDefault();
          const v = this.#meta.min;
          this.#applyValue(v);
          this.write(v, true);
          return;
        }
        case 'End': {
          e.preventDefault();
          const v = this.#meta.max;
          this.#applyValue(v);
          this.write(v, true);
          return;
        }
        default: return;
      }
      e.preventDefault();
      const v = clamp(quantize(this.#value + delta, this.#meta.step, this.#meta.min), this.#meta.min, this.#meta.max);
      this.#applyValue(v);
      this.write(v, true);
    });
  }

  #installWheel() {
    this.addEventListener('wheel', (e) => {
      e.preventDefault();
      const { min, max } = this.#meta;
      const t = clamp(toUnit(this.#value, min, max, this.#scaling), 0, 1);
      const fine = e.shiftKey ? 0.02 : 0.1;
      const dt = (-e.deltaY / DRAG_PIXELS_FULL_RANGE) * fine;
      this.#writeFromT(t + dt, true);
    }, { passive: false });
  }

  #installDoubleClick() {
    this.addEventListener('dblclick', (e) => {
      e.preventDefault();
      const init = this.#meta.init;
      if (init == null) return;
      this.#applyValue(init);
      this.write(init, true);
    });
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => (
    c === '&' ? '&amp;' :
    c === '<' ? '&lt;' :
    c === '>' ? '&gt;' :
    c === '"' ? '&quot;' : '&#39;'
  ));
}

customElements.define('plinken-knob', PlinkenKnob);
