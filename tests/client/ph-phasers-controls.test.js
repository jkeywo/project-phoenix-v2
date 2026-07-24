// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-phasers-controls.js';

// The component consumes the server wire type `PhaserBankState`
// (core/messages.rs): { id, fire_ready, on_cooldown, cooldown_remaining }.
// Auto/Manual is a ship-level `phaser_mode` surfaced through the header
// toggle, not a per-bank field.

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-phasers-controls id="test-el"></ph-phasers-controls>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhPhasersControls', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-phasers-controls')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO PHASER BANKS placeholder with empty banks', () => {
    const { el } = setup();
    el.state = { banks: [] };
    expect(queryText(el, '#banks')).toBe(t('component.phasers.empty'));
  });

  it('renders NO PHASER BANKS placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#banks')).toBe(t('component.phasers.empty'));
  });

  it('renders a phaser bank with label text', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    expect(queryText(el, '.lbl')).toBe('Fore');
  });

  it('marks the cooldown bar as cooling while on cooldown', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true }],
      target_valid: true,
      mode: 'Manual',
    };
    const fill = el.shadowRoot.querySelector('.cooldown-fill');
    expect(fill.classList.contains('cooling')).toBe(true);
  });

  it('fire button is disabled while on cooldown', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: true }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled when target is invalid', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: false,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled when the bank is not fire-ready', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: false, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled in Auto mode', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Auto',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is enabled when ready, target valid, cooldown clear, and Manual', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(false);
  });

  it('mode toggle shows AUTO in Auto mode and MANUAL in Manual mode', () => {
    const { el } = setup();
    el.state = { banks: [], mode: 'Auto' };
    expect(queryText(el, '#mode-toggle')).toBe(t('console.common.auto'));
    el.state = { banks: [], mode: 'Manual' };
    expect(queryText(el, '#mode-toggle')).toBe(t('component.phasers.manual'));
  });

  it('clicking the mode toggle dispatches set_phaser_mode with the flipped mode', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { banks: [], mode: 'Auto' };
    el.shadowRoot.querySelector('#mode-toggle').click();
    expect(sendAction).toHaveBeenCalledWith('set_phaser_mode', { mode: 'Manual' });

    el.state = { banks: [], mode: 'Manual' };
    el.shadowRoot.querySelector('#mode-toggle').click();
    expect(sendAction).toHaveBeenCalledWith('set_phaser_mode', { mode: 'Auto' });
  });

  it('clicking fire button dispatches fire_phaser action', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('fire_phaser', { bank: 'fore' });
  });

  it('reconciles banks by id, reusing existing DOM elements', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false },
        { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true },
      ],
      target_valid: true,
      mode: 'Manual',
    };
    const rows1 = el.shadowRoot.querySelectorAll('.bank-row');
    expect(rows1.length).toBe(2);
    expect(rows1[0].dataset.id).toBe('fore');
    expect(rows1[1].dataset.id).toBe('aft');

    el.state = {
      banks: [
        { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true },
      ],
      target_valid: true,
      mode: 'Manual',
    };
    const rows2 = el.shadowRoot.querySelectorAll('.bank-row');
    expect(rows2.length).toBe(1);
    expect(rows2[0].dataset.id).toBe('aft');
  });

  // ── Shared weapon readiness contract (issue #764) ──────────────────────
  // The same observable blocking cases the server publishes (see
  // src/console/weapons/blackboard.rs + blaster.rs model tests): Ready,
  // NoTarget, OutOfRange, OutOfArc, Offline.

  function readyBank(reason, extra) {
    return {
      id: 'fore',
      label: 'Fore',
      fire_ready: reason === 'Ready',
      on_cooldown: reason === 'Cooldown',
      readiness: {
        ready: reason === 'Ready',
        blocking_reason: reason,
        target_range: 20,
        target_arc: 5,
        ...extra,
      },
    };
  }

  it('renders READY state: enables fire and marks the row ready', () => {
    const { el } = setup();
    el.state = { banks: [readyBank('Ready')], target_valid: true, mode: 'Manual' };
    const row = el.shadowRoot.querySelector('.bank-row');
    expect(row.classList.contains('ready')).toBe(true);
    expect(el.shadowRoot.querySelector('.btn').disabled).toBe(false);
  });

  it('renders OUT OF RANGE block: shows the shared label and disables fire', () => {
    const { el } = setup();
    el.state = { banks: [readyBank('OutOfRange')], target_valid: true, mode: 'Manual' };
    const row = el.shadowRoot.querySelector('.bank-row');
    expect(row.classList.contains('blocked')).toBe(true);
    expect(queryText(el, '.status')).toBe(t('console.common.out_of_range'));
    expect(el.shadowRoot.querySelector('.btn').disabled).toBe(true);
  });

  it('renders OUT OF ARC block with the shared label', () => {
    const { el } = setup();
    el.state = { banks: [readyBank('OutOfArc')], target_valid: true, mode: 'Manual' };
    expect(queryText(el, '.status')).toBe(t('console.common.out_of_arc'));
    expect(el.shadowRoot.querySelector('.bank-row').classList.contains('blocked')).toBe(true);
  });

  it('renders NO TARGET block with the shared label', () => {
    const { el } = setup();
    el.state = { banks: [readyBank('NoTarget', { target_range: null, target_arc: null })], target_valid: false, mode: 'Manual' };
    expect(queryText(el, '.status')).toBe(t('console.common.no_target'));
  });

  it('renders OFFLINE as an unavailable state and disables fire', () => {
    const { el } = setup();
    el.state = { banks: [readyBank('Offline')], target_valid: true, mode: 'Manual' };
    const row = el.shadowRoot.querySelector('.bank-row');
    expect(row.classList.contains('unavailable')).toBe(true);
    expect(queryText(el, '.status')).toBe(t('console.common.offline'));
    expect(el.shadowRoot.querySelector('.btn').disabled).toBe(true);
  });

  it('removes surplus rows when banks are removed', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false },
        { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true },
      ],
      target_valid: true,
      mode: 'Manual',
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(2);

    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(1);
  });
});
