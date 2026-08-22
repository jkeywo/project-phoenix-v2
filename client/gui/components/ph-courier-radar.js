import { PhTacticalRadar } from './ph-tactical-radar.js';
import { phDefine } from './ph-element.js';

/**
 * Courier pilot radar — one scope, one selection, two consumers.
 *
 * The Courier has a single station, so there is no separate sensors scope to
 * select a target on. One tap therefore has to drive both the blaster target
 * (`set_target`) and the sensor readout (`set_sensors_target`), which is why
 * this fans the inner ph-radar's single tap out to both actions.
 *
 * Mirrors ph-sensor-radar, which intercepts the same event and remaps it to
 * `set_sensors_target` alone. Extends ph-tactical-radar to inherit its fire-arc
 * and selected-target overlays; only the tap wiring differs.
 */
export class PhCourierRadar extends PhTacticalRadar {
  connectedCallback() {
    this.sendAction ??= window.sendAction;
    // Deliberately NOT super.connectedCallback(): the tactical base installs a
    // single-action passthrough on the inner radar, which is exactly the wiring
    // this subclass replaces — one tap has to fan out to BOTH the blaster target
    // and the sensor readout. Reach the inner radar through the shadow root
    // (`this.innerRadar` is only set once the base's own connectedCallback runs).
    const inner = this.shadowRoot.getElementById('inner-radar');
    if (inner) {
      inner.sendAction = (_action, payload) => {
        this.sendAction?.('set_target', { uuid: payload.uuid });
        this.sendAction?.('set_sensors_target', { uuid: payload.uuid });
      };
    }
  }
}

phDefine('ph-courier-radar', PhCourierRadar);
