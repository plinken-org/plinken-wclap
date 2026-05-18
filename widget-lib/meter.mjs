// @ts-nocheck
import { PlinkenWidget } from './widget-base.mjs';
import { clamp, toUnit, formatValue, resolveMeta, KIND_DEFAULTS } from './utils.mjs';

const PEAK_HOLD_MS = 1500;
const PEAK_DECAY_DB_PER_SEC = 30;

export class PlinkenMeter extends PlinkenWidget {
  #meta = null;
  #value = -Infinity;
  #peak = -Infinity;
  #peakHeldUntil = 0;
  #orientation = 'vertical';
  #format = '';
  #labelText = '';
  #warnThreshold = null;
  #fillRect = null;
  #warnRect = null;
  #peakLine = null;
  #readout = null;
  #rafId = 0;
  #lastDecayTs = 0;

  onMeta(meta) {
    const resolved = resolveMeta(meta, this, KIND_DEFAULTS.meter);
    this.#meta = resolved;
    this.#orientation = (this.getAttribute('orientation') || 'vertical').toLowerCase();
    this.#format = this.getAttribute('format') || '';
    this.#labelText = this.getAttribute('label') || this.getAttribute('endpoint') || '';

    const accent = this.getAttribute('accent');
    if (accent) this.style.setProperty('--plk-accent', accent);

    const warnAttr = this.getAttribute('warn-threshold');
    if (warnAttr != null) {
      const n = parseFloat(warnAttr);
      this.#warnThreshold = Number.isFinite(n) ? n : null;
    } else if (resolved.unit === 'dB' && resolved.max === 0) {
      this.#warnThreshold = -3;
    } else {
      this.#warnThreshold = null;
    }

    const isVertical = this.#orientation !== 'horizontal';
    const vb = isVertical ? '0 0 20 200' : '0 0 200 20';
    const ticks = this.#tickMarks(resolved, isVertical);

    const shadow = this.attachShadow({ mode: 'open' });
    shadow.innerHTML = `
<style>
  :host {
    display: inline-flex;
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    color: var(--plk-text);
  }
  .wrap {
    display: flex;
    flex-direction: ${isVertical ? 'column' : 'row'};
    align-items: stretch;
    justify-content: center;
    width: 100%;
    height: 100%;
    gap: 0.2rem;
  }
  .bar-wrap {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .track { fill: var(--plk-bg-deep); }
  .fill  { fill: var(--plk-accent); }
  .warn  { fill: var(--plk-accent-warn); }
  .peak  { stroke: var(--plk-text); stroke-width: 2; }
  .tick  { stroke: var(--plk-border-soft); stroke-width: 0.5; }
  .text-row {
    display: flex;
    flex-direction: ${isVertical ? 'column' : 'row'};
    align-items: center;
    justify-content: ${isVertical ? 'center' : 'space-between'};
    gap: 0.15rem;
    flex: 0 0 auto;
  }
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
  <div class="bar-wrap">
    <svg viewBox="${vb}" preserveAspectRatio="none">
      <rect class="track" x="0" y="0" width="${isVertical ? 20 : 200}" height="${isVertical ? 200 : 20}"/>
      <rect class="fill" x="0" y="0" width="0" height="0"/>
      <rect class="warn" x="0" y="0" width="0" height="0"/>
      ${ticks}
      <line class="peak" x1="0" y1="0" x2="0" y2="0" stroke-opacity="0"/>
    </svg>
  </div>
  <div class="text-row">
    <div class="label">${escapeHtml(this.#labelText)}</div>
    <div class="readout">—</div>
  </div>
</div>`;

    this.#fillRect = shadow.querySelector('.fill');
    this.#warnRect = shadow.querySelector('.warn');
    this.#peakLine = shadow.querySelector('.peak');
    this.#readout = shadow.querySelector('.readout');

    this.setAttribute('role', 'meter');
    this.setAttribute('aria-valuemin', String(resolved.min));
    this.setAttribute('aria-valuemax', String(resolved.max));
    if (this.#labelText) this.setAttribute('aria-label', this.#labelText);

    this.#paint(resolved.init ?? resolved.min);
  }

  onValue(v) {
    if (!this.#meta) return;
    this.#paint(v);
    if (!this.#rafId) {
      this.#lastDecayTs = performance.now();
      this.#rafId = requestAnimationFrame(this.#tickDecay);
    }
  }

  #tickDecay = (ts) => {
    if (!this.#meta) { this.#rafId = 0; return; }
    const { min, max } = this.#meta;
    const dt = (ts - this.#lastDecayTs) / 1000;
    this.#lastDecayTs = ts;
    if (ts >= this.#peakHeldUntil && this.#peak > this.#value) {
      const range = max - min;
      // Decay rate: 30 dB/s in dB, else equivalent fraction of range per second
      const rate = (this.#meta.unit === 'dB') ? PEAK_DECAY_DB_PER_SEC : (range * PEAK_DECAY_DB_PER_SEC / 60);
      this.#peak = Math.max(this.#value, this.#peak - rate * dt);
      this.#paintPeak();
    }
    if (this.#peak > this.#value || ts < this.#peakHeldUntil) {
      this.#rafId = requestAnimationFrame(this.#tickDecay);
    } else {
      this.#rafId = 0;
    }
  };

  #paint(v) {
    const { min, max, unit } = this.#meta;
    const c = clamp(v, min, max);
    this.#value = c;
    if (c >= this.#peak) {
      this.#peak = c;
      this.#peakHeldUntil = performance.now() + PEAK_HOLD_MS;
    }

    const isVertical = this.#orientation !== 'horizontal';
    const t = clamp(toUnit(c, min, max), 0, 1);

    if (isVertical) {
      const h = t * 200;
      const y = 200 - h;
      if (this.#warnThreshold != null && c > this.#warnThreshold) {
        const tWarn = clamp(toUnit(this.#warnThreshold, min, max), 0, 1);
        const warnY = 200 - t * 200;
        const mainTopY = 200 - tWarn * 200;
        this.#fillRect.setAttribute('x', '0');
        this.#fillRect.setAttribute('y', String(mainTopY));
        this.#fillRect.setAttribute('width', '20');
        this.#fillRect.setAttribute('height', String(200 - mainTopY));
        this.#warnRect.setAttribute('x', '0');
        this.#warnRect.setAttribute('y', String(warnY));
        this.#warnRect.setAttribute('width', '20');
        this.#warnRect.setAttribute('height', String(mainTopY - warnY));
      } else {
        this.#fillRect.setAttribute('x', '0');
        this.#fillRect.setAttribute('y', String(y));
        this.#fillRect.setAttribute('width', '20');
        this.#fillRect.setAttribute('height', String(h));
        this.#warnRect.setAttribute('height', '0');
      }
    } else {
      const w = t * 200;
      if (this.#warnThreshold != null && c > this.#warnThreshold) {
        const tWarn = clamp(toUnit(this.#warnThreshold, min, max), 0, 1);
        const mainW = tWarn * 200;
        this.#fillRect.setAttribute('x', '0');
        this.#fillRect.setAttribute('y', '0');
        this.#fillRect.setAttribute('width', String(mainW));
        this.#fillRect.setAttribute('height', '20');
        this.#warnRect.setAttribute('x', String(mainW));
        this.#warnRect.setAttribute('y', '0');
        this.#warnRect.setAttribute('width', String(w - mainW));
        this.#warnRect.setAttribute('height', '20');
      } else {
        this.#fillRect.setAttribute('x', '0');
        this.#fillRect.setAttribute('y', '0');
        this.#fillRect.setAttribute('width', String(w));
        this.#fillRect.setAttribute('height', '20');
        this.#warnRect.setAttribute('width', '0');
      }
    }

    this.#paintPeak();

    const text = formatValue(c, unit, this.#format);
    this.#readout.textContent = text;
    this.setAttribute('aria-valuenow', String(c));
    this.setAttribute('aria-valuetext', text);
  }

  #paintPeak() {
    if (!this.#meta) return;
    const { min, max } = this.#meta;
    const isVertical = this.#orientation !== 'horizontal';
    const tp = clamp(toUnit(this.#peak, min, max), 0, 1);
    if (this.#peak <= min || tp <= 0) {
      this.#peakLine.setAttribute('stroke-opacity', '0');
      return;
    }
    this.#peakLine.setAttribute('stroke-opacity', '1');
    if (isVertical) {
      const y = 200 - tp * 200;
      this.#peakLine.setAttribute('x1', '0');
      this.#peakLine.setAttribute('x2', '20');
      this.#peakLine.setAttribute('y1', String(y));
      this.#peakLine.setAttribute('y2', String(y));
    } else {
      const x = tp * 200;
      this.#peakLine.setAttribute('x1', String(x));
      this.#peakLine.setAttribute('x2', String(x));
      this.#peakLine.setAttribute('y1', '0');
      this.#peakLine.setAttribute('y2', '20');
    }
  }

  #tickMarks(meta, isVertical) {
    const { min, max, unit } = meta;
    const stepUnit = (unit === 'dB') ? 6 : (max - min) / 10;
    if (!Number.isFinite(stepUnit) || stepUnit <= 0) return '';
    const out = [];
    for (let v = min; v <= max + 1e-6; v += stepUnit) {
      const t = clamp(toUnit(v, min, max), 0, 1);
      if (isVertical) {
        const y = 200 - t * 200;
        out.push(`<line class="tick" x1="0" y1="${y.toFixed(2)}" x2="20" y2="${y.toFixed(2)}"/>`);
      } else {
        const x = t * 200;
        out.push(`<line class="tick" x1="${x.toFixed(2)}" y1="0" x2="${x.toFixed(2)}" y2="20"/>`);
      }
    }
    return out.join('');
  }

  disconnectedCallback() {
    if (this.#rafId) {
      cancelAnimationFrame(this.#rafId);
      this.#rafId = 0;
    }
    super.disconnectedCallback();
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

customElements.define('plinken-meter', PlinkenMeter);
