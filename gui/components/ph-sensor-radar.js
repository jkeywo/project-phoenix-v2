import './ph-radar.js';

export class PhSensorRadar extends HTMLElement {
  #innerRadar = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; width: 100%; height: 100%; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      '</style>',
      '<ph-radar id="inner-radar"></ph-radar>',
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
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-sensor-radar')) {
  customElements.define('ph-sensor-radar', PhSensorRadar);
}
