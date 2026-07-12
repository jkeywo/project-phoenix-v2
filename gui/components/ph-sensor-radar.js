import './ph-radar.js';

export class PhSensorRadar extends HTMLElement {
  #innerRadar = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; width: 100%; height: 100%; position: relative; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      '.corner-label {',
      '  position: absolute; pointer-events: none; z-index: 10;',
      '  font-family: \'JetBrains Mono\', monospace; font-size: 0.6rem;',
      '  letter-spacing: 0.1em; color: #5a6a7e;',
      '}',
      '.corner-label.top-left { top: 4%; left: 6%; }',
      '.corner-label.top-right { top: 4%; right: 6%; text-align: right; }',
      '.corner-label.bottom-left { bottom: 6%; left: 6%; }',
      '.on-screen-btn {',
      '  position: absolute; bottom: 6%; right: 6%;',
      '  pointer-events: auto; z-index: 10;',
      '  font-family: \'JetBrains Mono\', monospace; font-size: 0.65rem;',
      '  letter-spacing: 0.15em; color: #8899b0; background: rgba(5,8,22,0.85);',
      '  border: 1px solid var(--line-faint); border-radius: 2px; padding: 2px 12px;',
      '  cursor: pointer; text-transform: uppercase;',
      '  transition: border-color 0.15s, color 0.15s, background 0.15s;',
      '}',
      '.on-screen-btn:hover { border-color: #6cb6d0; }',
      '.on-screen-btn.active { border-color: #6cb6d0; color: #6cb6d0; background: rgba(108,182,208,0.18); }',
      '</style>',
      '<ph-radar id="inner-radar"></ph-radar>',
      '<div class="corner-label top-left" id="label-pos">X: 0  Z: 0</div>',
      '<div class="corner-label top-right" id="label-bearing">000°</div>',
      '<div class="corner-label bottom-left" id="label-speed">0.0 km/s</div>',
      '<button class="on-screen-btn" id="on-screen-btn" type="button">ON SCREEN</button>',
    ].join('\n');
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this.#innerRadar = this.shadowRoot.getElementById('inner-radar');
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    if (this.#innerRadar) {
      this.#innerRadar.sendAction = (_action, payload) => {
        this.sendAction?.('set_sensors_target', { uuid: payload.uuid });
      };
    }
    this.shadowRoot.getElementById('on-screen-btn').addEventListener('click', () => {
      if (this.sendAction) {
        this.sendAction('set_view', { direction: 'SensorsRadar' });
      }
    });
  }

  set state(val) {
    if (this.#innerRadar) {
      this.#innerRadar.state = {
        blips: val?.blips || [],
        range: val?.scan_range || 0,
        ship_heading: val?.ship_heading || 0,
        config: val?.config || {},
        selected_target_uuid: val?.target_uuid || null,
        target_uuid: val?.target_uuid || null,
      };
    }

    const posLabel = this.shadowRoot.getElementById('label-pos');
    if (posLabel) {
      const x = val?.ship_x != null ? val.ship_x : 0;
      const z = val?.ship_z != null ? val.ship_z : 0;
      posLabel.textContent = 'X: ' + x.toFixed(0) + '  Z: ' + z.toFixed(0);
    }

    const bearingLabel = this.shadowRoot.getElementById('label-bearing');
    if (bearingLabel) {
      const h = val?.ship_heading != null ? ((val.ship_heading % 360) + 360) % 360 : 0;
      bearingLabel.textContent = String(h.toFixed(0)).padStart(3, '0') + '\u00B0';
    }

    const speedLabel = this.shadowRoot.getElementById('label-speed');
    if (speedLabel) {
      const spd = val?.ship_speed != null ? val.ship_speed : 0;
      speedLabel.textContent = (spd * 3.6).toFixed(1) + ' km/s';
    }

    const btn = this.shadowRoot.getElementById('on-screen-btn');
    if (btn) {
      btn.classList.toggle('active', !!val?.on_screen_active);
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-sensor-radar')) {
  customElements.define('ph-sensor-radar', PhSensorRadar);
}
