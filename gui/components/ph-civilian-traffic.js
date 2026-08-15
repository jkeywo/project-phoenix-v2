// Civilian traffic readout for the Navigation console (issue #1028).
//
// Sits under the nav chart: the chart says where the traffic IS, this says what
// each craft was told and whether it is doing it. Those are different questions
// and the second one cannot be answered by watching a blip — "it has not started
// turning yet" and "it has decided not to" look identical on a map.
//
// COMPLIANCE IS NOT INFERRED HERE. Every field arrives already derived from the
// authoritative per-entity state; this component formats what it was handed and
// runs no clock, no interpolation and no guesswork of its own. In particular
// `refused` and `non_compliant` are DIFFERENT rows on purpose: the first is a
// craft that declined and carried on with its own lane, the second is one that
// agreed and then got stuck — the one that actually needs a crew.
//
// strings-boot first: its top-level await delays this module's evaluation — and
// therefore this element's registration and upgrade — until the string table is
// loaded, so the constructor's template t() calls never see an empty table.
// No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

/**
 * The `strings.csv` id for a compliance state word.
 *
 * A closed table rather than string interpolation, so a state the server grows
 * later renders as a visible ⟨id⟩ miss instead of silently reading as blank.
 * @param {string} state
 */
export function complianceLabel(state) {
  return `component.civilians.compliance.${state || 'unordered'}`;
}

/**
 * One craft's lane position as `leg / legs`, or an empty string when it has no
 * standing lane.
 *
 * Digits and a slash only — no localisable text. `leg` is zero-based on the
 * wire (it is a cursor index) and one-based here, because a crew counts legs
 * from one.
 * @param {{route?: string, leg?: number, legs?: number}} row
 */
export function formatLeg(row) {
  const legs = row.legs || 0;
  if (!row.route || legs === 0) return '';
  return `${Math.min((row.leg || 0) + 1, legs)}/${legs}`;
}

export class PhCivilianTraffic extends HTMLElement {
  #state = null;
  #rowCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .heading { font-size: 0.6rem; letter-spacing: 0.2em; color: var(--ink-dim); padding: 0 0.2rem 0.3rem; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.5rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: baseline; gap: 0.5rem; font-size: 0.7rem; line-height: 1.3; border-radius: 2px; padding: 0.1rem 0.2rem; }
    .row .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .leg { flex-shrink: 0; font-variant-numeric: tabular-nums; color: var(--ink-dim); }
    .row .state { flex-shrink: 0; letter-spacing: 0.1em; color: var(--cyan); }
    .row.pending .state { color: var(--ink-dim); }
    .row.refused .state { color: var(--amber, #d4a820); }
    .row.stuck { background: #2a1a1a; border-left: 2px solid var(--amber, #d4a820); }
    .row.stuck .state { color: var(--ink); }
  </style>
  <div class="heading" id="heading"></div>
  <div class="list" id="list"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    this.shadowRoot.getElementById('heading').textContent = t('component.civilians.heading');
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const raw = Array.isArray(s.civilians) ? s.civilians : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map((c) => c.uuid || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) {
        this.#emptyEl = document.createElement('div');
        this.#emptyEl.className = 'empty';
        this.#emptyEl.textContent = t('component.civilians.empty');
        list.appendChild(this.#emptyEl);
      }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    raw.forEach((c) => {
      const key = c.uuid || '';
      const compliance = c.compliance || 'unordered';
      let el = this.#rowCache.get(key);
      if (!el) {
        el = document.createElement('div');
        el.innerHTML = '<span class="name"></span><span class="leg"></span><span class="state"></span>';
        this.#rowCache.set(key, el);
        list.appendChild(el);
      }
      el.className = [
        'row',
        (compliance === 'received' || compliance === 'acknowledged') && 'pending',
        compliance === 'refused' && 'refused',
        compliance === 'non_compliant' && 'stuck',
      ].filter(Boolean).join(' ');
      // The name is a strings.csv id, resolved here — no English crosses the
      // wire. An id with no row renders as ⟨id⟩ via t()'s own miss reporting.
      el.children[0].textContent = c.name ? t(c.name) : key;
      el.children[1].textContent = formatLeg(c);
      el.children[2].textContent = t(complianceLabel(compliance));
      // The reason a craft gave for refusing, or the one the world gave for it
      // being stuck. Both are strings.csv ids; a row with neither has no title.
      if (c.reason) el.title = t(c.reason); else el.removeAttribute('title');
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-civilian-traffic')) {
  customElements.define('ph-civilian-traffic', PhCivilianTraffic);
}
