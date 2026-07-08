// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-blasters-controls.js';

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

  it('renders idle bank with label and CHARGE button', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: false }],
    };
    expect(queryText(el, '.lbl')).toBe('Port');
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('CHARGE');
    expect(btn.disabled).toBe(false);
  });

  it('shows FIRING... when state is charging', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'charging', charge_pct: 0.6, cooldown_pct: 0, auto: false }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('FIRING...');
    expect(btn.classList.contains('charging')).toBe(true);
  });

  it('shows COOLDOWN when state is cooling and disables button', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'cooling', charge_pct: 0, cooldown_pct: 0.4, auto: false }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.textContent.trim()).toBe('COOLDOWN');
    expect(btn.disabled).toBe(true);
  });

  it('shows charge bar at correct width when charging', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'charging', charge_pct: 0.7, cooldown_pct: 0, auto: false }],
    };
    const fills = el.shadowRoot.querySelectorAll('.bar-fill');
    expect(fills[0].style.width).toBe('70%');
    expect(fills[0].classList.contains('charge')).toBe(true);
    expect(fills[0].style.display).not.toBe('none');
    expect(fills[1].style.display).toBe('none');
    expect(queryText(el, '.bar-label')).toBe('CHARGE');
  });

  it('shows cooldown bar at correct width when cooling', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'cooling', charge_pct: 0, cooldown_pct: 0.3, auto: false }],
    };
    const fills = el.shadowRoot.querySelectorAll('.bar-fill');
    expect(fills[1].style.width).toBe('30%');
    expect(fills[1].classList.contains('cooldown')).toBe(true);
    expect(fills[0].style.display).toBe('none');
    expect(fills[1].style.display).not.toBe('none');
    expect(queryText(el, '.bar-label')).toBe('COOLDOWN');
  });

  it('hides both bars when idle', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: false }],
    };
    const fills = el.shadowRoot.querySelectorAll('.bar-fill');
    expect(fills[0].style.display).toBe('none');
    expect(fills[1].style.display).toBe('none');
    expect(queryText(el, '.bar-label')).toBe('');
  });

  it('shows AUTO badge and disables button when auto is true', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: true }],
    };
    const badge = el.shadowRoot.querySelector('.auto-badge');
    expect(badge.style.display).not.toBe('none');
    const btn = el.shadowRoot.querySelector('.charge-btn');
    expect(btn.disabled).toBe(true);
  });

  it('dispatches charge_blaster_start on mousedown and fire_blaster on mouseup', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: false }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    btn.dispatchEvent(new MouseEvent('mousedown'));
    expect(sendAction).toHaveBeenCalledWith('charge_blaster_start', { bank_id: 'port' });
    btn.dispatchEvent(new MouseEvent('mouseup'));
    expect(sendAction).toHaveBeenCalledWith('fire_blaster', { bank_id: 'port' });
  });

  it('does not dispatch when button is disabled', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'cooling', charge_pct: 0, cooldown_pct: 0.5, auto: false }],
    };
    const btn = el.shadowRoot.querySelector('.charge-btn');
    btn.dispatchEvent(new MouseEvent('mousedown'));
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('reconciles banks by id', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'port', label: 'Port', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: false },
        { id: 'starboard', label: 'Starboard', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: false },
      ],
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(2);

    el.state = {
      banks: [{ id: 'port', label: 'Port', state: 'idle', charge_pct: 0, cooldown_pct: 0, auto: false }],
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(1);
  });
});
