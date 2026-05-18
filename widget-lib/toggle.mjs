// @ts-nocheck
import { PlinkenWidget } from './widget-base.mjs';
import { resolveMeta, KIND_DEFAULTS } from './utils.mjs';

export class PlinkenToggle extends PlinkenWidget {
  #meta = null;
  #on = false;
  #textOptions = null;
  #shadow = null;
  #thumb = null;
  #track = null;
  #readout = null;

  onMeta(meta) {
    const resolved = resolveMeta(meta, this, KIND_DEFAULTS.toggle);
    this.#meta = resolved;
    this.#textOptions = resolved.text ? String(resolved.text).split('|') : null;

    const label = this.getAttribute('label') || this.getAttribute('endpoint') || '';
    const accent = this.getAttribute('accent');
    if (accent) this.style.setProperty('--plk-accent', accent);

    this.setAttribute('role', 'switch');
    this.setAttribute('aria-label', label);
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');

    this.#shadow = this.attachShadow({ mode: 'open' });
    this.#shadow.innerHTML = `
      <style>
        :host {
          display: inline-flex;
          flex-direction: column;
          align-items: center;
          gap: 0.35em;
          cursor: pointer;
          user-select: none;
          -webkit-user-select: none;
          touch-action: none;
          outline: none;
        }
        :host(:focus-visible) .switch {
          outline: 1px solid var(--plk-accent);
          outline-offset: 2px;
        }
        .switch {
          width: 100%;
          height: 100%;
          display: block;
        }
        svg { display: block; width: 100%; height: 100%; }
        .track {
          fill: var(--plk-bg-deep);
          stroke: var(--plk-border);
          stroke-width: 1;
          transition: fill 120ms ease;
        }
        .thumb {
          fill: var(--plk-text);
          transition: transform 120ms ease;
          transform: translateX(0);
        }
        :host([data-on="1"]) .track { fill: var(--plk-accent); }
        :host([data-on="1"]) .thumb { transform: translateX(20px); }
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
          font-size: 0.6rem;
          color: var(--plk-text);
          line-height: 1;
        }
      </style>
      <svg class="switch" viewBox="0 0 40 20" preserveAspectRatio="none" aria-hidden="true">
        <rect class="track" x="0.5" y="0.5" width="39" height="19" rx="9.5" ry="9.5"/>
        <circle class="thumb" cx="10" cy="10" r="7"/>
      </svg>
      ${label ? `<span class="label">${escapeHtml(label)}</span>` : ''}
      ${this.#textOptions ? `<span class="readout"></span>` : ''}
    `;

    this.#track = this.#shadow.querySelector('.track');
    this.#thumb = this.#shadow.querySelector('.thumb');
    this.#readout = this.#shadow.querySelector('.readout');

    this.addEventListener('pointerdown', this.#onPointerDown);
    this.addEventListener('keydown', this.#onKeyDown);

    this.#applyState(resolved.init >= 0.5);
  }

  onValue(v) {
    this.#applyState(v >= 0.5);
  }

  #applyState(on) {
    this.#on = on;
    this.setAttribute('data-on', on ? '1' : '0');
    this.setAttribute('aria-checked', on ? 'true' : 'false');
    if (this.#readout && this.#textOptions) {
      const idx = on ? Math.min(1, this.#textOptions.length - 1) : 0;
      this.#readout.textContent = this.#textOptions[idx] ?? '';
    }
  }

  #toggle() {
    const next = this.#on ? 0 : 1;
    this.#applyState(next === 1);
    this.write(next, true);
  }

  #onPointerDown = (e) => {
    e.preventDefault();
    this.#toggle();
  };

  #onKeyDown = (e) => {
    if (e.key === ' ' || e.code === 'Space') {
      e.preventDefault();
      this.#toggle();
    } else if (e.key === 'Enter') {
      this.#toggle();
    }
  };
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[c]));
}

customElements.define('plinken-toggle', PlinkenToggle);
