var _damageBarTemplate = null;
function _getDamageBarTemplate() {
  if (!_damageBarTemplate && typeof document !== 'undefined') {
    _damageBarTemplate = document.createElement('template');
    _damageBarTemplate.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .bar-wrap { position: relative; width: 100%; height: 1.2em; background: #05080e; border: 1px solid #282c38; overflow: hidden; }
    .bar-wrap .fill { position: absolute; top: 0; left: 0; height: 100%; background: linear-gradient(90deg, #2a6838, #4ec870); transition: width 0.5s ease; }
    .bar-wrap .fill.warn { background: linear-gradient(90deg, #805818, #d8a040); }
    .bar-wrap .fill.crit { background: linear-gradient(90deg, #6a1a12, #e0402c); }
    .bar-wrap .label { position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; align-items: center; justify-content: center; font-size: 0.65rem; letter-spacing: 0.1em; color: #cce; text-shadow: 0 0 4px #000; pointer-events: none; }
  </style>
  <div class="bar-wrap">
    <div class="fill" id="bar-fill" style="width:100%"></div>
    <span class="label" id="bar-label">— / —</span>
  </div>
`;
  }
  return _damageBarTemplate;
}

export class PhDamageBar extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = _getDamageBarTemplate();
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this._data = null;
  }

  static get observedAttributes() { return ['data']; }

  attributeChangedCallback(name, _old, val) {
    if (name === 'data') {
      try {
        this._data = val ? JSON.parse(val) : null;
      } catch (_) {
        this._data = null;
      }
      this._render();
    }
  }

  set data(val) {
    this._data = val;
    this._render();
  }

  get data() { return this._data; }

  _render() {
    const d = this._data || {};
    const pct = d.pct != null ? d.pct : 1;
    const totalCurrent = d.totalCurrent != null ? d.totalCurrent : null;
    const totalMax = d.totalMax != null ? d.totalMax : null;

    const root = this.shadowRoot;
    const fill = root.getElementById('bar-fill');
    const label = root.getElementById('bar-label');

    const widthPct = Math.max(0, Math.min(1, pct)) * 100;
    fill.style.width = widthPct + '%';

    let cls = 'fill';
    if (pct < 0.4) cls += ' crit';
    else if (pct < 0.75) cls += ' warn';
    fill.className = cls;

    if (totalCurrent != null && totalMax != null) {
      label.textContent = Math.round(totalCurrent) + ' / ' + Math.round(totalMax);
    } else {
      label.textContent = '';
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-damage-bar')) {
  customElements.define('ph-damage-bar', PhDamageBar);
}
