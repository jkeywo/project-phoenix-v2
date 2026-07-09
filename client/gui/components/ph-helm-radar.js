import './ph-radar.js';

export class PhHelmRadar extends HTMLElement {
  #state = null;
  #innerRadar = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; position: relative; }',
      '.container { position: relative; width: 100%; height: 100%; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      '.svg-overlay { position: absolute; inset: 0; pointer-events: none; overflow: visible; }',
      '.thrust-arc { fill: none; stroke: #6cb6d0; stroke-width: 4; stroke-linecap: round; }',
      '.on-screen-btn {',
      '  position: absolute; bottom: 6%; left: 50%; transform: translateX(-50%);',
      '  pointer-events: auto; z-index: 10;',
      '  font-family: \'JetBrains Mono\', monospace; font-size: 0.65rem;',
      '  letter-spacing: 0.15em; color: #8899b0; background: rgba(5,8,22,0.85);',
      '  border: 1px solid #282c38; border-radius: 2px; padding: 2px 12px;',
      '  cursor: pointer; text-transform: uppercase;',
      '  transition: border-color 0.15s, color 0.15s, background 0.15s;',
      '}',
      '.on-screen-btn:hover { border-color: #6cb6d0; }',
      '.on-screen-btn.active { border-color: #6cb6d0; color: #6cb6d0; background: rgba(108,182,208,0.18); }',
      '</style>',
      '<div class="container">',
      '  <ph-radar id="inner-radar"></ph-radar>',
      '  <svg class="svg-overlay" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid meet">',
      '    <path class="thrust-arc" id="arc-port" />',
      '    <path class="thrust-arc" id="arc-stbd" />',
      '  </svg>',
      '  <button class="on-screen-btn" id="on-screen-btn" type="button">ON SCREEN</button>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this.#innerRadar = this.shadowRoot.getElementById('inner-radar');
  }

  connectedCallback() {
    if (!this.sendAction) {
      if (typeof window !== 'undefined' && typeof window.sendAction === 'function') {
        this.sendAction = window.sendAction;
      }
    }
    this.shadowRoot.getElementById('on-screen-btn').addEventListener('click', () => {
      if (this.sendAction) {
        this.sendAction('set_radar_view', {});
      }
    });
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};

    if (this.#innerRadar) {
      this.#innerRadar.state = {
        blips: s.blips || [],
        range: s.range,
        ship_heading: s.ship_heading,
        config: s.config || {},
      };
    }

    this.#updateThrustArcs(s);

    const btn = this.shadowRoot.getElementById('on-screen-btn');
    if (btn) {
      btn.classList.toggle('active', !!s.on_screen_active);
    }
  }

  #updateThrustArcs(state) {
    const port = Math.max(0, Math.min(1, state.engine_port_thrust || 0));
    const stbd = Math.max(0, Math.min(1, state.engine_stbd_thrust || 0));

    const cx = 50, cy = 50, r = 42;
    const maxSweep = 50;

    const arcPort = this.shadowRoot.getElementById('arc-port');
    const arcStbd = this.shadowRoot.getElementById('arc-stbd');

    if (arcPort) {
      const path = port > 0 ? this.#arcPath(cx, cy, r, 90 + port * maxSweep, 90) : '';
      arcPort.setAttribute('d', path);
      arcPort.style.opacity = String(0.2 + 0.8 * port);
    }
    if (arcStbd) {
      const path = stbd > 0 ? this.#arcPath(cx, cy, r, 90, 90 - stbd * maxSweep) : '';
      arcStbd.setAttribute('d', path);
      arcStbd.style.opacity = String(0.2 + 0.8 * stbd);
    }
  }

  #arcPath(cx, cy, r, startAngle, endAngle) {
    const startRad = startAngle * Math.PI / 180;
    const endRad = endAngle * Math.PI / 180;
    const x1 = cx + r * Math.cos(startRad);
    const y1 = cy + r * Math.sin(startRad);
    const x2 = cx + r * Math.cos(endRad);
    const y2 = cy + r * Math.sin(endRad);
    const sweep = endAngle - startAngle;
    const largeArc = Math.abs(sweep) > 180 ? 1 : 0;
    const sweepFlag = sweep >= 0 ? 1 : 0;
    return [
      'M', x1.toFixed(1), y1.toFixed(1),
      'A', r, r, 0, largeArc, sweepFlag, x2.toFixed(1), y2.toFixed(1),
    ].join(' ');
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-helm-radar')) {
  customElements.define('ph-helm-radar', PhHelmRadar);
}
