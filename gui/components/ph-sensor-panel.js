export class PhSensorPanel extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; }
    .header .v { color: #5fd8e8; font-weight: 600; }
    .blip-count { font-size: 0.65rem; color: #6a7178; }
    .target-card { background: #0e1117; border: 1px solid #282c38; padding: 0.5rem; }
    .target-card .name { font-family: 'Chakra Petch', sans-serif; font-size: 1rem; font-weight: 600; color: #cce; letter-spacing: 0.07em; }
    .target-card .name.empty { font-size: 0.75rem; color: #6a7178; letter-spacing: 0.28em; }
    .target-card .badges { display: flex; gap: 0.25rem; flex-wrap: wrap; margin-top: 0.25rem; }
    .target-card .badge { font-size: 0.55rem; letter-spacing: 0.18em; padding: 0.1rem 0.35rem; border: 1px solid; }
    .target-card .badge.hostile { color: #e0402c; border-color: #8a2a1e; }
    .target-card .badge.friendly { color: #4ec870; border-color: #2a6838; }
    .target-card .badge.neutral { color: #6cb6d0; border-color: #3a5a68; }
    .target-card .pos-row { display: flex; gap: 0.75rem; margin-top: 0.4rem; padding-top: 0.3rem; border-top: 1px solid #282c38; font-size: 0.65rem; }
    .target-card .pos-row .k { color: #6a7178; }
    .target-card .pos-row .v { font-family: 'Chakra Petch', sans-serif; font-size: 1rem; font-weight: 600; color: #cce; }
    .target-card .pos-row .u { color: #6a7178; font-size: 0.55rem; }
    .no-target { font-size: 0.7rem; color: #6a7178; letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
    .scan-data { display: flex; flex-direction: column; gap: 0.2rem; font-size: 0.6rem; }
    .scan-row { display: flex; justify-content: space-between; padding: 0.2rem 0; border-bottom: 1px solid rgba(40,44,56,0.5); }
    .scan-row .k { color: #6a7178; }
    .scan-row .v { color: #cce; }
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
  <div id="target-area"></div>
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

    const range = s.scan_range || 0;
    root.getElementById('range-val').textContent = range;

    const blips = s.blips || [];
    root.getElementById('blip-count').textContent = blips.length + ' CONTACT' + (blips.length !== 1 ? 'S' : '');

    const targetArea = root.getElementById('target-area');
    const hasTarget = !!s.target_uuid;
    if (!hasTarget) {
      targetArea.innerHTML = '<div class="no-target">NO TARGET</div>';
      root.getElementById('scan-data').innerHTML = '';
      return;
    }

    const kind = s.target_kind || 'unknown';
    const stance = s.target_stance || 'neutral';
    const stanceClass = { hostile: 'hostile', friendly: 'friendly', allied: 'friendly', neutral: 'neutral' }[stance] || 'neutral';
    const stanceLabel = { hostile: 'HOSTILE', friendly: 'ALLIED', allied: 'ALLIED', neutral: 'NEUTRAL' }[stance] || 'UNKNOWN';
    const kindLabel = { ship: 'WARSHIP', asteroid: 'ASTEROID', station: 'STARBASE', planet: 'PLANET', star: 'STAR' }[kind] || kind.toUpperCase();

    targetArea.innerHTML = `
      <div class="target-card">
        <div class="name">${s.target_name || s.target_uuid}</div>
        <div class="badges">
          <span class="badge ${stanceClass}">${stanceLabel}</span>
          <span class="badge neutral">${kindLabel}</span>
        </div>
        <div class="pos-row">
          <div><span class="k">BRG</span> <span class="v">${s.target_bearing != null ? s.target_bearing.toFixed(1) : '—'}</span><span class="u">°</span></div>
          <div><span class="k">RNG</span> <span class="v">${s.target_range != null ? Math.round(s.target_range) : '—'}</span><span class="u">AU</span></div>
        </div>
      </div>
    `;

    const scanRows = [];
    if (s.target_class) scanRows.push({ k: 'CLASS', v: s.target_class });
    if (s.target_hull_pct != null) scanRows.push({ k: 'HULL', v: Math.round(s.target_hull_pct) + '%' });
    if (s.target_heading != null) scanRows.push({ k: 'HEADING', v: s.target_heading.toFixed(0) + '°' });
    if (s.target_speed != null) scanRows.push({ k: 'SPEED', v: s.target_speed.toFixed(1) + ' kn' });
    if (s.target_threat) scanRows.push({ k: 'THREAT', v: s.target_threat.toUpperCase() });
    const sd = root.getElementById('scan-data');
    if (scanRows.length > 0) {
      sd.innerHTML = scanRows.map(r => `<div class="scan-row"><span class="k">${r.k}</span><span class="v">${r.v}</span></div>`).join('');
    } else {
      sd.innerHTML = '<div class="scan-row"><span class="k">STATUS</span><span class="v dim">SCANNING...</span></div>';
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-sensor-panel')) {
  customElements.define('ph-sensor-panel', PhSensorPanel);
}
