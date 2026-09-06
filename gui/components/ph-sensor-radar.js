import './ph-radar.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the t() call inside the template's shared ON SCREEN
// fragment never sees an empty table. No-op in Node tests (setup-strings.js
// loads the table there).
import '../strings-boot.js';
import {
  SCOPE_CHROME_CSS, SCOPE_ON_SCREEN_CSS, scopeChromeMarkup, scopeOnScreenMarkup,
  updateScopeChrome, setScopeOnScreen,
} from './ph-scope-chrome.js';
import { PhElement, phDefine } from './ph-element.js';
import {
  SENSORS_TARGET_ACTION_ID,
  SENSORS_VIEWSCREEN_ACTION_ID,
} from '../stations/sensors-actions.js';

export class PhSensorRadar extends PhElement {
  template() {
    return [
      '<style>',
      ':host { display: block; width: 100%; height: 100%; position: relative; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      SCOPE_CHROME_CSS,
      SCOPE_ON_SCREEN_CSS,
      '</style>',
      '<ph-radar id="inner-radar"></ph-radar>',
      scopeChromeMarkup(''),
      scopeOnScreenMarkup(''),
    ].join('\n');
  }

  onTemplate() {
    // A PLAIN property, not a #field (see ph-element.js's field-init note):
    // onTemplate runs before this subclass's field-init phase.
    this.innerRadar = this.shadowRoot.getElementById('inner-radar');
  }

  connectedCallback() {
    super.connectedCallback();
    if (this.innerRadar) {
      this.innerRadar.sendAction = (_action, payload) => {
        const activate = typeof window !== 'undefined' && window.activateSemanticAction;
        if (typeof activate === 'function') {
          activate(SENSORS_TARGET_ACTION_ID, {
            source: 'control',
            detail: { uuid: payload.uuid },
          });
        }
      };
    }
    this.shadowRoot.getElementById('on-screen-btn').addEventListener('click', () => {
      const activate = typeof window !== 'undefined' && window.activateSemanticAction;
      if (typeof activate === 'function') {
        activate(SENSORS_VIEWSCREEN_ACTION_ID, { source: 'control' });
      }
    });
  }

  set state(val) {
    if (this.innerRadar) {
      this.innerRadar.state = {
        blips: val?.blips || [],
        range: val?.scan_range || 0,
        ship_heading: val?.ship_heading || 0,
        config: val?.config || {},
        // ph-radar draws TWO independent rings: a cyan one around
        // `selected_target_uuid` (this console's own selection) and a red one
        // around `target_uuid` (the ship's weapons lock). Sensors handed its
        // one scan target to both, so every contact the sensors officer picked
        // came up double-ringed — and the red ring is a weapons lock the
        // sensors console has no business asserting, since weapons may be
        // holding fire or aimed at something else entirely.
        //
        // Sensors owns a selection, not a lock. Cyan only.
        selected_target_uuid: val?.target_uuid || null,
        // Selected-contact trajectory projection (issue #1339). `null`/absent
        // when the Science Target's velocity is unknown — ph-radar draws
        // nothing in that case.
        target_projection: val?.target_projection || null,
      };
    }

    // Sensors names the same three readings differently on the wire \u2014
    // `ship_x` / `ship_z` / `ship_speed` where helm and tactical say `x` / `z`
    // / `speed`. Unpacked here, once, so the divergence stops at this boundary
    // instead of being carried into a third copy of the rendering.
    updateScopeChrome(this.shadowRoot, {
      x: val?.ship_x, z: val?.ship_z,
      headingDeg: val?.ship_heading, speed: val?.ship_speed,
    });

    setScopeOnScreen(this.shadowRoot, val?.on_screen_active);
  }
}

phDefine('ph-sensor-radar', PhSensorRadar);
