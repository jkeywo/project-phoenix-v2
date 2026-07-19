// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

export class PhBatteryBar extends HTMLElement {
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
    .bar-wrap .fill.amber { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .bar-wrap .fill.red { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .bar-wrap .threshold-marker { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--ink); opacity: 0.7; pointer-events: none; }
    .bar-wrap .label { position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; align-items: center; justify-content: center; font-size: 0.65rem; letter-spacing: 0.1em; color: var(--ink); text-shadow: 0 0 4px #000; pointer-events: none; }
    .charging-indicator { position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: none; align-items: center; justify-content: center; font-size: 0.55rem; letter-spacing: 0.15em; color: var(--loaded); pointer-events: none; animation: pulse-glow 1.5s ease-in-out infinite; }
    @keyframes pulse-glow {
      0%, 100% { opacity: 0.5; text-shadow: 0 0 2px var(--loaded); }
      50% { opacity: 1; text-shadow: 0 0 8px var(--loaded), 0 0 16px var(--loaded-dim); }
    }
  </style>
  <div class="bar-wrap">
    <div class="fill" id="bar-fill" style="width:100%"></div>
    <div class="threshold-marker" id="threshold-marker" style="left:20%"></div>
    <span class="label" id="bar-label">—%</span>
    <span class="charging-indicator" id="charging-indicator">${t('component.battery_bar.charging')}</span>
  </div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {}

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const d = this.#state || {};

    let levelPct;
    if (d.level_pct != null) {
      levelPct = Number(d.level_pct);
    } else if (d.charge != null && d.capacity != null && d.capacity > 0) {
      levelPct = (Number(d.charge) / Number(d.capacity)) * 100;
    } else {
      levelPct = 100;
    }
    levelPct = Math.max(0, Math.min(100, levelPct));

    const emergencyThreshold = d.emergency_threshold_pct != null ? Number(d.emergency_threshold_pct) : 20;
    const charging = !!d.charging;

    const root = this.shadowRoot;
    const fill = root.getElementById('bar-fill');
    const label = root.getElementById('bar-label');
    const thresholdMarker = root.getElementById('threshold-marker');
    const chargingIndicator = root.getElementById('charging-indicator');

    fill.style.width = levelPct + '%';

    let cls = 'fill';
    if (levelPct <= emergencyThreshold) {
      cls += ' red';
    } else if (levelPct <= emergencyThreshold + 10) {
      cls += ' amber';
    }
    fill.className = cls;

    thresholdMarker.style.left = emergencyThreshold + '%';

    label.textContent = Math.round(levelPct) + '%';

    chargingIndicator.style.display = charging ? 'flex' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-battery-bar')) {
  customElements.define('ph-battery-bar', PhBatteryBar);
}
