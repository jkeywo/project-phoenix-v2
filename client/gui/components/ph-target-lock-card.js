// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * gui/components/ph-target-lock-card.js — the Tactical target lock card
 * (issue #1378).
 *
 * Shows the identity and tactical facts about whatever this seat's Tactical
 * lock currently holds — name, stance, class, bearing, range, hull integrity,
 * shield facings and shield frequency — fed by the shared `targetFactsFor`
 * helper (`gui/console-state.js`) off the SAME lock the radar and weapon
 * panels already fire at (`gui/stations/tactical-console.js`). The card takes
 * no selection of its own: it only ever reads `state.target_uuid` and its
 * siblings.
 *
 * Deliberately narrower than `ph-sensor-panel`'s target card, which this
 * mirrors the styling of: no kind badge, no heading/speed/threat/alert/
 * weapons rows. Those are Sensors' own scan-derived facts — not part of this
 * card's acceptance criteria — and Sensors' privacy boundary
 * (`target_alert`/`target_weapons` come off THAT ship's own SensorRadar
 * blackboard) has no equivalent for a Tactical lock.
 */
export class PhTargetLockCard extends PhElement {
  #scanRowCache = new Map();

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.4rem; }
    .no-target { font-size: var(--text-sm); color: var(--ink-dim); letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
    .card { background: var(--bg-card); border: 1px solid var(--line-faint); padding: 0.5rem; }
    .name { font-family: 'Chakra Petch', sans-serif; font-size: var(--text-lg); font-weight: 600; color: var(--ink); letter-spacing: 0.07em; }
    .badges { display: flex; gap: 0.25rem; flex-wrap: wrap; margin-top: 0.25rem; }
    .badge { font-size: var(--text-xs); letter-spacing: 0.18em; padding: 0.1rem 0.35rem; border: 1px solid; }
    .badge.hostile { color: var(--fire); border-color: var(--fire-dim); }
    .badge.friendly { color: var(--loaded); border-color: var(--loaded-dim); }
    .badge.neutral { color: var(--cyan); border-color: var(--edge); }
    .pos-row { display: flex; gap: 0.75rem; margin-top: 0.4rem; padding-top: 0.3rem; border-top: 1px solid var(--line-faint); font-size: var(--text-xs); }
    .pos-row .k { color: var(--ink-dim); }
    .pos-row .v { font-family: 'Chakra Petch', sans-serif; font-size: var(--text-lg); font-weight: 600; color: var(--ink); }
    .pos-row .u { color: var(--ink-dim); font-size: var(--text-xs); }
    .scan-data { display: flex; flex-direction: column; gap: 0.2rem; font-size: var(--text-xs); margin-top: 0.3rem; }
    .scan-row { display: flex; justify-content: space-between; padding: 0.2rem 0; border-bottom: 1px solid rgba(var(--rgb-panel-up), 0.5); }
    .scan-row .k { color: var(--ink-dim); }
    .scan-row .v { color: var(--ink); }
    @media (orientation: portrait) {
      :host { gap: 0.3rem; }
      .card { padding: 0.35rem; }
      .name { font-size: var(--text-md); }
      :host([portrait-strip]) .card {
        display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
        column-gap: 0.5rem;
      }
      :host([portrait-strip]) .name { grid-column: 1; overflow-wrap: anywhere; }
      :host([portrait-strip]) .badges { grid-column: 1; }
      :host([portrait-strip]) .pos-row {
        grid-column: 2; grid-row: 1 / 3; flex-direction: column;
        margin: 0; padding: 0; border: 0; gap: 0.2rem;
      }
      :host([portrait-strip]) .scan-data {
        grid-column: 1 / -1; flex-direction: row; flex-wrap: wrap; gap: 0.2rem 0.65rem;
      }
      :host([portrait-strip]) .scan-row { gap: 0.4rem; border: 0; padding: 0; }
      :host([portrait-strip]) .scan-row .v { text-align: right; overflow-wrap: anywhere; }

    }
  </style>
  <div class="header">${t('component.target_lock_card.title')}</div>
  <div class="no-target" id="no-target">${t('console.common.no_target')}</div>
  <div class="card" id="card" style="display:none">
    <div class="name" id="name"></div>
    <div class="badges" id="badges"></div>
    <div class="pos-row">
      <div><span class="k">${t('component.sensor_panel.brg')}</span> <span class="v" id="brg-val">—</span><span class="u">°</span></div>
      <div><span class="k">${t('component.sensor_panel.rng')}</span> <span class="v" id="rng-val">—</span><span class="u">${t('component.sensor_panel.au')}</span></div>
    </div>
    <div class="scan-data" id="scan-data"></div>
  </div>
`;
  }

  render(state) {
    const s = state || {};
    const root = this.shadowRoot;
    const hasTarget = !!s.target_uuid;

    root.getElementById('no-target').style.display = hasTarget ? 'none' : '';
    root.getElementById('card').style.display = hasTarget ? '' : 'none';

    if (!hasTarget) {
      // A cleared lock starts the next one from an empty row cache rather
      // than showing a frame of the previous target's stale rows.
      for (const [, row] of this.#scanRowCache) row.remove();
      this.#scanRowCache.clear();
      return;
    }

    root.getElementById('name').textContent = s.target_name || s.target_uuid;

    const stance = s.target_stance || 'neutral';
    const stanceClass = { hostile: 'hostile', friendly: 'friendly', allied: 'friendly', neutral: 'neutral' }[stance] || 'neutral';
    const stanceLabel = {
      hostile: t('console.stance.hostile'),
      friendly: t('console.stance.allied'),
      allied: t('console.stance.allied'),
      neutral: t('console.stance.neutral'),
    }[stance] || t('console.common.unknown');
    const badges = root.getElementById('badges');
    badges.innerHTML = '<span class="badge"></span>';
    badges.children[0].className = 'badge ' + stanceClass;
    badges.children[0].textContent = stanceLabel;

    root.getElementById('brg-val').textContent = s.target_bearing != null ? s.target_bearing.toFixed(1) : '—';
    root.getElementById('rng-val').textContent = s.target_range != null ? Math.round(s.target_range) : '—';

    // Shield facings (issue #927's classification, reused here rather than
    // re-derived): per-facing hp/max_hp, `online === false` means down.
    const scanRows = [];
    if (s.target_class) scanRows.push({ k: t('component.sensor_panel.class'), v: s.target_class });
    if (s.target_hull_pct != null) scanRows.push({ k: t('component.sensor_panel.hull'), v: Math.round(s.target_hull_pct) + '%' });

    const shields = s.target_shields || [];
    if (shields.length > 0) {
      const summary = shields.map((f) => {
        const pct = f.max_hp > 0 ? Math.round((f.hp / f.max_hp) * 100) : 0;
        const label = (f.label || '?').toUpperCase();
        return label + ' ' + (f.online === false ? t('console.shield.down') : pct + '%');
      }).join(' · ');
      scanRows.push({ k: t('component.sensor_panel.shield'), v: summary });
    }
    if (s.target_shield_freq != null) {
      scanRows.push({
        k: t('component.sensor_panel.shield_freq'),
        v: Math.round(s.target_shield_freq * 100) + '%',
      });
    }

    const sd = root.getElementById('scan-data');
    const live = new Set(scanRows.map(r => r.k));
    for (const [key, row] of this.#scanRowCache) {
      if (!live.has(key)) { row.remove(); this.#scanRowCache.delete(key); }
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
  }
}

phDefine('ph-target-lock-card', PhTargetLockCard);
