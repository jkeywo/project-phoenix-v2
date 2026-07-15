// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-boost-btn.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-boost-btn id="test-el"></ph-boost-btn>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhBoostBtn', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-boost-btn')).toBeDefined();
  });

  describe('Shift keybind', () => {
    const READY = { available: true, active: false, recharge_pct: 100, auto: false };
    const shiftDown = (code) => document.dispatchEvent(
      new KeyboardEvent('keydown', { code: code || 'ShiftLeft', bubbles: true }));
    const shiftUp = (code) => document.dispatchEvent(
      new KeyboardEvent('keyup', { code: code || 'ShiftLeft', bubbles: true }));

    it('engages boost on hold and releases it on key up', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = READY;
      shiftDown();
      expect(sendAction).toHaveBeenCalledWith('set_boost', { active: true });
      shiftUp();
      expect(sendAction).toHaveBeenCalledWith('set_boost', { active: false });
      expect(sendAction).toHaveBeenCalledTimes(2);
    });

    it('sends once for a held key that auto-repeats', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = READY;
      shiftDown();
      shiftDown();
      shiftDown();
      expect(sendAction).toHaveBeenCalledTimes(1);
    });

    it('keeps boost engaged until the last held source releases', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = READY;
      const btn = el.shadowRoot.getElementById('btn');
      btn.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1 }));
      shiftDown();
      expect(sendAction).toHaveBeenCalledTimes(1);
      // Releasing the pointer must not cut boost while Shift is still down.
      btn.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
      expect(sendAction).toHaveBeenCalledTimes(1);
      shiftUp();
      expect(sendAction).toHaveBeenCalledTimes(2);
      expect(sendAction).toHaveBeenLastCalledWith('set_boost', { active: false });
    });

    it('releases boost when the window loses focus mid-hold', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = READY;
      shiftDown();
      window.dispatchEvent(new Event('blur'));
      expect(sendAction).toHaveBeenLastCalledWith('set_boost', { active: false });
    });

    it('does not engage while recharging or under AUTO', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { available: true, active: false, recharge_pct: 30, auto: false };
      shiftDown();
      shiftUp();
      el.state = { available: true, active: false, recharge_pct: 100, auto: true };
      shiftDown();
      shiftUp();
      expect(sendAction).not.toHaveBeenCalled();
    });

    it('ignores other keys and keys typed into a text field', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = READY;
      document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyW', bubbles: true }));
      const input = document.createElement('input');
      document.body.appendChild(input);
      input.dispatchEvent(new KeyboardEvent('keydown', { code: 'ShiftLeft', bubbles: true }));
      expect(sendAction).not.toHaveBeenCalled();
    });

    it('releases a held boost when removed from the page', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = READY;
      shiftDown();
      el.remove();
      expect(sendAction).toHaveBeenLastCalledWith('set_boost', { active: false });
      shiftDown();
      expect(sendAction).toHaveBeenCalledTimes(2);
    });
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders available state with BOOST text and enabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('BOOST');
    expect(btn.className).toContain('available');
    expect(btn.disabled).toBe(false);
  });

  it('renders active state with BOOSTING text and enabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: true, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('BOOSTING');
    expect(btn.className).toContain('active');
    expect(btn.disabled).toBe(false);
  });

  it('renders active state with percentage when battery partially drained', () => {
    const { el } = setup();
    el.state = { available: true, active: true, recharge_pct: 65, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('BOOSTING 65%');
    expect(btn.className).toContain('active');
    expect(btn.disabled).toBe(false);
  });

  it('renders recharging state with percentage text and disabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 45, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('RECHARGING 45%');
    expect(btn.className).toContain('recharging');
    expect(btn.disabled).toBe(true);
  });

  it('shows battery bar at correct width when recharging', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 30, system_id: '', auto: false };
    const rechargeWrap = el.shadowRoot.getElementById('recharge-wrap');
    const rechargeFill = el.shadowRoot.getElementById('recharge-fill');
    expect(rechargeWrap.style.display).not.toBe('none');
    expect(rechargeFill.style.width).toBe('30%');
    expect(rechargeFill.className).not.toContain('draining');
  });

  it('shows battery bar at correct width when draining (active)', () => {
    const { el } = setup();
    el.state = { available: true, active: true, recharge_pct: 55, system_id: '', auto: false };
    const rechargeWrap = el.shadowRoot.getElementById('recharge-wrap');
    const rechargeFill = el.shadowRoot.getElementById('recharge-fill');
    expect(rechargeWrap.style.display).not.toBe('none');
    expect(rechargeFill.style.width).toBe('55%');
    expect(rechargeFill.className).toContain('draining');
  });

  it('shows AUTO badge and disables button when auto=true', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: true };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe('AUTO');
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.disabled).toBe(true);
  });

  it('holding button when available calls sendAction with set_boost active on pointerdown and inactive on pointerup', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1 }));
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_boost', { active: true });
    btn.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
    expect(sendAction).toHaveBeenCalledTimes(2);
    expect(sendAction).toHaveBeenCalledWith('set_boost', { active: false });
  });

  it('holding button when recharging does not dispatch action', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { available: true, active: false, recharge_pct: 30, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1 }));
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('hides battery bar when fully charged and not active', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: false };
    const rechargeWrap = el.shadowRoot.getElementById('recharge-wrap');
    expect(rechargeWrap.style.display).toBe('none');
  });

  it('hides battery bar when fully charged and active (full boost)', () => {
    const { el } = setup();
    el.state = { available: true, active: true, recharge_pct: 100, system_id: '', auto: false };
    const rechargeWrap = el.shadowRoot.getElementById('recharge-wrap');
    expect(rechargeWrap.style.display).toBe('none');
  });

  it('available=false with full recharge_pct still shows ready state', () => {
    const { el } = setup();
    el.state = { available: false, active: false, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('BOOST');
    expect(btn.className).toContain('available');
  });
});
