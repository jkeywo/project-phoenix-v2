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
 *
 * ── Card art (PRD #1023 module 4, user story 3) ────────────────────────────
 *
 * "I want the ship picker to show me the ship I am choosing, so that the
 * choice feels meaningful rather than abstract." The cards showed a name, a
 * class badge, a hull number and two numbers — an accurate description of a
 * spreadsheet row.
 *
 * The picture comes from the capture pipeline that already runs: each playable
 * hull's rig sidecar ends its LOD ladder with a captured yaw-ring billboard
 * atlas, and scripts/ship-cards.mjs copies those four atlases into the dist at
 * build time with an index keyed by `template_path` (the wire carries no model
 * path, and entity stem does not reliably match model stem). One tile of the
 * strip is shown per card.
 *
 * The index is fetched once, lazily, and its absence is not an error: a mod
 * pack's hull was never scanned at build time and simply gets a card with no
 * portrait, exactly as before.
 */
// strings-boot first: its synchronous load delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is populated, so #render's t() calls never see an empty table. No-op
// under vitest, where setup-strings.js owns the table.
import '../strings-boot.js';
import { t, has } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';
import { installRovingTabindex, syncRovingTabindex } from '../roving-tabindex.js';

/**
 * Resolve `value` when it is a known string id; pass anything else through.
 * @param {string|null|undefined} value
 * @returns {string} the resolved text, or '' when there is nothing to show
 */
function localised(value) {
  if (!value) return '';
  return has(value) ? t(value) : String(value);
}

/**
 * The build-time `template_path` → card-art index, once fetched.
 *
 * Module-level so the two pages that mount this element (the phone lobby and
 * the host) each fetch it once. `{}` means "fetched, nothing there" — a dist
 * without the build step, which is not an error and must not retry per render.
 * @type {Object<string, {image: string, views: number, tile: number}>|null}
 */
let shipCards = null;
let shipCardsPending = null;

/**
 * Fetch the card index. Resolves to `{}` on any failure, so a missing file is
 * indistinguishable from an empty one at the call site — both mean "no art".
 * @returns {Promise<Object>}
 */
export function loadShipCards(fetchImpl) {
  if (shipCards) return Promise.resolve(shipCards);
  if (shipCardsPending) return shipCardsPending;
  const doFetch = fetchImpl || (typeof fetch === 'function' ? fetch : null);
  if (!doFetch) {
    shipCards = {};
    return Promise.resolve(shipCards);
  }
  shipCardsPending = Promise.resolve()
    .then(() => doFetch('assets/ship-cards/index.json'))
    .then((r) => (r && r.ok ? r.json() : {}))
    .catch(() => ({}))
    .then((json) => {
      shipCards = (json && typeof json === 'object') ? json : {};
      shipCardsPending = null;
      return shipCards;
    });
  return shipCardsPending;
}

/** Test seam: install a known index (or reset with `null`). */
export function setShipCards(index) {
  shipCards = index;
  shipCardsPending = null;
}

/**
 * The inline background style that shows tile `tile` of a `views`-wide strip.
 *
 * The strip is `views` square tiles packed left→right. Sizing it to
 * `views * 100%` of the box width makes one tile exactly as wide as the box;
 * the background-position percentage then resolves against
 * `(box width − image width)`, so tile `i` sits at `i / (views − 1)`.
 *
 * @returns {string} a `style="…"` value, or '' when there is no art
 */
export function shipArtStyle(card) {
  if (!card || !card.image || !(card.views > 1)) return '';
  const x = (card.tile / (card.views - 1)) * 100;
  return `background-image:url('${card.image}');`
    + `background-size:${card.views * 100}% auto;`
    + `background-position-x:${x.toFixed(4)}%;`;
}

export class PhShipPicker extends HTMLElement {
  #state = null;
  #roving = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; }
    :host * { box-sizing: border-box; }
    .ship-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; padding: 4px 0; }
    /* The card is a native <button role="option"> (issue #1178): focusable,
       named by its hull name, choosing the hull on Enter/Space through the SAME
       ship-selected event a pointer tap dispatches. The reset strips the
       browser button chrome so it still reads as a card. */
    .ship-card {
      width: 100%; margin: 0; font: inherit; text-align: left; -webkit-appearance: none; appearance: none;
      background: var(--surface-panel); border: 1px solid var(--cyan-dim); border-radius: 6px;
      padding: 12px 14px; cursor: pointer; transition: all 0.15s ease;
      display: flex; flex-direction: column; gap: 6px; min-height: var(--control-hit-min);
    }
    .ship-card:disabled { cursor: default; }
    .ship-card:hover { background: var(--surface-panel-up); border-color: var(--edge-control); }
    .ship-card:active { background: var(--surface-panel-up); border-color: var(--violet); }
    /* The hull itself (PRD #1023 user story 3). One yaw tile out of the
       captured billboard strip; shipArtStyle() computes the size and offset.
       Height is deliberately shorter than the square tile — every capture has
       generous empty sky above and below the hull, and cropping it is what
       makes the card read as a portrait rather than a stamp. */
    .ship-art {
      width: 100%; height: 88px;
      background-repeat: no-repeat;
      background-position-y: center;
      border-radius: 4px;
      background-color: var(--surface-abyss);
    }
    /* An unacknowledged pick (PRD #1023 module 4). The chosen card holds its
       accent and says the request is out; the rest recede and stop taking
       taps, because the host's arbiter ignores a second request anyway. */
    .ship-grid[data-busy="true"] .ship-card { opacity: 0.45; pointer-events: none; }
    .ship-grid[data-busy="true"] .ship-card.pending {
      opacity: 1; border-color: var(--signal); background: var(--surface-panel-up);
    }
    .ship-pending {
      font-size: var(--text-sm); color: var(--signal);
      letter-spacing: var(--tracking-wide); text-transform: uppercase;
    }
    .ship-name {
      font-family: 'Chakra Petch', sans-serif; font-size: var(--text-lg); font-weight: 600;
      color: var(--ink); letter-spacing: 0.06em; white-space: nowrap; overflow: hidden;
      text-overflow: ellipsis;
    }
    .ship-meta { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
    .ship-badge {
      font-size: var(--text-xs); letter-spacing: 0.18em; text-transform: uppercase;
      padding: 2px 7px; border-radius: 3px; font-weight: 600;
    }
    .ship-badge.battleship { background: rgba(var(--rgb-reloading), 0.15); border: 1px solid var(--reloading); color: var(--reloading); }
    .ship-badge.courier { background: rgba(var(--rgb-loaded), 0.15); border: 1px solid var(--loaded); color: var(--loaded); }
    .ship-badge.cruiser { background: rgba(var(--rgb-cyan), 0.15); border: 1px solid var(--cyan); color: var(--cyan); }
    .ship-badge.destroyer { background: rgba(var(--rgb-violet), 0.15); border: 1px solid var(--violet); color: var(--violet); }
    .ship-badge.unknown { background: rgba(var(--rgb-edge-strong), 0.15); border: 1px solid var(--edge-bright); color: var(--ink-dim); }
    .ship-hull-id { font-size: var(--text-sm); color: var(--edge-strong); letter-spacing: 0.04em; }
    .ship-stats { display: flex; gap: 12px; margin-top: 2px; }
    .ship-stat { display: flex; flex-direction: column; gap: 1px; }
    .ship-stat-label { font-size: var(--text-xs); letter-spacing: 0.15em; color: var(--edge-strong); text-transform: uppercase; }
    .ship-stat-value { font-size: var(--text-md); color: var(--ink-dim); }
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
    // Role + accessible name + keyboard operation (issue #1178). The cards were
    // clickable <div>s; the grid is now a listbox — one Tab stop, arrows roving
    // over the option cards — with the pending pick marked selected.
    this.setAttribute('role', 'listbox');
    this.setAttribute('aria-label', t('component.ship_picker.label'));
    this.#roving ??= installRovingTabindex(this, {
      getItems: () => this.#rovingItems(),
      orientation: 'both',
    });
    this.#syncRoving();
  }

  /** The grid's rovable option cards, in document order. */
  #rovingItems() {
    return Array.from(this.shadowRoot.querySelectorAll('.ship-card'));
  }

  /** Re-establish the single tab stop after a render rebuilds the grid. */
  #syncRoving() {
    syncRovingTabindex(this.#rovingItems());
  }

  set state(val) {
    this.#state = val;
    this.#render();
    // Art arrives on a second paint when the index has not been fetched yet.
    // The card is complete without it, so the first paint is never blocked.
    if (shipCards === null) {
      loadShipCards().then(() => {
        if (this.#state === val) this.#render();
      });
    }
  }

  get state() { return this.#state; }

  #render() {
    const grid = this.shadowRoot.getElementById('grid');
    const ships = this.#state?.ships ?? [];
    // An unacknowledged pick greys the grid and marks the chosen card; the
    // caller passes both (see gui/scenario-pick.js).
    const pendingTemplate = this.#state?.pendingTemplate ?? null;
    grid.dataset.busy = pendingTemplate ? 'true' : 'false';
    if (ships.length === 0) {
      grid.innerHTML = `<div style="color:var(--edge-strong);font-size:var(--text-md);padding:8px 0;">${t('component.ship_picker.empty')}</div>`;
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
      // The hull's portrait, when the build resolved one for this template.
      // `aria-hidden`: the card's own name is the accessible label, so the
      // picture is decoration and a screen reader should not announce it.
      const art = shipArtStyle(shipCards ? shipCards[ship.template_path] : null);
      const isPending = pendingTemplate === ship.template_path;
      // When a pick is out (busy), every card but the chosen one is disabled —
      // the keyboard equivalent of the `pointer-events: none` the CSS applies,
      // so an arrow never lands on a card that would not answer anyway.
      const disabled = pendingTemplate && !isPending ? ' disabled' : '';
      return `
  <button type="button" role="option" aria-selected="${isPending}" class="ship-card${isPending ? ' pending' : ''}" data-template="${ship.template_path}"${disabled}>
    ${art ? `<div class="ship-art" aria-hidden="true" style="${art}"></div>` : ''}
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
    ${isPending ? `<div class="ship-pending">${t('client.pick_pending')}</div>` : ''}
  </button>`;
    }).join('');

    grid.querySelectorAll('.ship-card').forEach(card => {
      // Enter/Space (native to the button) and a pointer tap alike run this one
      // handler, dispatching the SAME ship-selected event.
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
    this.#syncRoving();
  }
}

customElements.define('ph-ship-picker', PhShipPicker);