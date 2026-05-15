// Rotary pot (knob) widget — synome-style SVG, auto-pan-style drag UX.
//
// Drag up = increment, drag down = decrement (240 px = full range, 4×
// slower with Shift). Wheel nudges 2 %. Double-click resets to default.
//
// Usage:
//   const p = new Pot(el, { id: 4097, min: 0.1, max: 20, default: 5,
//                           unit: 'Hz', log: true, label: 'Speed' });
//   p.onChange(v => { ... });
//   p.setValue(2.5);                  // programmatic (no event emit)
//   p.setValueFromHost(v);            // from snapshot — same as setValue

const STYLE_ID = 'plinken-pot-style';

const STYLES = `
.pot {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  touch-action: none;
}
.pot-dial {
  position: relative;
  width: var(--pot-size, 56px);
  height: var(--pot-size, 56px);
  cursor: ns-resize;
}
.pot-dial svg {
  width: 100%;
  height: 100%;
  display: block;
  overflow: visible;
}
.pot-track {
  fill: none;
  stroke: var(--bg-deep);
  stroke-width: var(--pot-track, 4px);
  stroke-linecap: round;
}
.pot-fill {
  fill: none;
  stroke: var(--accent);
  stroke-width: var(--pot-track, 4px);
  stroke-linecap: round;
  transition: stroke 0.12s;
}
.pot.dragging .pot-fill { stroke: var(--accent-purple); }
.pot-needle {
  stroke: var(--text);
  stroke-width: 2px;
  stroke-linecap: round;
}
.pot-stack {
  display: flex;
  flex-direction: column;
  gap: 1px;
  align-items: center;
}
.pot-readout {
  font-family: var(--font-mono);
  font-size: 0.7rem;
  color: var(--accent);
  line-height: 1;
  text-align: center;
}
.pot-label {
  font-family: var(--font-display);
  font-size: 0.55rem;
  letter-spacing: 0.14em;
  color: var(--text-dim);
  text-transform: uppercase;
  line-height: 1;
}
`;

function injectStyles() {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = STYLES;
  document.head.appendChild(style);
}

const ARC_START_DEG = -135;
const ARC_END_DEG = 135;
const ARC_R = 40;

function polar(deg, r) {
  const a = ((deg - 90) * Math.PI) / 180;
  return [Math.cos(a) * r, Math.sin(a) * r];
}

function arcPath(fromDeg, toDeg, r) {
  const [x1, y1] = polar(fromDeg, r);
  const [x2, y2] = polar(toDeg, r);
  const large = Math.abs(toDeg - fromDeg) > 180 ? 1 : 0;
  const sweep = toDeg > fromDeg ? 1 : 0;
  return `M ${x1.toFixed(2)} ${y1.toFixed(2)} A ${r} ${r} 0 ${large} ${sweep} ${x2.toFixed(2)} ${y2.toFixed(2)}`;
}
const TRACK_PATH = arcPath(ARC_START_DEG, ARC_END_DEG, ARC_R);

export class Pot {
  /**
   * @param {HTMLElement} container
   * @param {object} cfg
   * @param {number} cfg.id       — CLAP param id (used in onChange events)
   * @param {number} cfg.min
   * @param {number} cfg.max
   * @param {number} cfg.default
   * @param {string} [cfg.unit]   — appended to readout
   * @param {boolean} [cfg.log]   — log-scaled range
   * @param {number} [cfg.scale]  — display multiplier (e.g. 100 for percent)
   * @param {string} [cfg.label]
   * @param {function} [cfg.format] — custom value→string
   */
  constructor(container, cfg) {
    injectStyles();
    this.id = cfg.id;
    this.min = cfg.min;
    this.max = cfg.max;
    this.def = cfg.default;
    this.unit = cfg.unit || '';
    this.log = !!cfg.log;
    this.scale = cfg.scale ?? 1;
    this.format = cfg.format || this.#defaultFormat.bind(this);
    this.value = this.def;
    this.listeners = new Set();

    this.el = document.createElement('div');
    this.el.className = 'pot';
    this.el.dataset.id = String(this.id);
    this.el.innerHTML = `
      <div class="pot-dial">
        <svg viewBox="-50 -50 100 100" aria-hidden="true">
          <path class="pot-track" d="${TRACK_PATH}"></path>
          <path class="pot-fill" d=""></path>
          <line class="pot-needle" x1="0" y1="0" x2="0" y2="-32"></line>
        </svg>
      </div>
      <div class="pot-stack">
        <div class="pot-readout"></div>
        <div class="pot-label">${cfg.label || ''}</div>
      </div>
    `;
    container.appendChild(this.el);

    this.dial = this.el.querySelector('.pot-dial');
    this.fillEl = this.el.querySelector('.pot-fill');
    this.needleEl = this.el.querySelector('.pot-needle');
    this.readoutEl = this.el.querySelector('.pot-readout');

    this.#wireInput();
    this.#render();
  }

  setValue(v) {
    v = Math.max(this.min, Math.min(this.max, v));
    if (v === this.value) return;
    this.value = v;
    this.#render();
  }

  setValueFromHost(v) {
    this.setValue(v);
  }

  onChange(cb) {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  #emit() {
    for (const cb of this.listeners) cb(this.value, this.id);
  }

  #setFromUI(v) {
    v = Math.max(this.min, Math.min(this.max, v));
    if (v === this.value) return;
    this.value = v;
    this.#render();
    this.#emit();
  }

  #valueToNorm(v) {
    if (this.log) {
      const lmin = Math.log(this.min);
      const lmax = Math.log(this.max);
      return (Math.log(v) - lmin) / (lmax - lmin);
    }
    return (v - this.min) / (this.max - this.min);
  }

  #normToValue(n) {
    n = Math.max(0, Math.min(1, n));
    if (this.log) {
      const lmin = Math.log(this.min);
      const lmax = Math.log(this.max);
      return Math.exp(lmin + n * (lmax - lmin));
    }
    return this.min + n * (this.max - this.min);
  }

  #defaultFormat(v) {
    if (this.unit === '%') return `${Math.round(v * 100)} %`;
    if (this.unit === 'Hz') return v >= 10 ? `${v.toFixed(1)} Hz` : `${v.toFixed(2)} Hz`;
    if (this.unit === 'dB') return `${v.toFixed(1)} dB`;
    return String(v.toFixed(2)) + (this.unit ? ' ' + this.unit : '');
  }

  #render() {
    const norm = Math.max(0, Math.min(1, this.#valueToNorm(this.value)));
    const deg = ARC_START_DEG + norm * (ARC_END_DEG - ARC_START_DEG);
    this.fillEl.setAttribute('d', arcPath(ARC_START_DEG, deg, ARC_R));
    this.needleEl.setAttribute('transform', `rotate(${deg.toFixed(2)})`);
    this.readoutEl.textContent = this.format(this.value);
  }

  #wireInput() {
    let dragStartY = 0;
    let dragStartNorm = 0;
    let pointerId = null;
    this.dial.addEventListener('pointerdown', e => {
      e.preventDefault();
      pointerId = e.pointerId;
      this.dial.setPointerCapture(pointerId);
      this.el.classList.add('dragging');
      dragStartY = e.clientY;
      dragStartNorm = this.#valueToNorm(this.value);
    });
    this.dial.addEventListener('pointermove', e => {
      if (pointerId === null) return;
      const dy = e.clientY - dragStartY;
      const range = e.shiftKey ? 960 : 240;
      const n = dragStartNorm - dy / range;
      this.#setFromUI(this.#normToValue(n));
    });
    const end = () => {
      if (pointerId !== null) {
        try { this.dial.releasePointerCapture(pointerId); } catch {}
        pointerId = null;
      }
      this.el.classList.remove('dragging');
    };
    this.dial.addEventListener('pointerup', end);
    this.dial.addEventListener('pointercancel', end);
    this.dial.addEventListener('dblclick', e => {
      e.preventDefault();
      this.#setFromUI(this.def);
    });
    this.dial.addEventListener('wheel', e => {
      e.preventDefault();
      const n = this.#valueToNorm(this.value);
      this.#setFromUI(this.#normToValue(n - Math.sign(e.deltaY) * 0.02));
    }, { passive: false });
  }
}
