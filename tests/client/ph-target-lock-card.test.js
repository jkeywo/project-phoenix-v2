// @vitest-environment jsdom
/**
 * tests/client/ph-target-lock-card.test.js — the Tactical target lock card
 * (issue #1378).
 */
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-target-lock-card.js';

function setup() {
  document.body.innerHTML = '<ph-target-lock-card id="test-card"></ph-target-lock-card>';
  return document.getElementById('test-card');
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

function scanRow(host, key) {
  const rows = Array.from(host.shadowRoot.querySelectorAll('.scan-row'));
  const row = rows.find(r => r.children[0].textContent === key);
  return row ? row.children[1].textContent : null;
}

describe('PhTargetLockCard', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-target-lock-card')).toBeDefined();
  });

  it('shows the no-target state with no lock (AC #1)', () => {
    const el = setup();
    el.state = {};
    expect(el.shadowRoot.getElementById('no-target').style.display).not.toBe('none');
    expect(el.shadowRoot.getElementById('card').style.display).toBe('none');
    expect(queryText(el, '#no-target')).toBe(t('console.common.no_target'));
  });

  it('shows the no-target state when state is entirely absent', () => {
    const el = setup();
    el.state = null;
    expect(el.shadowRoot.getElementById('no-target').style.display).not.toBe('none');
  });

  it('does not throw on a locked target_uuid with every other field absent (smoke-shape guard)', () => {
    // Mirrors tests/smoke/combat-keyboard.spec.js's TACTICAL_STATE: a real
    // Tactical lock with none of the target-fact fields set.
    const el = setup();
    expect(() => { el.state = { target_uuid: 'raider-1' }; }).not.toThrow();
    expect(el.shadowRoot.getElementById('card').style.display).not.toBe('none');
    expect(queryText(el, '#name')).toBe('raider-1');
  });

  it('shows name, stance, class, bearing, range, hull, shield facings and shield frequency with a lock (AC #1)', () => {
    const el = setup();
    el.state = {
      target_uuid: 'raider-1',
      target_name: 'KSV Raider',
      target_stance: 'hostile',
      target_class: "B'rel-class",
      target_bearing: 243.4,
      target_range: 321,
      target_hull_pct: 78,
      target_shields: [
        { label: 'fore', hp: 40, max_hp: 100, online: true },
        { label: 'aft', hp: 0, max_hp: 100, online: false },
      ],
      target_shield_freq: 0.42,
    };
    expect(el.shadowRoot.getElementById('no-target').style.display).toBe('none');
    expect(el.shadowRoot.getElementById('card').style.display).not.toBe('none');
    expect(queryText(el, '#name')).toBe('KSV Raider');
    expect(queryText(el, '.badge')).toBe(t('console.stance.hostile'));
    expect(queryText(el, '#brg-val')).toBe('243.4');
    expect(queryText(el, '#rng-val')).toBe('321');
    expect(scanRow(el, t('component.sensor_panel.class'))).toBe("B'rel-class");
    expect(scanRow(el, t('component.sensor_panel.hull'))).toBe('78%');
    expect(scanRow(el, t('component.sensor_panel.shield')))
      .toBe('FORE 40% · AFT ' + t('console.shield.down'));
    expect(scanRow(el, t('component.sensor_panel.shield_freq'))).toBe('42%');
  });

  it('falls back to the raw uuid when the lock has no name', () => {
    const el = setup();
    el.state = { target_uuid: 'raider-1' };
    expect(queryText(el, '#name')).toBe('raider-1');
  });

  it('omits shield rows when the lock has no shield facings or frequency', () => {
    const el = setup();
    el.state = { target_uuid: 'raider-1', target_name: 'Raider' };
    expect(scanRow(el, t('component.sensor_panel.shield'))).toBeNull();
    expect(scanRow(el, t('component.sensor_panel.shield_freq'))).toBeNull();
  });

  it('clears stale rows when the lock is dropped and shows the no-target state again', () => {
    const el = setup();
    el.state = {
      target_uuid: 'raider-1', target_name: 'Raider', target_class: 'Corvette',
      target_hull_pct: 50,
    };
    expect(scanRow(el, t('component.sensor_panel.class'))).toBe('Corvette');

    el.state = { target_uuid: null };
    expect(el.shadowRoot.getElementById('no-target').style.display).not.toBe('none');
    expect(el.shadowRoot.querySelectorAll('.scan-row').length).toBe(0);
  });

  it('reconciles rows across renders rather than accumulating duplicates', () => {
    const el = setup();
    el.state = { target_uuid: 'raider-1', target_name: 'Raider', target_hull_pct: 50 };
    el.state = { target_uuid: 'raider-1', target_name: 'Raider', target_hull_pct: 30 };
    const hullRows = Array.from(el.shadowRoot.querySelectorAll('.scan-row'))
      .filter(r => r.children[0].textContent === t('component.sensor_panel.hull'));
    expect(hullRows.length).toBe(1);
    expect(hullRows[0].children[1].textContent).toBe('30%');
  });
});
