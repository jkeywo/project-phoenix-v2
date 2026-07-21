export class PhDamageBar extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .bar-wrap { position: relative; width: 100%; height: 1.2em; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .bar-wrap .fill { position: absolute; top: 0; left: 0; height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.5s ease; }
    .bar-wrap .fill.warn { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .bar-wrap .fill.crit { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .bar-wrap .label { position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; align-items: center; justify-content: center; font-size: 0.65rem; letter-spacing: 0.1em; color: var(--ink); text-shadow: 0 0 4px #000; pointer-events: none; }
  </style>
  <div class="bar-wrap">
    <div class="fill" id="bar-fill" style="width:100%"></div>
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
    const label = root.getElementById('bar-label');

    const widthPct = Math.max(0, Math.min(1, pct)) * 100;
    fill.style.width = widthPct + '%';

    let cls = 'fill';
    if (pct < 0.4) cls += ' crit';
    else if (pct < 0.75) cls += ' warn';
    fill.className = cls;

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
