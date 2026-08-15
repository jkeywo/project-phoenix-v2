import { phAdoptConsoleStyles } from './ph-console-styles.js';
export class PhDamageBar extends HTMLElement {
  #state = null;

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
    .bar-wrap { position: relative; width: 100%; height: 1.2em; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .bar-wrap .fill { position: absolute; top: 0; left: 0; height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.5s ease; }
    .bar-wrap .fill.warn { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .bar-wrap .fill.crit { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    /* Destroyed capability (issue #1014): capacity that is gone, not merely
       damaged. Anchored to the RIGHT — the segment lost off the top of the bar —
       and hatched so it reads as a distinct band rather than as the crit fill
       colour bleeding across the whole bar. */
    /* Deliberately no transition here, unlike .fill above: capability loss is instantaneous, so .lost snaps to its new width; the 0.5s glide is only for ordinary HP movement. */
    .bar-wrap .lost { position: absolute; top: 0; right: 0; height: 100%; border-left: 1px solid var(--fire); background: repeating-linear-gradient(135deg, var(--fire-dim) 0 3px, var(--fire) 3px 6px); }
    .bar-wrap .label { position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; align-items: center; justify-content: center; font-size: var(--text-xs); letter-spacing: 0.1em; color: var(--ink); text-shadow: 0 0 4px var(--surface-void); pointer-events: none; }
  </style>
  <div class="bar-wrap">
    <div class="fill" id="bar-fill" style="width:100%"></div>
    <div class="lost" id="bar-lost" style="display:none;width:0%"></div>
    <span class="label" id="bar-label">— / —</span>
  </div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const d = this.#state || {};
    const pct = d.pct != null ? d.pct : 1;
    const totalCurrent = d.totalCurrent != null ? d.totalCurrent : null;
    const totalMax = d.totalMax != null ? d.totalMax : null;

    const root = this.shadowRoot;
    const fill = root.getElementById('bar-fill');
    const lost = root.getElementById('bar-lost');
    const label = root.getElementById('bar-label');

    const widthPct = Math.max(0, Math.min(1, pct)) * 100;
    fill.style.width = widthPct + '%';

    let cls = 'fill';
    if (pct < 0.4) cls += ' crit';
    else if (pct < 0.75) cls += ' warn';
    fill.className = cls;

    // `destroyed` is a host-supplied ship-wide share (issue #1014), never
    // derived here: it is the fraction of total capacity held by destroyed
    // systems, painted as a band at the right end. It is independent of the
    // fill/warn/crit state above, which still reads the remaining-hull `pct`.
    const destroyed = typeof d.destroyed === 'number' && Number.isFinite(d.destroyed)
      ? Math.max(0, Math.min(1, d.destroyed))
      : 0;
    if (destroyed > 0) {
      lost.style.display = 'block';
      lost.style.width = (destroyed * 100) + '%';
    } else {
      lost.style.display = 'none';
      lost.style.width = '0%';
    }

    if (totalCurrent != null && totalMax != null) {
      label.textContent = Math.round(totalCurrent) + ' / ' + Math.round(totalMax);
    } else {
      // No HP totals supplied (e.g. hull integrity is fed just an overall
      // fraction) — show the percentage so the bar still reads a value.
      label.textContent = Math.round(Math.max(0, Math.min(1, pct)) * 100) + '%';
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-damage-bar')) {
  customElements.define('ph-damage-bar', PhDamageBar);
}
