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
      '</style>',
      '<ph-radar id="inner-radar"></ph-radar>',
      '<div class="corner-label top-left" id="label-pos">X: 0  Z: 0</div>',
      '<div class="corner-label top-right" id="label-bearing">000°</div>',
      '<div class="corner-label bottom-left" id="label-speed">0.0 km/s</div>',
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
  }

  set state(val) {
    if (this.#innerRadar) {
      this.#innerRadar.state = {
        blips: val?.blips || [],
        range: val?.scan_range || 0,
        ship_heading: val?.ship_heading || 0,
        config: val?.config || {},
        selected_target_uuid: val?.science_target_uuid || null,
        target_uuid: val?.science_target_uuid || null,
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
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-sensor-radar')) {
  customElements.define('ph-sensor-radar', PhSensorRadar);
}
