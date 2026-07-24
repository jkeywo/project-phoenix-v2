// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-blasters-controls.js';

// The component consumes the server wire type `BlasterBankState`
// (core/messages.rs): { id, fire_ready, on_cooldown, cooldown_remaining,
// pending_volley, charge_progress, has_charge }. Display state (charging /
// cooling / idle) is derived from on_cooldown + charge_progress.

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-blasters-controls id="test-el"></ph-blasters-controls>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhBlastersControls', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-blasters-controls')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO BLASTER BANKS placeholder with empty banks', () => {
    const { el } = setup();
    el.state = { banks: [] };
    expect(queryText(el, '#banks')).toBe(t('component.blasters.empty'));
  });

  it('renders NO BLASTER BANKS placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#banks')).toBe(t('component.blasters.empty'));
  });

  it('renders idle bank with label and FIRE button (instant-fire, no charge time)', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0, has_charge: false }],
    };
    expect(queryText(el, '.lbl')).toBe('Port');
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.textContent.trim()).toBe(t('console.common.fire'));
    expect(btn.disabled).toBe(false);
  });

  it('renders idle bank with CHARGE button when the bank has a charge time', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'heavy', label: 'Heavy', on_cooldown: false, charge_progress: 0, has_charge: true }],
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.textContent.trim()).toBe(t('component.blasters.charge'));
  });

  it('shows FIRING... while a charge is in progress', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0.6 }],
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.textContent.trim()).toBe(t('component.blasters.firing'));
    expect(btn.classList.contains('tactical')).toBe(true);
  });

  it('shows COOLDOWN while on cooldown and disables button', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: true, charge_progress: 0 }],
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.textContent.trim()).toBe(t('console.common.cooldown'));
    expect(btn.disabled).toBe(true);
  });

  it('shows charge bar at correct width while charging', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0.7 }],
    };
    const fills = el.shadowRoot.querySelectorAll('.bar-fill');
    expect(fills[0].style.width).toBe('70%');
    expect(fills[0].classList.contains('charge')).toBe(true);
    expect(fills[0].style.display).not.toBe('none');
    expect(fills[1].style.display).toBe('none');
    expect(queryText(el, '.bar-label')).toBe(t('component.blasters.charge'));
  });

  it('shows the cooldown bar while cooling', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: true, charge_progress: 0 }],
    };
    const fills = el.shadowRoot.querySelectorAll('.bar-fill');
    expect(fills[1].classList.contains('cooldown')).toBe(true);
    expect(fills[0].style.display).toBe('none');
    expect(fills[1].style.display).not.toBe('none');
    expect(queryText(el, '.bar-label')).toBe(t('console.common.cooldown'));
  });

  it('hides both bars when idle', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0 }],
    };
    const fills = el.shadowRoot.querySelectorAll('.bar-fill');
    expect(fills[0].style.display).toBe('none');
    expect(fills[1].style.display).toBe('none');
    expect(queryText(el, '.bar-label')).toBe('');
  });

  it('dispatches charge_blaster_start on mousedown and fire_blaster on mouseup', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0 }],
    };
    const btn = el.shadowRoot.querySelector('.btn');
    btn.dispatchEvent(new MouseEvent('mousedown'));
    expect(sendAction).toHaveBeenCalledWith('charge_blaster_start', { bank: 'port' });
    btn.dispatchEvent(new MouseEvent('mouseup'));
    expect(sendAction).toHaveBeenCalledWith('fire_blaster', { bank: 'port' });
  });

  it('does not dispatch when button is disabled', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: true, charge_progress: 0 }],
    };
    const btn = el.shadowRoot.querySelector('.btn');
    btn.dispatchEvent(new MouseEvent('mousedown'));
    expect(sendAction).not.toHaveBeenCalled();
  });

  // ── Shared weapon readiness contract (issue #764) ──────────────────────
  // Mirrors the same observable blocking cases the pure blaster model reports
  // (src/weapons/blaster.rs bank_state tests): Ready / NoTarget / OutOfRange /
  // OutOfArc / Cooldown / Loading / Offline.

  function blasterBank(reason, extra) {
    return {
      id: 'fore',
      label: 'Fore',
      on_cooldown: reason === 'Cooldown',
      charge_progress: 0,
      has_charge: false,
      readiness: {
        ready: reason === 'Ready',
        blocking_reason: reason,
        target_range: 12,
        target_arc: 4,
        ...extra,
      },
    };
  }

  it('renders READY state and enables the fire button', () => {
    const { el } = setup();
    el.state = { banks: [blasterBank('Ready')] };
    const row = el.shadowRoot.querySelector('.bank-row');
    expect(row.classList.contains('ready')).toBe(true);
    expect(el.shadowRoot.querySelector('.btn').disabled).toBe(false);
  });

  it('renders OUT OF RANGE block with the shared label and disables fire', () => {
    const { el } = setup();
    el.state = { banks: [blasterBank('OutOfRange')] };
    expect(el.shadowRoot.querySelector('.bank-row').classList.contains('blocked')).toBe(true);
    expect(queryText(el, '.status')).toBe(t('console.common.out_of_range'));
    expect(el.shadowRoot.querySelector('.btn').disabled).toBe(true);
  });

  it('renders OUT OF ARC block with the shared label', () => {
    const { el } = setup();
    el.state = { banks: [blasterBank('OutOfArc')] };
    expect(queryText(el, '.status')).toBe(t('console.common.out_of_arc'));
  });

  it('renders NO TARGET block with the shared label', () => {
    const { el } = setup();
    el.state = { banks: [blasterBank('NoTarget', { target_range: null, target_arc: null })] };
    expect(queryText(el, '.status')).toBe(t('console.common.no_target'));
  });

  it('renders OFFLINE as an unavailable state and disables fire', () => {
    const { el } = setup();
    el.state = { banks: [blasterBank('Offline')] };
    const row = el.shadowRoot.querySelector('.bank-row');
    expect(row.classList.contains('unavailable')).toBe(true);
    expect(queryText(el, '.status')).toBe(t('console.common.offline'));
    expect(el.shadowRoot.querySelector('.btn').disabled).toBe(true);
  });

  it('reconciles banks by id', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0 },
        { id: 'starboard', label: 'Starboard', on_cooldown: false, charge_progress: 0 },
      ],
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(2);

    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0 }],
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(1);
  });
});
