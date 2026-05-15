// Vertical level meter — port of synome's Peakmeter.svelte in plain ESM.
//
// Two modes:
//   - 'peak'  : level meter, fills bottom→top, hot/clip color zones
//   - 'gr'    : gain reduction, fills top→bottom (0 dB at top), accent-purple
//
// Values are set via `setDb(dbfs)`. Internally rendered as 0..1 normalized
// against `[minDb, maxDb]`. A peak-hold marker decays at `decayRate` per
// requestAnimationFrame tick.
//
// CSS: shipped inline via `Meter.injectStyles()` (called automatically on
// first construction). Plugins only pay the CSS cost if they use the widget.

const STYLE_ID = 'plinken-meter-style';

const STYLES = `
.meter {
  position: relative;
  width: var(--meter-w, 12px);
  height: var(--meter-h, 120px);
  background: var(--meter-track);
  border-radius: 2px;
  overflow: hidden;
  flex: 0 0 auto;
}
.meter-fill {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  background: var(--meter-fill-ok);
  transition: height 0.05s linear;
}
.meter[data-mode="gr"] .meter-fill {
  top: 0;
  bottom: auto;
  background: var(--meter-fill-gr);
}
.meter[data-hot="1"] .meter-fill { background: var(--meter-fill-hot); }
.meter[data-clip="1"] .meter-fill { background: var(--meter-fill-clip); }
.meter-peak {
  position: absolute;
  left: 0;
  right: 0;
  height: 2px;
  background: var(--text);
  pointer-events: none;
}
.meter[data-mode="gr"] .meter-peak { background: var(--accent-purple); }
.meter-stack {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}
.meter-stack .label {
  font-size: 0.55rem;
  letter-spacing: 0.12em;
  color: var(--text-dim);
  text-transform: uppercase;
}
.meter-stack .readout {
  font-size: 0.65rem;
  color: var(--accent);
}
`;

function injectStyles() {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = STYLES;
  document.head.appendChild(style);
}

export class Meter {
  /**
   * @param {HTMLElement} container — element to render into (we replace its content)
   * @param {object} cfg
   * @param {'peak'|'gr'} [cfg.mode='peak']
   * @param {number} [cfg.minDb=-60]  — bottom of the scale
   * @param {number} [cfg.maxDb=0]    — top of the scale
   * @param {number} [cfg.hotDb=-6]   — color shifts to "hot" above this
   * @param {number} [cfg.clipDb=0]   — color shifts to "clip" at/above this
   * @param {number} [cfg.decayRate=0.005]  — peak-hold decay per frame (0..1)
   * @param {string} [cfg.label]
   * @param {boolean} [cfg.showReadout=true]
   */
  constructor(container, cfg = {}) {
    injectStyles();
    this.mode = cfg.mode || 'peak';
    this.minDb = cfg.minDb ?? -60;
    this.maxDb = cfg.maxDb ?? 0;
    this.hotDb = cfg.hotDb ?? -6;
    this.clipDb = cfg.clipDb ?? 0;
    this.decayRate = cfg.decayRate ?? 0.005;
    this.showReadout = cfg.showReadout !== false;

    this.currentNorm = 0;
    this.peakNorm = 0;
    this.currentDb = -Infinity;

    const stack = document.createElement('div');
    stack.className = 'meter-stack';
    container.appendChild(stack);

    this.bar = document.createElement('div');
    this.bar.className = 'meter';
    this.bar.dataset.mode = this.mode;
    this.fillEl = document.createElement('div');
    this.fillEl.className = 'meter-fill';
    this.peakEl = document.createElement('div');
    this.peakEl.className = 'meter-peak';
    this.bar.appendChild(this.fillEl);
    this.bar.appendChild(this.peakEl);
    stack.appendChild(this.bar);

    if (this.showReadout) {
      this.readoutEl = document.createElement('div');
      this.readoutEl.className = 'readout';
      this.readoutEl.textContent = this.mode === 'gr' ? '0 dB' : '−∞';
      stack.appendChild(this.readoutEl);
    }
    if (cfg.label) {
      const lab = document.createElement('div');
      lab.className = 'label';
      lab.textContent = cfg.label;
      stack.appendChild(lab);
    }

    this.#tick = this.#tick.bind(this);
    this.#raf = requestAnimationFrame(this.#tick);
  }

  /**
   * Set the current level in dBFS. Peak meters: positive numbers indicate
   * how close to 0 dBFS; -∞ means silence. GR meters: how many dB the
   * limiter is pulling down (always negative or 0; 0 = no reduction).
   */
  setDb(dbfs) {
    this.currentDb = dbfs;
    let norm;
    if (this.mode === 'gr') {
      // GR: 0 dB = no reduction (norm 0), -maxRange = full reduction (norm 1).
      // Map (0 .. minDb) → (0 .. 1).
      const range = this.minDb; // e.g. -60
      norm = range === 0 ? 0 : Math.min(1, Math.max(0, dbfs / range));
    } else {
      // Peak: minDb (silence) → 0, maxDb (full) → 1.
      norm = (dbfs - this.minDb) / (this.maxDb - this.minDb);
      norm = Math.min(1, Math.max(0, norm));
    }
    this.currentNorm = norm;
    if (norm > this.peakNorm) this.peakNorm = norm;
    this.bar.dataset.hot = dbfs >= this.hotDb && this.mode === 'peak' ? '1' : '0';
    this.bar.dataset.clip = dbfs >= this.clipDb && this.mode === 'peak' ? '1' : '0';
  }

  setLinear(amp) {
    this.setDb(amp > 0 ? 20 * Math.log10(amp) : -Infinity);
  }

  destroy() {
    if (this.#raf) cancelAnimationFrame(this.#raf);
    this.#raf = 0;
  }

  #raf = 0;
  #tick() {
    this.peakNorm = Math.max(this.currentNorm, this.peakNorm - this.decayRate);
    const pct = (this.currentNorm * 100).toFixed(1);
    this.fillEl.style.height = pct + '%';
    const peakPct = (this.peakNorm * 100).toFixed(1);
    if (this.mode === 'gr') {
      this.peakEl.style.top = `calc(${peakPct}% - 1px)`;
    } else {
      this.peakEl.style.bottom = `calc(${peakPct}% - 1px)`;
    }
    if (this.readoutEl) {
      if (!isFinite(this.currentDb)) {
        this.readoutEl.textContent = '−∞';
      } else if (this.mode === 'gr') {
        this.readoutEl.textContent = this.currentDb >= -0.05
          ? '0 dB'
          : `${this.currentDb.toFixed(1)} dB`;
      } else {
        this.readoutEl.textContent = `${this.currentDb.toFixed(1)} dB`;
      }
    }
    this.#raf = requestAnimationFrame(this.#tick);
  }
}
