// Boolean toggle widget — used for stereo link, bypass, etc.
//
// Internally a 0/1 valued control so it round-trips through `clap.params`
// like any other automatable param.
//
// Usage:
//   const t = new Toggle(el, { id: 3, default: 1, label: 'Link' });
//   t.onChange(v => sendSet(t.id, v));   // v is 0 or 1

const STYLE_ID = 'plinken-toggle-style';

const STYLES = `
.toggle {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}
.toggle-button {
  width: var(--toggle-w, 36px);
  height: var(--toggle-h, 20px);
  background: var(--bg-deep);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  position: relative;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s;
  user-select: none;
}
.toggle-button::after {
  content: '';
  position: absolute;
  top: 1px;
  left: 1px;
  width: calc(var(--toggle-h, 20px) - 4px);
  height: calc(var(--toggle-h, 20px) - 4px);
  background: var(--text-dim);
  border-radius: 50%;
  transition: transform 0.12s, background 0.12s;
}
.toggle[data-on="1"] .toggle-button {
  background: var(--accent);
  border-color: var(--accent-deep);
}
.toggle[data-on="1"] .toggle-button::after {
  background: var(--text);
  transform: translateX(calc(var(--toggle-w, 36px) - var(--toggle-h, 20px)));
}
.toggle-label {
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

export class Toggle {
  /**
   * @param {HTMLElement} container
   * @param {object} cfg
   * @param {number} cfg.id
   * @param {number} [cfg.default=0]  — 0 or 1
   * @param {string} [cfg.label]
   */
  constructor(container, cfg) {
    injectStyles();
    this.id = cfg.id;
    this.def = cfg.default ? 1 : 0;
    this.value = this.def;
    this.listeners = new Set();

    this.el = document.createElement('div');
    this.el.className = 'toggle';
    this.el.dataset.id = String(this.id);
    this.el.dataset.on = String(this.value);
    this.el.innerHTML = `
      <div class="toggle-button" role="switch" aria-checked="${this.value === 1}"></div>
      <div class="toggle-label">${cfg.label || ''}</div>
    `;
    container.appendChild(this.el);

    this.button = this.el.querySelector('.toggle-button');
    this.button.addEventListener('click', () => this.#setFromUI(this.value ? 0 : 1));
  }

  setValue(v) {
    const next = v >= 0.5 ? 1 : 0;
    if (next === this.value) return;
    this.value = next;
    this.el.dataset.on = String(next);
    this.button.setAttribute('aria-checked', next === 1 ? 'true' : 'false');
  }

  setValueFromHost(v) {
    this.setValue(v);
  }

  onChange(cb) {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  #setFromUI(v) {
    this.setValue(v);
    for (const cb of this.listeners) cb(this.value, this.id);
  }
}
