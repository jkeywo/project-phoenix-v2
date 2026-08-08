/**
 * gui/components/ph-ship-picker.js — the lobby hull chooser.
 *
 * Card text arrives as string ids, not English. A world's `[[available_ships]]
 * label` and an entity template's top-level `name` are both authored as
 * strings.csv ids (see scripts/strings-rules.mjs), and the hull `class` is a
 * machine token. A phone receives the catalog over the peer link, where
 * `localiseTree` has already resolved the ids; the HOST page reads the very
 * same catalog straight out of `wasm_get_scenario_catalog()` and never crosses
 * that wire boundary — so on the host every card read `world.combat_test.
 * available_ships.0.label` verbatim (issue #949).
 *
 * Resolving here fixes both transports at once, using the rule `localiseTree`
 * already uses: substitute only what the table actually holds. Text a phone
 * already resolved (and a mod pack's literal prose) is not an id, so it passes
 * through untouched.
 */
// strings-boot first: its synchronous load delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is populated, so #render's t() calls never see an empty table. No-op
// under vitest, where setup-strings.js owns the table.
import '../strings-boot.js';
import { t, has } from '../strings.js';

/**
 * Resolve `value` when it is a known string id; pass anything else through.
 * @param {string|null|undefined} value
 * @returns {string} the resolved text, or '' when there is nothing to show
 */
function localised(value) {
  if (!value) return '';
  return has(value) ? t(value) : String(value);
}

export class PhShipPicker extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; }
    :host * { box-sizing: border-box; }
    .ship-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; padding: 4px 0; }
    .ship-card {
      background: #111; border: 1px solid #334; border-radius: 6px;
      padding: 12px 14px; cursor: pointer; transition: all 0.15s ease;
      display: flex; flex-direction: column; gap: 6px;
    }
    .ship-card:hover { background: #1a1a2e; border-color: #558; }
    .ship-card:active { background: #1c2438; border-color: #66c; }
    .ship-name {
      font-family: 'Chakra Petch', sans-serif; font-size: 1rem; font-weight: 600;
      color: #d8e2ff; letter-spacing: 0.06em; white-space: nowrap; overflow: hidden;
      text-overflow: ellipsis;
    }
    .ship-meta { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
    .ship-badge {
      font-size: 0.6rem; letter-spacing: 0.18em; text-transform: uppercase;
      padding: 2px 7px; border-radius: 3px; font-weight: 600;
    }
    .ship-badge.battleship { background: rgba(208,160,48,0.15); border: 1px solid #d4a030; color: #d4a030; }
    .ship-badge.courier { background: rgba(96,190,140,0.15); border: 1px solid #60be8c; color: #60be8c; }
    .ship-badge.cruiser { background: rgba(108,182,208,0.15); border: 1px solid #6cb6d0; color: #6cb6d0; }
    .ship-badge.destroyer { background: rgba(140,100,200,0.15); border: 1px solid #8c64c8; color: #8c64c8; }
    .ship-badge.unknown { background: rgba(100,120,160,0.15); border: 1px solid #647ca0; color: #8a98c4; }
    .ship-hull-id { font-size: 0.7rem; color: #5a6694; letter-spacing: 0.04em; }
    .ship-stats { display: flex; gap: 12px; margin-top: 2px; }
    .ship-stat { display: flex; flex-direction: column; gap: 1px; }
    .ship-stat-label { font-size: 0.55rem; letter-spacing: 0.15em; color: #5a6694; text-transform: uppercase; }
    .ship-stat-value { font-size: 0.85rem; color: #8a98c4; }
    @media (max-width: 500px) {
      .ship-grid { grid-template-columns: 1fr; }
    }
  </style>
  <div class="ship-grid" id="grid"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const grid = this.shadowRoot.getElementById('grid');
    const ships = this.#state?.ships ?? [];
    if (ships.length === 0) {
      grid.innerHTML = `<div style="color:#5a6694;font-size:0.8rem;padding:8px 0;">${t('component.ship_picker.empty')}</div>`;
      return;
    }
    grid.innerHTML = ships.map(ship => {
      const name = localised(ship.label) || localised(ship.name)
        || ship.template_path.split('/').pop().replace('.toml', '');
      // `cls` stays the raw token: it is also the badge's CSS class. Only the
      // caption is localised, falling back to the token so a hull class with
      // no authored caption still reads (the same has()/t() shape as
      // gui/manual-panel.js ratingCaption).
      const cls = (ship.class || 'unknown').toLowerCase();
      const clsId = `component.ship_picker.class.${cls}`;
      const clsLabel = has(clsId) ? t(clsId) : cls;
      const hullId = ship.hull_id ? `#${ship.hull_id}` : '';
      const power = ship.power_rating != null ? `⚡${ship.power_rating}` : '';
      const stations = ship.station_count || '';
      return `
  <div class="ship-card" data-template="${ship.template_path}">
    <div class="ship-name">${name}</div>
    <div class="ship-meta">
      <span class="ship-badge ${cls}">${clsLabel}</span>
      ${hullId ? `<span class="ship-hull-id">${hullId}</span>` : ''}
    </div>
    ${(power || stations) ? `
    <div class="ship-stats">
      ${power ? `<div class="ship-stat"><span class="ship-stat-label">${t('component.ship_picker.power')}</span><span class="ship-stat-value">${ship.power_rating}</span></div>` : ''}
      ${stations ? `<div class="ship-stat"><span class="ship-stat-label">${t('component.ship_picker.stations')}</span><span class="ship-stat-value">${stations}</span></div>` : ''}
    </div>` : ''}
  </div>`;
    }).join('');

    grid.querySelectorAll('.ship-card').forEach(card => {
      card.addEventListener('click', () => {
        const templatePath = card.dataset.template;
        const ship = ships.find(s => s.template_path === templatePath);
        this.dispatchEvent(new CustomEvent('ship-selected', {
          bubbles: true,
          composed: true,
          detail: { template_path: templatePath, ship }
        }));
      });
    });
  }
}

customElements.define('ph-ship-picker', PhShipPicker);