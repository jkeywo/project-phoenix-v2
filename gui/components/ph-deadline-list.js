// Mission-deadline countdown readout (issue #1024).
//
// Sits alongside <ph-objective-list> on the captain console: objectives are what
// the crew is being asked to do, deadlines are what the mission is doing to them
// regardless. Only the deadlines a world authored as `visible = true` ever reach
// this payload — an invisible deadline is a mission keeping a clock to itself,
// and the server-side filter is what keeps it there.
//
// THE COUNTDOWN IS NOT CLIENT-SIDE. `remaining_secs` arrives already computed
// against the authoritative `SimTick` and is re-published every tick, so this
// component formats a number it was handed and never runs a timer of its own. A
// client-side clock would drift away from the tick the deadline actually fires
// on and would show two players different numbers off the same mission.
//
// strings-boot first: its top-level await delays this module's evaluation — and
// therefore this element's registration and upgrade — until the string table is
// loaded, so the constructor's template t() calls never see an empty table.
// No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * Format whole seconds as `M:SS`, or `H:MM:SS` past an hour.
 *
 * Digits and colons only — no localisable text, so nothing here needs a
 * strings.csv row. Negative input (the server's "no deadline" sentinel) never
 * reaches this: a cancelled deadline renders its state word instead.
 * @param {number} secs
 */
export function formatCountdown(secs) {
  const total = Math.max(0, Math.floor(secs));
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const pad = (n) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

export class PhDeadlineList extends PhElement {
  #rowCache = new Map();
  #emptyEl = null;

  template() {
    return `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .heading { font-size: var(--text-xs); letter-spacing: 0.2em; color: var(--ink-dim); padding: 0 0.2rem 0.3rem; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.5rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: baseline; gap: 0.5rem; font-size: var(--text-sm); line-height: 1.3; border-radius: 2px; padding: 0.1rem 0.2rem; }
    .row .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .clock { flex-shrink: 0; font-variant-numeric: tabular-nums; letter-spacing: 0.05em; color: var(--cyan); }
    .row.spent .label { text-decoration: line-through; color: var(--ink-dim); }
    .row.spent .clock { color: var(--ink-dim); }
    .row.fired { background: var(--reloading-deep); border-left: 2px solid var(--ink-dim); }
    .row.fired .clock { color: var(--ink); }
  </style>
  <div class="heading" id="heading"></div>
  <div class="list" id="list"></div>
`;
  }

  onTemplate() {
    this.$('heading').textContent = t('component.deadlines.heading');
  }

  render(state) {
    const s = state || {};
    const raw = Array.isArray(s.deadlines) ? s.deadlines : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map((d) => d.id || d.label || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) {
        this.#emptyEl = document.createElement('div');
        this.#emptyEl.className = 'empty';
        this.#emptyEl.textContent = t('component.deadlines.empty');
        list.appendChild(this.#emptyEl);
      }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    raw.forEach((d) => {
      const key = d.id || d.label || '';
      const state = d.state || 'pending';
      let el = this.#rowCache.get(key);
      if (!el) {
        el = document.createElement('div');
        el.innerHTML = '<span class="label"></span><span class="clock"></span>';
        this.#rowCache.set(key, el);
        list.appendChild(el);
      }
      el.className = ['row', state !== 'pending' && 'spent', state === 'fired' && 'fired']
        .filter(Boolean).join(' ');
      // The label is a strings.csv id, resolved here — no English crosses the
      // wire. An id with no row renders as ⟨id⟩ via t()'s own miss reporting.
      el.firstChild.textContent = d.label ? t(d.label) : key;
      el.lastChild.textContent = state === 'cancelled'
        ? t('component.deadlines.cancelled')
        : state === 'fired'
          ? t('component.deadlines.fired')
          : formatCountdown(d.remaining_secs ?? 0);
    });
  }
}

phDefine('ph-deadline-list', PhDeadlineList);
