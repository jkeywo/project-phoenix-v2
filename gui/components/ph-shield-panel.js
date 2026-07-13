export class PhShieldPanel extends HTMLElement {
  #state = null;
  #facingCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .header .v { color: var(--tactical); font-weight: 600; }
    .hull-row { display: flex; align-items: center; gap: 0.5rem; font-size: 0.65rem; }
    .hull-row .lbl { color: var(--ink-dim); min-width: 3rem; }
    .hull-row .bar-wrap { flex: 1; height: 0.7rem; background: var(--bg-deep); border: 1px solid var(--line-faint); position: relative; overflow: hidden; }
    .hull-row .bar-wrap .fill { position: absolute; inset: 0; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.5s ease; }
    .hull-row .bar-wrap .fill.warn { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .hull-row .bar-wrap .fill.crit { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .hull-row .val { min-width: 3rem; text-align: right; font-family: 'Chakra Petch', sans-serif; font-weight: 600; font-size: 0.9rem; }
    .facings { display: flex; flex-direction: column; gap: 0.25rem; }
    .facing-row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.6rem; }
    .facing-row .lbl { color: var(--ink-dim); min-width: 2.5rem; letter-spacing: 0.15em; }
    .facing-row .bar-wrap { flex: 1; height: 0.5rem; background: var(--bg-deep); border: 1px solid var(--line-faint); position: relative; overflow: hidden; }
    .facing-row .bar-wrap .fill { position: absolute; inset: 0; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.5s ease; }
    .facing-row .bar-wrap .fill.warn { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .facing-row .bar-wrap .fill.crit { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .facing-row .bar-wrap .fill.down { background: var(--line-faint); opacity: 0.4; }
    .facing-row .pct { min-width: 2rem; text-align: right; color: var(--ink-dim); }
    .status { font-size: 0.6rem; color: var(--ink-dim); letter-spacing: 0.2em; }
    .grid-online { color: var(--loaded); }
    .grid-offline { color: var(--fire); }
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
      <div class="bar-wrap"><div class="fill" id="panel-hull-fill" style="width:100%"></div></div>
      <span class="val" id="panel-hull-val">100%</span>
    </div>
  </div>
  <div class="facings" id="facings-container"></div>
  <div class="status" id="panel-focus-display">FOCUS: OMNI</div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const root = this.shadowRoot;

    const hullPct = s.hull_integrity_pct != null ? s.hull_integrity_pct : 100;
    const hullFill = root.getElementById('panel-hull-fill');
    hullFill.style.width = hullPct + '%';
    hullFill.className = 'fill' + (hullPct < 30 ? ' crit' : hullPct < 60 ? ' warn' : '');
    root.getElementById('panel-hull-val').textContent = Math.round(hullPct) + '%';

    const gridStatus = s.grid_status || 'GRID NOMINAL';
    const gs = root.getElementById('grid-status');
    gs.textContent = gridStatus;
    gs.className = gridStatus === 'GRID OFFLINE' ? 'grid-offline' : 'grid-online';

    const focusName = s.focused_facing || 'OMNI';
    root.getElementById('panel-focus-display').textContent = 'FOCUS: ' + focusName;

    const facings = s.facings || [];
    const container = root.getElementById('facings-container');

    const live = new Set(facings.map(f => f.label || ''));
    for (const [key, el] of this.#facingCache) {
      if (!live.has(key)) { el.remove(); this.#facingCache.delete(key); }
    }

    if (facings.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.style.cssText = 'font-size:0.6rem;color:var(--ink-dim);padding:0.5rem 0;text-align:center'; this.#emptyEl.textContent = 'NO SHIELD DATA'; container.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    facings.forEach(f => {
      const key = f.label || '';
      const pct = f.max_hp > 0 ? Math.round(f.hp / f.max_hp * 100) : 0;
      const cls = !f.online ? 'down' : pct > 60 ? '' : pct > 25 ? 'warn' : 'crit';
      const pctLabel = !f.online ? 'OFF' : pct + '%';
      let row = this.#facingCache.get(key);
      if (!row) {
        row = document.createElement('div');
        row.className = 'facing-row';
        row.innerHTML = '<span class="lbl"></span><div class="bar-wrap"><div class="fill"></div></div><span class="pct"></span>';
        this.#facingCache.set(key, row);
        container.appendChild(row);
      }
      row.children[0].textContent = (f.label || '?').substring(0, 4).toUpperCase();
      row.children[1].firstChild.className = 'fill ' + cls;
      row.children[1].firstChild.style.width = pct + '%';
      row.children[2].textContent = pctLabel;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-shield-panel')) {
  customElements.define('ph-shield-panel', PhShieldPanel);
}
