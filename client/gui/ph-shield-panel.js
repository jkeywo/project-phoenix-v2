var _shieldTemplate = null;
function _getShieldTemplate() {
  if (!_shieldTemplate && typeof document !== 'undefined') {
    _shieldTemplate = document.createElement('template');
    _shieldTemplate.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; }
    .header .v { color: #f08438; font-weight: 600; }
    .hull-row { display: flex; align-items: center; gap: 0.5rem; font-size: 0.65rem; }
    .hull-row .lbl { color: #6a7178; min-width: 3rem; }
    .hull-row .bar-wrap { flex: 1; height: 0.7rem; background: #05080e; border: 1px solid #282c38; position: relative; overflow: hidden; }
    .hull-row .bar-wrap .fill { position: absolute; inset: 0; background: linear-gradient(90deg, #2a6838, #4ec870); transition: width 0.5s ease; }
    .hull-row .bar-wrap .fill.warn { background: linear-gradient(90deg, #805818, #d8a040); }
    .hull-row .bar-wrap .fill.crit { background: linear-gradient(90deg, #6a1a12, #e0402c); }
    .hull-row .val { min-width: 3rem; text-align: right; font-family: 'Chakra Petch', sans-serif; font-weight: 600; font-size: 0.9rem; }
    .facings { display: flex; flex-direction: column; gap: 0.25rem; }
    .facing-row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.6rem; }
    .facing-row .lbl { color: #6a7178; min-width: 2.5rem; letter-spacing: 0.15em; }
    .facing-row .bar-wrap { flex: 1; height: 0.5rem; background: #05080e; border: 1px solid #282c38; position: relative; overflow: hidden; }
    .facing-row .bar-wrap .fill { position: absolute; inset: 0; background: linear-gradient(90deg, #2a6838, #4ec870); transition: width 0.5s ease; }
    .facing-row .bar-wrap .fill.warn { background: linear-gradient(90deg, #805818, #d8a040); }
    .facing-row .bar-wrap .fill.crit { background: linear-gradient(90deg, #6a1a12, #e0402c); }
    .facing-row .bar-wrap .fill.down { background: #282c38; opacity: 0.4; }
    .facing-row .pct { min-width: 2rem; text-align: right; color: #6a7178; }
    .status { font-size: 0.6rem; color: #6a7178; letter-spacing: 0.2em; }
    .grid-online { color: #4ec870; }
    .grid-offline { color: #e0402c; }
    @media (orientation: portrait) {
      .hull-row .val { font-size: 0.75rem; }
    }
  </style>
  <div class="header">
    <span>SHIELDS</span>
    <span class="v" id="grid-status">GRID NOMINAL</span>
  </div>
  <div id="hull-section">
    <div class="hull-row">
      <span class="lbl">INTEGRITY</span>
      <div class="bar-wrap"><div class="fill" id="hull-fill" style="width:100%"></div></div>
      <span class="val" id="hull-val">100%</span>
    </div>
  </div>
  <div class="facings" id="facings-container"></div>
  <div class="status" id="focus-display">FOCUS: OMNI</div>
`;
  }
  return _shieldTemplate;
}

export class PhShieldPanel extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = _getShieldTemplate();
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this._state = null;
  }

  set state(val) {
    this._state = val;
    this._render();
  }

  get state() { return this._state; }

  _render() {
    const s = this._state || {};
    const root = this.shadowRoot;

    const hullPct = s.hull_integrity_pct != null ? s.hull_integrity_pct : 100;
    const hullFill = root.getElementById('hull-fill');
    hullFill.style.width = hullPct + '%';
    hullFill.className = 'fill' + (hullPct < 30 ? ' crit' : hullPct < 60 ? ' warn' : '');
    root.getElementById('hull-val').textContent = Math.round(hullPct) + '%';

    const gridStatus = s.grid_status || 'GRID NOMINAL';
    const gs = root.getElementById('grid-status');
    gs.textContent = gridStatus;
    gs.className = gridStatus === 'GRID OFFLINE' ? 'grid-offline' : 'grid-online';

    const focusName = s.focused_facing || 'OMNI';
    root.getElementById('focus-display').textContent = 'FOCUS: ' + focusName;

    const facings = s.facings || [];
    const container = root.getElementById('facings-container');
    if (facings.length === 0) {
      container.innerHTML = '<div style="font-size:0.6rem;color:#6a7178;padding:0.5rem 0;text-align:center">NO SHIELD DATA</div>';
      return;
    }

    container.innerHTML = facings.map(f => {
      const pct = f.max_hp > 0 ? Math.round(f.hp / f.max_hp * 100) : 0;
      const cls = !f.online ? 'down' : pct > 60 ? '' : pct > 25 ? 'warn' : 'crit';
      const pctLabel = !f.online ? 'OFF' : pct + '%';
      return `<div class="facing-row">
        <span class="lbl">${(f.label || '?').substring(0, 4).toUpperCase()}</span>
        <div class="bar-wrap"><div class="fill ${cls}" style="width:${pct}%"></div></div>
        <span class="pct">${pctLabel}</span>
      </div>`;
    }).join('');
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-shield-panel')) {
  customElements.define('ph-shield-panel', PhShieldPanel);
}
