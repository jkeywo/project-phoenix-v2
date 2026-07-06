var _damageDetailTemplate = null;
function _getDamageDetailTemplate() {
  if (!_damageDetailTemplate && typeof document !== 'undefined') {
    _damageDetailTemplate = document.createElement('template');
    _damageDetailTemplate.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.2rem; }
    .row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.65rem; }
    .row .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .bar-wrap { width: 4rem; height: 0.6rem; background: #05080e; border: 1px solid #282c38; position: relative; overflow: hidden; flex-shrink: 0; }
    .row .bar-wrap .fill { position: absolute; top: 0; left: 0; height: 100%; background: linear-gradient(90deg, #2a6838, #4ec870); }
    .row .bar-wrap .fill.warn { background: linear-gradient(90deg, #805818, #d8a040); }
    .row .bar-wrap .fill.crit { background: linear-gradient(90deg, #6a1a12, #e0402c); }
    .row .tier { font-size: 0.55rem; color: #6a7178; letter-spacing: 0.1em; min-width: 1.6rem; text-align: right; flex-shrink: 0; }
    .row.destroyed .name { color: #e0402c; letter-spacing: 0.15em; }
    .row.destroyed .bar-wrap .fill { background: #6a1a12; opacity: 0.5; }
    .destroyed-label { color: #e0402c; font-size: 0.55rem; letter-spacing: 0.2em; flex-shrink: 0; }
  </style>
  <div class="list" id="list"></div>
`;
  }
  return _damageDetailTemplate;
}

export class PhDamageDetail extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = _getDamageDetailTemplate();
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
    const entries = Array.isArray(d.entries) ? d.entries : [];
    const list = this.shadowRoot.getElementById('list');

    if (entries.length === 0) {
      list.innerHTML = '';
      return;
    }

    list.innerHTML = entries.map(e => {
      const max = e.max_hp != null ? e.max_hp : 0;
      const cur = e.current != null ? e.current : 0;
      const pct = max > 0 ? cur / max : 0;
      const destroyed = cur === 0;
      const widthPct = Math.max(0, Math.min(1, pct)) * 100;

      let fillCls = 'fill';
      if (pct < 0.4) fillCls += ' crit';
      else if (pct < 0.75) fillCls += ' warn';

      const tierLabel = e.tier != null ? 'T' + e.tier : '';
      const rowCls = destroyed ? 'row destroyed' : 'row';
      const nameLabel = e.display_name || '';

      const destroyedSpan = destroyed
        ? '<span class="destroyed-label">DESTROYED</span>'
        : '';

      return `<div class="${rowCls}">
        <span class="name">${nameLabel}</span>
        <div class="bar-wrap"><div class="${fillCls}" style="width:${widthPct}%"></div></div>
        ${destroyedSpan}
        <span class="tier">${tierLabel}</span>
      </div>`;
    }).join('');
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-damage-detail')) {
  customElements.define('ph-damage-detail', PhDamageDetail);
}
