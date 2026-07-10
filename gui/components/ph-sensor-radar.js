import './ph-radar.js';

export class PhSensorRadar extends HTMLElement {
  #state = null;
  #innerRadar = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; position: relative; }',
      '.container { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; }',
      'ph-radar { display: block; width: 100%; flex: 1; }',
      '.btn-wrap { display: flex; justify-content: center; padding: 0.35rem 0; }',
      '.designate-btn {',
      '  font-family: \'JetBrains Mono\', monospace; font-size: 0.65rem;',
      '  letter-spacing: 0.15em; text-transform: uppercase;',
      '  color: #8899b0; background: rgba(5,8,22,0.85);',
      '  border: 1px solid #282c38; border-radius: 2px;',
      '  padding: 4px 16px; cursor: pointer;',
      '  transition: border-color 0.15s, color 0.15s, background 0.15s;',
      '}',
      '.designate-btn:not(:disabled):hover { border-color: #5fd8e8; color: #cce; }',
      '.designate-btn:disabled { opacity: 0.35; cursor: default; }',
      '</style>',
      '<div class="container">',
      '  <ph-radar id="inner-radar"></ph-radar>',
      '  <div class="btn-wrap">',
      '    <button class="designate-btn" id="designate-btn" type="button" disabled>DESIGNATE</button>',
      '  </div>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this.#innerRadar = this.shadowRoot.getElementById('inner-radar');
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    if (this.#innerRadar) {
      this.#innerRadar.sendAction = (action, payload) => {
        if (action === 'set_target') {
          this.#state = { ...this.#state, selected_target_uuid: payload.uuid };
          this.#render();
        }
        this.sendAction?.(action, payload);
      };
    }
    const btn = this.shadowRoot.getElementById('designate-btn');
    btn.addEventListener('click', () => {
      const sel = this.#state?.selected_target_uuid;
      if (sel && this.sendAction) {
        this.sendAction('set_sensors_target', { uuid: sel });
      }
    });
  }

  set state(val) {
    const prevSelected = this.#state?.selected_target_uuid;
    let selected_target_uuid = null;
    if (prevSelected != null && prevSelected !== val?.target_uuid) {
      const stillPresent = (val?.blips || []).some((b) => b.uuid === prevSelected);
      if (stillPresent) selected_target_uuid = prevSelected;
    }
    this.#state = { ...val, selected_target_uuid };
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    if (this.#innerRadar) {
      this.#innerRadar.state = {
        blips: s.blips || [],
        range: s.scan_range || 0,
        ship_heading: s.ship_heading || 0,
        config: s.config || {},
        selected_target_uuid: s.selected_target_uuid || null,
        target_uuid: s.target_uuid || null,
      };
    }
    this.#updateButton();
  }

  #updateButton() {
    const s = this.#state || {};
    const btn = this.shadowRoot.getElementById('designate-btn');
    if (!btn) return;
    const sel = s.selected_target_uuid;
    const confirmed = s.target_uuid;
    btn.disabled = sel == null || sel === confirmed;
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-sensor-radar')) {
  customElements.define('ph-sensor-radar', PhSensorRadar);
}
