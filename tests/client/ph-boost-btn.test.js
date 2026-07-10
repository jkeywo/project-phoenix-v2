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

  it('renders recharging state with percentage text and disabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 45, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('RECHARGING 45%');
    expect(btn.className).toContain('recharging');
    expect(btn.disabled).toBe(true);
  });

  it('renders recharging state and shows recharge bar at correct width', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 30, system_id: '', auto: false };
    const rechargeWrap = el.shadowRoot.getElementById('recharge-wrap');
    const rechargeFill = el.shadowRoot.getElementById('recharge-fill');
    expect(rechargeWrap.style.display).not.toBe('none');
    expect(rechargeFill.style.width).toBe('30%');
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

  it('hides recharge bar when not recharging', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: false };
    const rechargeWrap = el.shadowRoot.getElementById('recharge-wrap');
    expect(rechargeWrap.style.display).toBe('none');
  });

  it('available=false with full recharge_pct still shows recharging state (disabled)', () => {
    const { el } = setup();
    el.state = { available: false, active: false, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('BOOST');
    expect(btn.className).toContain('available');
  });
});
