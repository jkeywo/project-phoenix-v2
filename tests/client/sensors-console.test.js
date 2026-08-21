// @vitest-environment jsdom
/**
 * tests/client/sensors-console.test.js — the battleship's Sensors console on
 * the shared renderer (issue #1235, T4.C3 chunk 2).
 *
 * `gui/battleship/sensors.html` imports its `renderStation` from
 * `gui/battleship/sensors.console.js`; this suite imports the SAME function
 * and drives it against a jsdom fixture. The cruiser folds Sensors into
 * Science instead (see science-console.test.js) and the destroyer has
 * neither, so this is the only surviving dedicated-Sensors variant.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation } from '../../gui/battleship/sensors.console.js';

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURE =
  '<ph-sensor-radar id="sensor-radar"></ph-sensor-radar>' +
  '<ph-sensor-panel id="sensor-panel"></ph-sensor-panel>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="scan-range-val">0</span>' +
  '<span id="contact-sub">0</span>' +
  '<span id="tgt-name"></span>' +
  '<span id="tgt-kind-tag"></span>' +
  '<span id="footer-target"></span>' +
  '<button id="btn-cancel-impulse" hidden></button>' +
  '<span id="sensors-auto-badge" hidden></span>' +
  '<div id="shield-facings"></div>' +
  '<span id="shields-tag"></span>' +
  '<span id="shield-freq-tag"></span>';

describe('battleship sensors renderStation', () => {
  beforeEach(() => mount(FIXTURE));

  const payload = {
    blips: [{ id: 'b1' }], scan_range: 6000, target_name: 'Raider', target_kind: 'fighter',
    impulse_charge_progress: 0, sensors_auto: false, own_hull: { pct: 1 },
  };

  it('pushes the whole payload straight through to the radar and shared sensor panel', () => {
    renderStation(payload, document);
    expect(el('sensor-radar').state).toBe(payload);
    expect(el('sensor-panel').state).toBe(payload);
  });

  it('drives the scan summary and target analysis readouts', () => {
    renderStation(payload, document);
    expect(el('station-damage').state).toEqual({ pct: 1 });
    expect(el('scan-range-val').textContent).toBe('6000');
    expect(el('contact-sub').textContent).toBe(t('console.common.contacts.one', { n: 1 }));
    expect(el('tgt-name').textContent).toBe('Raider');
    expect(el('tgt-kind-tag').textContent).toBe('FIGHTER');
    expect(el('footer-target').textContent).toBe('Raider');
  });

  it('falls back to localized placeholders with no target and no contacts', () => {
    renderStation({ ...payload, blips: [], target_name: null, target_kind: null }, document);
    expect(el('contact-sub').textContent).toBe(t('console.common.contacts.other', { n: 0 }));
    expect(el('tgt-name').textContent).toBe(t('console.common.no_target'));
    expect(el('tgt-kind-tag').textContent).toBe(t('console.common.no_contact'));
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
  });

  it('shows Cancel Impulse only while charging and disables it (+ shows AUTO) when sensors is AI-run', () => {
    renderStation(payload, document);
    expect(el('btn-cancel-impulse').hidden).toBe(true);
    renderStation({ ...payload, impulse_charge_progress: 0.3 }, document);
    expect(el('btn-cancel-impulse').hidden).toBe(false);
    expect(el('btn-cancel-impulse').disabled).toBe(false);
    expect(el('sensors-auto-badge').hidden).toBe(true);
    renderStation({ ...payload, impulse_charge_progress: 0.3, sensors_auto: true }, document);
    expect(el('btn-cancel-impulse').disabled).toBe(true);
    expect(el('sensors-auto-badge').hidden).toBe(false);
  });

  it('shows no-shield-data when the target carries neither facings nor a fraction', () => {
    renderStation({ ...payload, target_shields: [], target_shield_fraction: null, target_shield_freq: null }, document);
    expect(el('shield-facings').textContent).toBe(t('console.common.no_shield_data'));
    expect(el('shields-tag').textContent).toBe(t('console.common.no_data'));
    expect(el('shield-freq-tag').textContent).toBe(t('console.common.no_data'));
  });

  it('shows an aggregate percentage when the target carries only a shield fraction', () => {
    renderStation({ ...payload, target_shields: [], target_shield_fraction: 0.42 }, document);
    expect(el('shield-facings').innerHTML).toContain('42%');
    expect(el('shields-tag').textContent).toBe(t('console.shield.online'));
    renderStation({ ...payload, target_shields: [], target_shield_fraction: 0 }, document);
    expect(el('shields-tag').textContent).toBe(t('console.shield.shield_down'));
  });

  it('shows per-facing bars and a degraded tag when any facing is offline', () => {
    renderStation({
      ...payload,
      target_shields: [{ label: 'fwd', hp: 50, max_hp: 100, online: true }, { label: 'aft', hp: 0, max_hp: 100, online: false }],
      target_shield_freq: 0.5,
    }, document);
    expect(el('shield-facings').innerHTML).toContain('FWD');
    expect(el('shield-facings').innerHTML).toContain('50%');
    expect(el('shields-tag').textContent).toBe(t('console.shield.degraded'));
    expect(el('shield-freq-tag').textContent).toBe('50%');
  });
});
