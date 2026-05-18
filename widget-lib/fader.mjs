// @ts-nocheck
import { PlinkenWidget } from './widget-base.mjs';
import {
  clamp,
  quantize,
  fromUnit,
  toUnit,
  formatValue,
  resolveMeta,
  KIND_DEFAULTS,
} from './utils.mjs';

export class PlinkenFader extends PlinkenWidget {
  #meta = null;
  #value = 0;
  #scaling = 'lin';
  #format = '';
  #horizontal = false;
  #hasCenterZero = false;
  #zeroT = 0;

  #svg = null;
  #fill = null;
  #thumb = null;
  #readout = null;
  #track = null;
  #root = null;

  onMeta(meta) {
    const m = resolveMeta(meta, this, KIND_DEFAULTS.fader);
    this.#meta = m;
    this.#scaling = this.getAttribute('scaling') || 'lin';
    this.#format = this.getAttribute('format') || '';
    this.#horizontal = this.getAttribute('orientation') === 'horizontal';
    this.#hasCenterZero = m.min < 0 && m.max > 0;
    this.#zeroT = this.#hasCenterZero
      ? toUnit(0, m.min, m.max, this.#scaling)
      : (m.min >= 0 ? 0 : 1);

    const accent = this.getAttribute('accent');
    if (accent) this.style.setProperty('--plk-accent', accent);

    const labelText = (this.getAttribute('label') ?? (meta?.endpointID ?? this.getAttribute('endpoint') ?? '')).toString();

    const shadow = this.attachShadow({ mode: 'open' });
    const viewBox = this.#horizontal ? '0 0 200 20' : '0 0 20 200';
    const orientClass = this.#horizontal ? 'horizontal' : 'vertical';

    shadow.innerHTML = `
<style>
  :host {
    display: inline-flex;
    ${this.#horizontal
      ? 'flex-direction: row; align-items: center; gap: 8px;'
      : 'flex-direction: column; align-items: stretch; gap: 4px;'}
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    color: var(--plk-text);
    user-select: none;
    -webkit-user-select: none;
    touch-action: none;
  }
  .root {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: block;
    outline: none;
  }
  .root:focus-visible {
    box-shadow: 0 0 0 2px var(--plk-accent-deep);
    border-radius: var(--plk-radius, 2px);
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .track {
    fill: var(--plk-bg-deep);
    stroke: var(--plk-border-soft);
    stroke-width: 0.5;
  }
  .fill {
    fill: var(--plk-accent);
  }
  .zero {
    stroke: var(--plk-border);
    stroke-width: 0.75;
  }
  .thumb {
    fill: var(--plk-text);
    stroke: var(--plk-accent-deep);
    stroke-width: 0.5;
  }
  .meta {
    display: flex;
    ${this.#horizontal
      ? 'flex-direction: column; align-items: flex-start; gap: 2px;'
      : 'flex-direction: column; align-items: center; gap: 2px;'}
    flex: 0 0 auto;
  }
  .label {
    font-family: var(--plk-font-display);
    font-size: 0.55rem;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--plk-text-dim);
    line-height: 1;
    white-space: nowrap;
  }
  .readout {
    font-family: var(--plk-font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 0.7rem;
    color: var(--plk-text);
    line-height: 1;
    white-space: nowrap;
  }
</style>
<div class="root ${orientClass}" tabindex="0">
  <svg viewBox="${viewBox}" preserveAspectRatio="none">
    <rect class="track" x="${this.#horizontal ? 0 : 8}" y="${this.#horizontal ? 8 : 0}"
          width="${this.#horizontal ? 200 : 4}" height="${this.#horizontal ? 4 : 200}"
          rx="1" ry="1"/>
    ${this.#hasCenterZero ? `<line class="zero"
        x1="${this.#horizontal ? 200 * this.#zeroT : 4}"
        y1="${this.#horizontal ? 4 : 200 - 200 * this.#zeroT}"
        x2="${this.#horizontal ? 200 * this.#zeroT : 16}"
        y2="${this.#horizontal ? 16 : 200 - 200 * this.#zeroT}"/>` : ''}
    <rect class="fill" x="0" y="0" width="0" height="0"/>
    <rect class="thumb" x="0" y="0" width="${this.#horizontal ? 6 : 18}" height="${this.#horizontal ? 18 : 6}" rx="1" ry="1"/>
  </svg>
</div>
<div class="meta">
  <div class="label">${escapeHtml(labelText)}</div>
  <div class="readout">—</div>
</div>`;

    this.#root = shadow.querySelector('.root');
    this.#svg = shadow.querySelector('svg');
    this.#track = shadow.querySelector('.track');
    this.#fill = shadow.querySelector('.fill');
    this.#thumb = shadow.querySelector('.thumb');
    this.#readout = shadow.querySelector('.readout');

    this.#root.setAttribute('role', 'slider');
    this.#root.setAttribute('aria-orientation', this.#horizontal ? 'horizontal' : 'vertical');
    this.#root.setAttribute('aria-valuemin', String(m.min));
    this.#root.setAttribute('aria-valuemax', String(m.max));

    this.#wireInput();
    this.#applyValue(m.init);
  }

  onValue(v) {
    this.#applyValue(v);
  }

  #applyValue(v) {
    const m = this.#meta;
    if (!m) return;
    const cv = clamp(v, m.min, m.max);
    this.#value = cv;
    const t = toUnit(cv, m.min, m.max, this.#scaling);

    if (this.#horizontal) {
      const x = 200 * Math.min(t, this.#zeroT);
      const w = 200 * Math.abs(t - this.#zeroT);
      this.#fill.setAttribute('x', String(x));
      this.#fill.setAttribute('y', '8');
      this.#fill.setAttribute('width', String(w));
      this.#fill.setAttribute('height', '4');
      const tx = 200 * t - 3;
      this.#thumb.setAttribute('x', String(clamp(tx, 0, 194)));
      this.#thumb.setAttribute('y', '1');
    } else {
      const yTop = 200 * (1 - Math.max(t, this.#zeroT));
      const h = 200 * Math.abs(t - this.#zeroT);
      this.#fill.setAttribute('x', '8');
      this.#fill.setAttribute('y', String(yTop));
      this.#fill.setAttribute('width', '4');
      this.#fill.setAttribute('height', String(h));
      const ty = 200 * (1 - t) - 3;
      this.#thumb.setAttribute('x', '1');
      this.#thumb.setAttribute('y', String(clamp(ty, 0, 194)));
    }

    const text = formatValue(cv, m.unit, this.#format);
    this.#readout.textContent = text;
    this.#root.setAttribute('aria-valuenow', String(cv));
    this.#root.setAttribute('aria-valuetext', text);
  }

  #posToValue(clientX, clientY) {
    const m = this.#meta;
    const rect = this.#svg.getBoundingClientRect();
    let t;
    if (this.#horizontal) {
      t = (clientX - rect.left) / rect.width;
    } else {
      t = 1 - (clientY - rect.top) / rect.height;
    }
    t = clamp(t, 0, 1);
    let v = fromUnit(t, m.min, m.max, this.#scaling);
    v = quantize(v, m.step ?? 0, m.min);
    return clamp(v, m.min, m.max);
  }

  #wireInput() {
    const root = this.#root;
    let dragging = false;

    root.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      root.setPointerCapture(e.pointerId);
      dragging = true;
      const v = this.#posToValue(e.clientX, e.clientY);
      this.#applyValue(v);
      this.write(v);
    });

    root.addEventListener('pointermove', (e) => {
      if (!dragging) return;
      const v = this.#posToValue(e.clientX, e.clientY);
      this.#applyValue(v);
      this.write(v);
    });

    const end = (e) => {
      if (!dragging) return;
      dragging = false;
      try { root.releasePointerCapture(e.pointerId); } catch {}
      this.write(this.#value, true);
    };
    root.addEventListener('pointerup', end);
    root.addEventListener('pointercancel', end);

    root.addEventListener('dblclick', (e) => {
      e.preventDefault();
      const m = this.#meta;
      this.#applyValue(m.init);
      this.write(m.init, true);
    });

    root.addEventListener('wheel', (e) => {
      e.preventDefault();
      const m = this.#meta;
      const step = (m.step && m.step > 0) ? m.step : (m.max - m.min) / 200;
      const dir = e.deltaY < 0 ? 1 : -1;
      const fine = e.shiftKey ? 0.1 : 1;
      let v = this.#value + dir * step * fine;
      v = quantize(v, m.step ?? 0, m.min);
      v = clamp(v, m.min, m.max);
      this.#applyValue(v);
      this.write(v, true);
    }, { passive: false });

    root.addEventListener('keydown', (e) => {
      const m = this.#meta;
      const step = (m.step && m.step > 0) ? m.step : (m.max - m.min) / 100;
      let delta = 0;
      switch (e.key) {
        case 'ArrowUp':
        case 'ArrowRight':   delta =  step; break;
        case 'ArrowDown':
        case 'ArrowLeft':    delta = -step; break;
        case 'PageUp':       delta =  step * 10; break;
        case 'PageDown':     delta = -step * 10; break;
        case 'Home':
          e.preventDefault();
          this.#applyValue(m.min);
          this.write(m.min, true);
          return;
        case 'End':
          e.preventDefault();
          this.#applyValue(m.max);
          this.write(m.max, true);
          return;
        default: return;
      }
      e.preventDefault();
      let v = this.#value + delta;
      v = quantize(v, m.step ?? 0, m.min);
      v = clamp(v, m.min, m.max);
      this.#applyValue(v);
      this.write(v, true);
    });
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[c]));
}

customElements.define('plinken-fader', PlinkenFader);
