// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * `hide-shield-rows` (boolean attribute): suppresses the SHIELD and SHIELD
 * FREQ scan rows this panel would otherwise render (issue #927 gave every
 * embedder these rows for free, but the battleship's dedicated Sensors
 * console already has its own richer Target Analysis shield readout —
 * `gui/battleship/sensors.html`'s `renderShieldFacings()` plus its own
 * Shield Freq metric — so without this flag the battleship showed shields
 * twice). Set on the element in markup, e.g.
 * `<ph-sensor-panel hide-shield-rows>`; every other hull leaves it unset and
 * keeps the rows.
 */
export class PhSensorPanel extends PhElement {
  #scanRowCache = new Map();
  #noTargetEl = null;
  #targetCardEl = null;
  #targetNameEl = null;
  #badgesEl = null;
  #brgEl = null;
  #rngEl = null;

  static get observedAttributes() { return ['hide-shield-rows']; }

  attributeChangedCallback(name) {
    if (name === 'hide-shield-rows') this.render(this.state);
  }

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .header .v { color: var(--cyan); font-weight: 600; }
    .blip-count { font-size: var(--text-xs); color: var(--ink-dim); }
    .target-card { background: var(--bg-card); border: 1px solid var(--line-faint); padding: 0.5rem; }
    .target-card .name { font-family: 'Chakra Petch', sans-serif; font-size: var(--text-lg); font-weight: 600; color: var(--ink); letter-spacing: 0.07em; }
    .target-card .name.empty { font-size: var(--text-sm); color: var(--ink-dim); letter-spacing: 0.28em; }
    .target-card .badges { display: flex; gap: 0.25rem; flex-wrap: wrap; margin-top: 0.25rem; }
    .target-card .badge { font-size: var(--text-xs); letter-spacing: 0.18em; padding: 0.1rem 0.35rem; border: 1px solid; }
    .target-card .badge.hostile { color: var(--fire); border-color: var(--fire-dim); }
    .target-card .badge.friendly { color: var(--loaded); border-color: var(--loaded-dim); }
    .target-card .badge.neutral { color: var(--cyan); border-color: var(--edge); }
    .target-card .pos-row { display: flex; gap: 0.75rem; margin-top: 0.4rem; padding-top: 0.3rem; border-top: 1px solid var(--line-faint); font-size: var(--text-xs); }
    .target-card .pos-row .k { color: var(--ink-dim); }
    .target-card .pos-row .v { font-family: 'Chakra Petch', sans-serif; font-size: var(--text-lg); font-weight: 600; color: var(--ink); }
    .target-card .pos-row .u { color: var(--ink-dim); font-size: var(--text-xs); }
    .no-target { font-size: var(--text-sm); color: var(--ink-dim); letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
    .scan-data { display: flex; flex-direction: column; gap: 0.2rem; font-size: var(--text-xs); }
    .scan-row { display: flex; justify-content: space-between; padding: 0.2rem 0; border-bottom: 1px solid rgba(var(--rgb-panel-up), 0.5); }
    .scan-row .k { color: var(--ink-dim); }
    .scan-row .v { color: var(--ink); }
    @media (orientation: portrait) {
      :host { gap: 0.35rem; }
      .target-card { padding: 0.35rem; }
      .target-card .name { font-size: var(--text-md); }
    }
  </style>
  <div class="header">
    <span>${t('component.sensor_panel.scan_range')} <span class="v" id="range-val">0</span></span>
    <span class="blip-count" id="blip-count">${t('console.common.contacts.other', { n: 0 })}</span>
  </div>
  <div id="target-area">
    <div class="no-target" id="no-target">${t('console.common.no_target')}</div>
    <div class="target-card" id="target-card" style="display:none">
      <div class="name" id="target-name"></div>
      <div class="badges" id="badges"></div>
      <div class="pos-row">
        <div><span class="k">${t('component.sensor_panel.brg')}</span> <span class="v" id="brg-val">—</span><span class="u">°</span></div>
        <div><span class="k">${t('component.sensor_panel.rng')}</span> <span class="v" id="rng-val">—</span><span class="u">${t('component.sensor_panel.au')}</span></div>
      </div>
    </div>
  </div>
  <div class="scan-data" id="scan-data"></div>
`;
  }

  render(state) {
    const s = state || {};
    const root = this.shadowRoot;

    root.getElementById('range-val').textContent = s.scan_range || 0;

    const blips = s.blips || [];
    root.getElementById('blip-count').textContent = blips.length === 1
      ? t('console.common.contacts.one', { n: 1 })
      : t('console.common.contacts.other', { n: blips.length });

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
      const stanceLabel = {
        hostile: t('console.stance.hostile'),
        friendly: t('console.stance.allied'),
        allied: t('console.stance.allied'),
        neutral: t('console.stance.neutral'),
      }[stance] || t('console.common.unknown');
      const kindLabel = {
        ship: t('console.kind.ship'),
        asteroid: t('console.kind.asteroid'),
        station: t('console.kind.station'),
        planet: t('console.kind.planet'),
        star: t('console.kind.star'),
      }[kind] || kind.toUpperCase();

      this.#badgesEl.innerHTML = '<span class="badge"></span><span class="badge neutral"></span>';
      this.#badgesEl.children[0].className = 'badge ' + stanceClass;
      this.#badgesEl.children[0].textContent = stanceLabel;
      this.#badgesEl.children[1].textContent = kindLabel;

      this.#brgEl.textContent = s.target_bearing != null ? s.target_bearing.toFixed(1) : '—';
      this.#rngEl.textContent = s.target_range != null ? Math.round(s.target_range) : '—';
    }

    const sd = root.getElementById('scan-data');
    const scanRows = [];
    if (s.target_class) scanRows.push({ k: t('component.sensor_panel.class'), v: s.target_class });
    if (s.target_hull_pct != null) scanRows.push({ k: t('component.sensor_panel.hull'), v: Math.round(s.target_hull_pct) + '%' });
    if (s.target_heading != null) scanRows.push({ k: t('component.sensor_panel.heading'), v: s.target_heading.toFixed(0) + '°' });
    if (s.target_speed != null) scanRows.push({ k: t('component.sensor_panel.speed'), v: s.target_speed.toFixed(1) + ' kn' });
    if (s.target_threat) scanRows.push({ k: t('component.sensor_panel.threat'), v: s.target_threat.toUpperCase() });
    // Selected-target red alert (issue #749). Only present for Red-Alert-capable
    // ship targets; `null` (non-ship/incapable/no selection) hides the row.
    if (s.target_alert != null) scanRows.push({ k: t('component.sensor_panel.alert'), v: s.target_alert ? t('component.sensor_panel.alert_active') : t('component.sensor_panel.alert_standby') });

    // Target shields (issue #927). `target_shields`, `target_shield_fraction`
    // and `target_shield_freq` are on every Sensors payload already
    // (gui/console-state.js buildSensorsConsoleState) but this shared panel —
    // the one the destroyer, cruiser and courier all embed — used to drop
    // all three on the floor, so `fact(target_facing_shields)` and the
    // FrequencyHint's sender-side origin were invisible on three of four
    // hulls. Same classification `gui/battleship/sensors.html`'s
    // renderShieldFacings() applies (per-facing hp/max_hp, `online === false`
    // means down), so the two surfaces agree on the same payload — no second
    // derivation, no divergent formatting rule. Degrades to no row at all
    // when there is no target or the target has no shields, matching the
    // no-target-shields case at gui/battleship/sensors.html:78.
    //
    // `hide-shield-rows` (issue #927 duplication fix): the battleship sets
    // this attribute because its dedicated Target Analysis section already
    // shows the same facts (renderShieldFacings() + its own Shield Freq
    // metric) — without the guard both surfaces rendered on that one console.
    if (!this.hasAttribute('hide-shield-rows')) {
      const shields = s.target_shields || [];
      if (shields.length > 0) {
        const summary = shields.map((f) => {
          const pct = f.max_hp > 0 ? Math.round((f.hp / f.max_hp) * 100) : 0;
          const label = (f.label || '?').toUpperCase();
          return label + ' ' + (f.online === false ? t('console.shield.down') : pct + '%');
        }).join(' · ');
        scanRows.push({ k: t('component.sensor_panel.shield'), v: summary });
      } else if (s.target_shield_fraction != null) {
        const pct = Math.max(0, Math.round(s.target_shield_fraction * 100));
        scanRows.push({
          k: t('component.sensor_panel.shield'),
          v: pct > 0 ? pct + '%' : t('console.shield.down'),
        });
      }
      if (s.target_shield_freq != null) {
        scanRows.push({
          k: t('component.sensor_panel.shield_freq'),
          v: Math.round(s.target_shield_freq * 100) + '%',
        });
      }
    }

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
        scanning.innerHTML = '<span class="k">' + t('component.sensor_panel.status') + '</span><span class="v dim">' + t('component.sensor_panel.scanning') + '</span>';
        this.#scanRowCache.set('__scanning', scanning);
        sd.appendChild(scanning);
      }
      for (const [key, el] of this.#scanRowCache) {
        if (key !== '__scanning') { el.remove(); this.#scanRowCache.delete(key); }
      }
    }
  }
}

phDefine('ph-sensor-panel', PhSensorPanel);
