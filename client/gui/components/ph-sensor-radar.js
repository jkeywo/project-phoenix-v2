import './ph-radar.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  SCOPE_CHROME_CSS, scopeChromeMarkup, updateScopeChrome,
} from './ph-scope-chrome.js';
import { PhElement, phDefine } from './ph-element.js';

export class PhSensorRadar extends PhElement {
  template() {
    return [
      '<style>',
      ':host { display: block; width: 100%; height: 100%; position: relative; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      SCOPE_CHROME_CSS,
      '.on-screen-btn {',
      '  position: absolute; bottom: 6%; right: 6%;',
      '  pointer-events: auto; z-index: 10;',
      '  font-family: \'JetBrains Mono\', monospace; font-size: var(--text-xs);',
      '  letter-spacing: 0.15em; color: var(--ink-dim); background: rgba(var(--rgb-deep), 0.85);',
      '  border: 1px solid var(--line-faint); border-radius: 2px; padding: 2px 12px;',
      '  cursor: pointer; text-transform: uppercase;',
      '  transition: border-color 0.15s, color 0.15s, background 0.15s;',
      /* The touch floor (PRD #1023 module 3). inline-flex because min-height
         does nothing to an inline box, and the label has to stay centred in a
         control that is now taller than its own text. */
      '  display: inline-flex; align-items: center; justify-content: center;',
      '  min-height: var(--control-hit-min);',
      '}',
      '.on-screen-btn:hover { border-color: var(--cyan); }',
      '.on-screen-btn.active { border-color: var(--cyan); color: var(--cyan); background: rgba(var(--rgb-cyan), 0.18); }',
      '</style>',
      '<ph-radar id="inner-radar"></ph-radar>',
      scopeChromeMarkup(''),
      '<button class="on-screen-btn" id="on-screen-btn" type="button">' + t('console.common.on_screen') + '</button>',
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

    const btn = this.shadowRoot.getElementById('on-screen-btn');
    if (btn) {
      btn.classList.toggle('active', !!val?.on_screen_active);
    }
  }
}

phDefine('ph-sensor-radar', PhSensorRadar);
