// @vitest-environment jsdom
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
    expect(queryText(el, '#banks')).toBe('NO BLASTER BANKS');
  });

  it('renders NO BLASTER BANKS placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#banks')).toBe('NO BLASTER BANKS');
  });

  it('renders idle bank with label and FIRE button (instant-fire, no charge time)', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0, has_charge: false }],
    };
    expect(queryText(el, '.lbl')).toBe('Port');
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('FIRE');
    expect(btn.disabled).toBe(false);
  });

  it('renders idle bank with CHARGE button when the bank has a charge time', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'heavy', label: 'Heavy', on_cooldown: false, charge_progress: 0, has_charge: true }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('CHARGE');
  });

  it('shows FIRING... while a charge is in progress', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: false, charge_progress: 0.6 }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('FIRING...');
    expect(btn.classList.contains('charging')).toBe(true);
  });

  it('shows COOLDOWN while on cooldown and disables button', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', on_cooldown: true, charge_progress: 0 }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('COOLDOWN');
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
    expect(queryText(el, '.bar-label')).toBe('CHARGE');
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
    expect(queryText(el, '.bar-label')).toBe('COOLDOWN');
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
    const btn = el.shadowRoot.querySelector('.charge-btn');
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
    const btn = el.shadowRoot.querySelector('.charge-btn');
    btn.dispatchEvent(new MouseEvent('mousedown'));
    expect(sendAction).not.toHaveBeenCalled();
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
