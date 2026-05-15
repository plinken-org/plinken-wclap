// Vertical fader — port of synome's VerticalFader.svelte to plain ESM,
// with the value-range + log scaling + readout/label pattern of `pot.mjs`.
//
// Usage:
//   const f = new Fader(el, { id: 1, min: -24, max: 0, default: -1,
//                             unit: 'dB', label: 'Threshold', height: 120 });
//   f.onChange(v => sendSet(f.id, v));
//   f.setValue(-3);                  // programmatic (no event emit)

const STYLE_ID = 'plinken-fader-style';

const STYLES = `
.fader {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  touch-action: none;
}
.fader-track {
  position: relative;
  width: var(--fader-thumb, 20px);
  height: var(--fader-h, 120px);
  cursor: ns-resize;
}
.fader-track::before {
  content: '';
  position: absolute;
  left: 50%;
  top: 0;
  bottom: 0;
  width: var(--fader-track-w, 4px);
  transform: translateX(-50%);
  background: var(--bg-deep);
  border-radius: 2px;
}
.fader-fill {
  position: absolute;
  left: 50%;
  bottom: 0;
  width: var(--fader-track-w, 4px);
  transform: translateX(-50%);
  background: var(--accent);
  border-radius: 2px;
  transition: background 0.12s;
}
.fader.dragging .fader-fill { background: var(--accent-purple); }
.fader-thumb {
  position: absolute;
  left: 50%;
  transform: translate(-50%, -50%);
  width: var(--fader-thumb, 20px);
  height: var(--fader-thumb-h, 10px);
  background: var(--text);
  border-radius: 2px;
  box-shadow: 0 0 0 1px var(--border-soft);
  pointer-events: none;
}
.fader.dragging .fader-thumb { background: var(--accent); }
.fader-stack {
  display: flex;
  flex-direction: column;
  gap: 1px;
  align-items: center;
}
.fader-readout {
  font-family: var(--font-mono);
  font-size: 0.7rem;
  color: var(--accent);
  line-height: 1;
  text-align: center;
  /* Keep width independent of value width so the readout text growing
     (e.g. "10 ms" → "500 ms") doesn't widen the fader column and shift
     surrounding widgets. Tabular nums stabilises digit width; min-width
     reserves space for the widest expected reading. */
  min-width: 7ch;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.fader-label {
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

export class Fader {
  /**
   * @param {HTMLElement} container
   * @param {object} cfg
   * @param {number} cfg.id
   * @param {number} cfg.min
   * @param {number} cfg.max
   * @param {number} cfg.default
   * @param {string} [cfg.unit]
   * @param {boolean} [cfg.log]
   * @param {string} [cfg.label]
   * @param {function} [cfg.format]
   * @param {number} [cfg.height=120]
   */
  constructor(container, cfg) {
    injectStyles();
    this.id = cfg.id;
    this.min = cfg.min;
    this.max = cfg.max;
    this.def = cfg.default;
    this.unit = cfg.unit || '';
    this.log = !!cfg.log;
    this.format = cfg.format || this.#defaultFormat.bind(this);
    this.value = this.def;
    this.listeners = new Set();

    this.el = document.createElement('div');
    this.el.className = 'fader';
    this.el.dataset.id = String(this.id);
    if (cfg.height) this.el.style.setProperty('--fader-h', cfg.height + 'px');

    this.el.innerHTML = `
      <div class="fader-readout"></div>
      <div class="fader-track">
        <div class="fader-fill"></div>
        <div class="fader-thumb"></div>
      </div>
      <div class="fader-label">${cfg.label || ''}</div>
    `;
    container.appendChild(this.el);

    this.track = this.el.querySelector('.fader-track');
    this.fillEl = this.el.querySelector('.fader-fill');
    this.thumbEl = this.el.querySelector('.fader-thumb');
    this.readoutEl = this.el.querySelector('.fader-readout');

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
    if (this.unit === 'dB') return `${v.toFixed(1)} dB`;
    if (this.unit === 'ms') return `${v.toFixed(1)} ms`;
    if (this.unit === 'Hz') return v >= 1000 ? `${(v / 1000).toFixed(1)} kHz` : `${v.toFixed(1)} Hz`;
    return v.toFixed(2) + (this.unit ? ' ' + this.unit : '');
  }

  #render() {
    const norm = Math.max(0, Math.min(1, this.#valueToNorm(this.value)));
    const pct = (norm * 100).toFixed(1);
    this.fillEl.style.height = pct + '%';
    // Position the thumb so its bounding box stays inside the track:
    // when norm=1 the thumb's top edge sits at the track top (not its
    // center), so it doesn't bleed up into the readout. Combined with
    // the `translateY(-50%)` in the thumb CSS, the center lands at
    // `thumbH/2 + (1-norm) * (trackH - thumbH)`.
    const t = (1 - norm).toFixed(4);
    this.thumbEl.style.top =
      `calc(${t} * (100% - var(--fader-thumb-h, 10px)) + var(--fader-thumb-h, 10px) / 2)`;
    this.readoutEl.textContent = this.format(this.value);
  }

  #wireInput() {
    let pointerId = null;
    const updateFromY = clientY => {
      const rect = this.track.getBoundingClientRect();
      const relY = clientY - rect.top;
      const norm = 1 - relY / rect.height;
      this.#setFromUI(this.#normToValue(norm));
    };
    this.track.addEventListener('pointerdown', e => {
      e.preventDefault();
      pointerId = e.pointerId;
      this.track.setPointerCapture(pointerId);
      this.el.classList.add('dragging');
      updateFromY(e.clientY);
    });
    this.track.addEventListener('pointermove', e => {
      if (pointerId === null) return;
      updateFromY(e.clientY);
    });
    const end = () => {
      if (pointerId !== null) {
        try { this.track.releasePointerCapture(pointerId); } catch {}
        pointerId = null;
      }
      this.el.classList.remove('dragging');
    };
    this.track.addEventListener('pointerup', end);
    this.track.addEventListener('pointercancel', end);
    this.track.addEventListener('dblclick', e => {
      e.preventDefault();
      this.#setFromUI(this.def);
    });
    this.track.addEventListener('wheel', e => {
      e.preventDefault();
      const n = this.#valueToNorm(this.value);
      this.#setFromUI(this.#normToValue(n - Math.sign(e.deltaY) * 0.02));
    }, { passive: false });
  }
}
