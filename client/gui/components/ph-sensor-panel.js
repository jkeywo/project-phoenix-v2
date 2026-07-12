export class PhSensorPanel extends HTMLElement {
  #state = null;
  #scanRowCache = new Map();
  #noTargetEl = null;
  #targetCardEl = null;
  #targetNameEl = null;
  #badgesEl = null;
  #brgEl = null;
  #rngEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .header .v { color: var(--cyan); font-weight: 600; }
    .blip-count { font-size: 0.65rem; color: var(--ink-dim); }
    .target-card { background: var(--bg-card); border: 1px solid var(--line-faint); padding: 0.5rem; }
    .target-card .name { font-family: 'Chakra Petch', sans-serif; font-size: 1rem; font-weight: 600; color: var(--ink); letter-spacing: 0.07em; }
    .target-card .name.empty { font-size: 0.75rem; color: var(--ink-dim); letter-spacing: 0.28em; }
    .target-card .badges { display: flex; gap: 0.25rem; flex-wrap: wrap; margin-top: 0.25rem; }
    .target-card .badge { font-size: 0.55rem; letter-spacing: 0.18em; padding: 0.1rem 0.35rem; border: 1px solid; }
    .target-card .badge.hostile { color: var(--fire); border-color: #8a2a1e; }
    .target-card .badge.friendly { color: var(--loaded); border-color: var(--loaded-dim); }
    .target-card .badge.neutral { color: #6cb6d0; border-color: #3a5a68; }
    .target-card .pos-row { display: flex; gap: 0.75rem; margin-top: 0.4rem; padding-top: 0.3rem; border-top: 1px solid var(--line-faint); font-size: 0.65rem; }
    .target-card .pos-row .k { color: var(--ink-dim); }
    .target-card .pos-row .v { font-family: 'Chakra Petch', sans-serif; font-size: 1rem; font-weight: 600; color: var(--ink); }
    .target-card .pos-row .u { color: var(--ink-dim); font-size: 0.55rem; }
    .no-target { font-size: 0.7rem; color: var(--ink-dim); letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
    .scan-data { display: flex; flex-direction: column; gap: 0.2rem; font-size: 0.6rem; }
    .scan-row { display: flex; justify-content: space-between; padding: 0.2rem 0; border-bottom: 1px solid rgba(40,44,56,0.5); }
    .scan-row .k { color: var(--ink-dim); }
    .scan-row .v { color: var(--ink); }
    @media (orientation: portrait) {
      :host { gap: 0.35rem; }
      .target-card { padding: 0.35rem; }
      .target-card .name { font-size: 0.85rem; }
    }
  </style>
  <div class="header">
    <span>SCAN RANGE <span class="v" id="range-val">0</span></span>
    <span class="blip-count" id="blip-count">0 CONTACTS</span>
  </div>
  <div id="target-area">
    <div class="no-target" id="no-target">NO TARGET</div>
    <div class="target-card" id="target-card" style="display:none">
      <div class="name" id="target-name"></div>
      <div class="badges" id="badges"></div>
      <div class="pos-row">
        <div><span class="k">BRG</span> <span class="v" id="brg-val">—</span><span class="u">°</span></div>
        <div><span class="k">RNG</span> <span class="v" id="rng-val">—</span><span class="u">AU</span></div>
      </div>
    </div>
  </div>
  <div class="scan-data" id="scan-data"></div>
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

    root.getElementById('range-val').textContent = s.scan_range || 0;

    const blips = s.blips || [];
    root.getElementById('blip-count').textContent = blips.length + ' CONTACT' + (blips.length !== 1 ? 'S' : '');

    if (!this.#noTargetEl) this.#noTargetEl = root.getElementById('no-target');
    if (!this.#targetCardEl) this.#targetCardEl = root.getElementById('target-card');
    if (!this.#targetNameEl) this.#targetNameEl = root.getElementById('target-name');
    if (!this.#badgesEl) this.#badgesEl = root.getElementById('badges');
    if (!this.#brgEl) this.#brgEl = root.getElementById('brg-val');
    if (!this.#rngEl) this.#rngEl = root.getElementById('rng-val');

    const hasTarget = !!s.target_uuid;
    this.#noTargetEl.style.display = hasTarget ? 'none' : '';
    this.#targetCardEl.style.display = hasTarget ? '' : 'none';
    root.getElementById('target-area').dataset.hasTarget = String(hasTarget);

    if (hasTarget) {
      this.#targetNameEl.textContent = s.target_name || s.target_uuid;

      const kind = s.target_kind || 'unknown';
      const stance = s.target_stance || 'neutral';
      const stanceClass = { hostile: 'hostile', friendly: 'friendly', allied: 'friendly', neutral: 'neutral' }[stance] || 'neutral';
      const stanceLabel = { hostile: 'HOSTILE', friendly: 'ALLIED', allied: 'ALLIED', neutral: 'NEUTRAL' }[stance] || 'UNKNOWN';
      const kindLabel = { ship: 'WARSHIP', asteroid: 'ASTEROID', station: 'STARBASE', planet: 'PLANET', star: 'STAR' }[kind] || kind.toUpperCase();

      this.#badgesEl.innerHTML = '<span class="badge"></span><span class="badge neutral"></span>';
      this.#badgesEl.children[0].className = 'badge ' + stanceClass;
      this.#badgesEl.children[0].textContent = stanceLabel;
      this.#badgesEl.children[1].textContent = kindLabel;

      this.#brgEl.textContent = s.target_bearing != null ? s.target_bearing.toFixed(1) : '—';
      this.#rngEl.textContent = s.target_range != null ? Math.round(s.target_range) : '—';
    }

    const sd = root.getElementById('scan-data');
    const scanRows = [];
    if (s.target_class) scanRows.push({ k: 'CLASS', v: s.target_class });
    if (s.target_hull_pct != null) scanRows.push({ k: 'HULL', v: Math.round(s.target_hull_pct) + '%' });
    if (s.target_heading != null) scanRows.push({ k: 'HEADING', v: s.target_heading.toFixed(0) + '°' });
    if (s.target_speed != null) scanRows.push({ k: 'SPEED', v: s.target_speed.toFixed(1) + ' kn' });
    if (s.target_threat) scanRows.push({ k: 'THREAT', v: s.target_threat.toUpperCase() });

    if (scanRows.length > 0) {
      const live = new Set(scanRows.map(r => r.k));
      for (const [key, el] of this.#scanRowCache) {
        if (!live.has(key)) { el.remove(); this.#scanRowCache.delete(key); }
      }
      scanRows.forEach(r => {
        let row = this.#scanRowCache.get(r.k);
        if (!row) {
          row = document.createElement('div');
          row.className = 'scan-row';
          row.innerHTML = '<span class="k"></span><span class="v"></span>';
          this.#scanRowCache.set(r.k, row);
          sd.appendChild(row);
        }
        row.children[0].textContent = r.k;
        row.children[1].textContent = r.v;
      });
    } else {
      let scanning = this.#scanRowCache.get('__scanning');
      if (!scanning) {
        scanning = document.createElement('div');
        scanning.className = 'scan-row';
        scanning.innerHTML = '<span class="k">STATUS</span><span class="v dim">SCANNING...</span>';
        this.#scanRowCache.set('__scanning', scanning);
        sd.appendChild(scanning);
      }
      for (const [key, el] of this.#scanRowCache) {
        if (key !== '__scanning') { el.remove(); this.#scanRowCache.delete(key); }
      }
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-sensor-panel')) {
  customElements.define('ph-sensor-panel', PhSensorPanel);
}
