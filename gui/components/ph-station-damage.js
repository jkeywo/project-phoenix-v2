// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import './ph-damage-detail.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * ph-station-damage — compact per-station hull bar for a console footer.
 *
 * Fed the console's own aggregate hull (`own_hull` from
 * `aggregateStationHull`): `{ entries, totalCurrent, totalMax, pct, damagePct }`.
 * Shows a small integrity bar; tapping it opens a read-only popup listing the
 * station's individual system statuses via `ph-damage-detail`. When the station
 * has no damageable systems the whole element hides itself.
 *
 * Read-only: dispatching repair teams stays on the repair console (issue #12).
 */

/**
 * The label shown when the host console does not name the station.
 *
 * A function, not a constant: the template is built in the constructor, and
 * `t()` must be called after strings-boot has installed the table rather than
 * at module-evaluation time.
 */
const defaultLabel = () => t('component.station_damage.default_label');

export class PhStationDamage extends PhElement {
  #open = false;
  #onDocClick = null;

  template() {
    const label = defaultLabel();
    return `
  <style>
    :host { display: inline-flex; position: relative; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host([hidden]) { display: none; }
    :host * { box-sizing: border-box; }
    .bar {
      display: inline-flex; align-items: center; gap: 0.4rem; cursor: pointer;
      background: none; border: none; padding: 0; color: inherit; font: inherit; min-height: var(--control-hit-min);
    }
    .bar-label { font-size: var(--text-xs); letter-spacing: 0.15em; color: var(--ink-dim); text-transform: uppercase; white-space: nowrap; }
    .bar-wrap { position: relative; width: 90px; height: 0.7em; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .bar-wrap .fill { position: absolute; top: 0; left: 0; height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.4s ease; }
    .bar-wrap .fill.warn { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .bar-wrap .fill.crit { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .pct { font-size: var(--text-xs); color: var(--ink-dim); min-width: 2.2rem; text-align: right; }
    .caret { font-size: var(--text-xs); color: var(--ink-dim); }
    .popup {
      position: absolute; bottom: calc(100% + 8px); right: 0; z-index: 50;
      width: 260px; max-height: 40vh; overflow-y: auto;
      background: var(--surface-panel); border: 1px solid var(--cyan-dim); box-shadow: 0 6px 24px rgba(var(--rgb-void), 0.6);
      padding: 0.6rem; display: none;
    }
    .popup.open { display: block; }
    .popup-title { font-size: var(--text-xs); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.4rem; padding-bottom: 0.3rem; border-bottom: 1px solid var(--line-faint); }
  </style>
  <button class="bar" id="bar" type="button" aria-haspopup="true" aria-expanded="false" title="${t('component.station_damage.bar_title', { name: label })}">
    <span class="bar-label" id="bar-label">${label}</span>
    <span class="bar-wrap"><span class="fill" id="fill" style="width:100%"></span></span>
    <span class="pct" id="pct">—</span>
    <span class="caret">▲</span>
  </button>
  <div class="popup" id="popup">
    <div class="popup-title" id="popup-title">${t('component.station_damage.popup_title', { name: label })}</div>
    <ph-damage-detail id="detail"></ph-damage-detail>
  </div>
`;
  }

  static get observedAttributes() { return ['label']; }

  attributeChangedCallback(name, oldVal, newVal) {
    if (name === 'label') this.#applyLabel(newVal || defaultLabel());
  }

  #applyLabel(label) {
    const root = this.shadowRoot;
    root.getElementById('bar-label').textContent = label;
    root.getElementById('popup-title').textContent = t('component.station_damage.popup_title', { name: label });
    root.getElementById('bar').title = t('component.station_damage.bar_title', { name: label });
  }

  connectedCallback() {
    super.connectedCallback();
    const bar = this.shadowRoot.getElementById('bar');
    bar.addEventListener('click', (e) => { e.stopPropagation(); this.#toggle(); });
    // Close when clicking anywhere outside the popup.
    this.#onDocClick = () => { if (this.#open) this.#toggle(false); };
    document.addEventListener('click', this.#onDocClick);
    this.#applyLabel(this.getAttribute('label') || defaultLabel());
  }

  disconnectedCallback() {
    if (this.#onDocClick) { document.removeEventListener('click', this.#onDocClick); this.#onDocClick = null; }
  }

  #toggle(force) {
    this.#open = force === undefined ? !this.#open : !!force;
    const popup = this.shadowRoot.getElementById('popup');
    const bar = this.shadowRoot.getElementById('bar');
    popup.classList.toggle('open', this.#open);
    bar.setAttribute('aria-expanded', String(this.#open));
  }

  render(state) {
    const d = state || {};
    const entries = Array.isArray(d.entries) ? d.entries : [];

    // No damageable systems on this station → nothing to show.
    if (entries.length === 0) {
      this.hidden = true;
      this.#toggle(false);
      return;
    }
    this.hidden = false;

    const totalMax = d.totalMax != null ? d.totalMax : entries.reduce((s, e) => s + (e.max_hp || 0), 0);
    const totalCur = d.totalCurrent != null ? d.totalCurrent : entries.reduce((s, e) => s + (e.current || 0), 0);
    const pct = d.pct != null ? d.pct : (totalMax > 0 ? totalCur / totalMax : 1);

    const root = this.shadowRoot;
    const fill = root.getElementById('fill');
    fill.style.width = (Math.max(0, Math.min(1, pct)) * 100) + '%';
    let cls = 'fill';
    if (pct < 0.4) cls += ' crit';
    else if (pct < 0.75) cls += ' warn';
    fill.className = cls;

    root.getElementById('pct').textContent = Math.round(pct * 100) + '%';
    root.getElementById('detail').state = { entries };
  }
}

phDefine('ph-station-damage', PhStationDamage);
