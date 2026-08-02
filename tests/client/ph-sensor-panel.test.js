// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-sensor-panel.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-sensor-panel id="test-panel"></ph-sensor-panel>';
  const el = document.getElementById('test-panel');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhSensorPanel', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-sensor-panel')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders default state with no target', () => {
    const { el } = setup();
    el.state = {};
    expect(queryText(el, '#range-val')).toBe('0');
    expect(queryText(el, '#blip-count')).toBe(t('console.common.contacts.other', { n: 0 }));
    const targetArea = el.shadowRoot.querySelector('#target-area');
    expect(targetArea.textContent).toContain(t('console.common.no_target'));
  });

  it('renders target info when state has target_uuid', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300,
      blips: [],
      target_uuid: 'abc-123',
      target_name: 'KSV NEMESIS',
      target_kind: 'ship',
      target_stance: 'hostile',
      target_bearing: 243.4,
      target_range: 321,
      target_class: "B'REL-CLASS",
      target_hull_pct: 78,
      target_heading: 124,
      target_speed: 18.2,
      target_threat: 'high',
    };
    expect(queryText(el, '#range-val')).toBe('300');
    expect(el.shadowRoot.textContent).toContain('KSV NEMESIS');
    expect(el.shadowRoot.textContent).toContain('HOSTILE');
  });

  it('shows a red-alert scan row when target_alert is true', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_class: "B'REL", target_alert: true,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('component.sensor_panel.alert'));
    expect(sd.textContent).toContain(t('component.sensor_panel.alert_active'));
  });

  it('shows a standby scan row when target_alert is false', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_class: "B'REL", target_alert: false,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('component.sensor_panel.alert'));
    expect(sd.textContent).toContain(t('component.sensor_panel.alert_standby'));
  });

  it('hides the alert scan row when target_alert is null', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_class: "B'REL", target_alert: null,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).not.toContain(t('component.sensor_panel.alert'));
  });

  // ── Target shields (issue #927) ───────────────────────────────────────
  //
  // `target_shields` / `target_shield_fraction` / `target_shield_freq` are on
  // every Sensors payload already; this shared panel — the one the
  // destroyer, cruiser and courier all embed — used to drop all three.

  it('shows a per-facing shield row when target_shields is non-empty', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shields: [
        { label: 'Fore', hp: 80, max_hp: 80, online: true },
        { label: 'Aft', hp: 0, max_hp: 80, online: false },
      ],
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('component.sensor_panel.shield'));
    expect(sd.textContent).toContain('FORE 100%');
    expect(sd.textContent).toContain('AFT ' + t('console.shield.down'));
  });

  it('shows the single-fraction shield row when target_shields is empty but target_shield_fraction is present', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shields: [], target_shield_fraction: 0.5,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('component.sensor_panel.shield'));
    expect(sd.textContent).toContain('50%');
  });

  it('shows shield DOWN when target_shield_fraction is zero', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shields: [], target_shield_fraction: 0,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('console.shield.down'));
  });

  it('hides the shield row entirely when target_shields is empty and target_shield_fraction is null', () => {
    // Matches the no-shield-data case at gui/battleship/sensors.html:78 — a
    // shieldless target (e.g. an asteroid) gets no shield row at all.
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'asteroid',
      target_shields: [], target_shield_fraction: null,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).not.toContain(t('component.sensor_panel.shield'));
  });

  it('shows a shield-frequency row when target_shield_freq is present', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shield_freq: 0.75,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('component.sensor_panel.shield_freq'));
    expect(sd.textContent).toContain('75%');
  });

  it('hides the shield-frequency row when target_shield_freq is null', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shield_freq: null,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).not.toContain(t('component.sensor_panel.shield_freq'));
  });

  // ── hide-shield-rows (issue #927 battleship duplication fix) ────────────
  //
  // The battleship's dedicated Target Analysis section already shows target
  // shields (its own renderShieldFacings() + Shield Freq metric); without
  // this attribute the shared panel duplicated both rows on that console.

  it('suppresses the per-facing shield row when hide-shield-rows is set', () => {
    const { el } = setup('<ph-sensor-panel id="test-panel" hide-shield-rows></ph-sensor-panel>');
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shields: [{ label: 'Fore', hp: 80, max_hp: 80, online: true }],
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).not.toContain(t('component.sensor_panel.shield'));
  });

  it('suppresses the single-fraction shield row when hide-shield-rows is set', () => {
    const { el } = setup('<ph-sensor-panel id="test-panel" hide-shield-rows></ph-sensor-panel>');
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shields: [], target_shield_fraction: 0.5,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).not.toContain(t('component.sensor_panel.shield'));
  });

  it('suppresses the shield-frequency row when hide-shield-rows is set', () => {
    const { el } = setup('<ph-sensor-panel id="test-panel" hide-shield-rows></ph-sensor-panel>');
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shield_freq: 0.75,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).not.toContain(t('component.sensor_panel.shield_freq'));
  });

  it('still shows shield rows when hide-shield-rows is absent (every other hull)', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300, blips: [], target_uuid: 'abc', target_kind: 'ship',
      target_shields: [{ label: 'Fore', hp: 80, max_hp: 80, online: true }],
      target_shield_freq: 0.5,
    };
    const sd = el.shadowRoot.querySelector('#scan-data');
    expect(sd.textContent).toContain(t('component.sensor_panel.shield'));
    expect(sd.textContent).toContain(t('component.sensor_panel.shield_freq'));
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = { scan_range: 100 };
    expect(queryText(el, '#range-val')).toBe('100');
    el.state = { scan_range: 500 };
    expect(queryText(el, '#range-val')).toBe('500');
  });

  it('responds to container sizing via CSS classes', () => {
    const { el } = setup();
    el.style.width = '300px';
    el.style.height = '200px';
    el.state = { scan_range: 400, blips: [] };
    const hostStyle = window.getComputedStyle(el);
    expect(hostStyle.display).toBe('inline');
  });
});
