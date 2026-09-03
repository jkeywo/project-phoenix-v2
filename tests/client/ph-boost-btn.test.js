// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-boost-btn.js';
import {
  HELM_ACTION_CONTEXT,
  HELM_BOOST_ACTION_ID,
} from '../../gui/stations/helm-actions.js';

const BOOST_SOURCE = String(customElements.get('ph-boost-btn'));

function setup(opts) {
  const activateSemanticAction = opts && opts.activateSemanticAction;
  if (activateSemanticAction) {
    window.activateSemanticAction = activateSemanticAction;
  }
  document.body.innerHTML = '<ph-boost-btn id="test-el"></ph-boost-btn>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhBoostBtn', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-boost-btn')).toBeDefined();
  });

  it('leaves Shift and gamepad matching to the parent semantic runtime', () => {
    expect(BOOST_SOURCE).not.toContain('observeGamepadButton');
    expect(BOOST_SOURCE).not.toContain("document.addEventListener('keydown'");
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders available state with BOOST text and enabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe(t('component.boost.ready'));
    expect(btn.className).toContain('available');
    expect(btn.disabled).toBe(false);
  });

  it('renders active state with BOOSTING text and enabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: true, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe(t('component.boost.boosting_full'));
    expect(btn.className).toContain('active');
    expect(btn.disabled).toBe(false);
  });

  it('renders active state with percentage when battery partially drained', () => {
    const { el } = setup();
    el.state = { available: true, active: true, recharge_pct: 65, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe(t('component.boost.boosting', { pct: 65 }));
    expect(btn.className).toContain('active');
    expect(btn.disabled).toBe(false);
  });

  it('renders recharging state with percentage text and disabled button', () => {
    const { el } = setup();
    el.state = { available: true, active: false, recharge_pct: 45, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe(t('component.boost.recharging', { pct: 45 }));
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
    expect(badge.textContent.trim()).toBe(t('console.common.auto'));
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.disabled).toBe(true);
  });

  it('holding the button activates and releases the shared boost identity', () => {
    const activateSemanticAction = vi.fn(() => ({ handled: true }));
    const { el } = setup({ activateSemanticAction });
    el.state = { available: true, active: false, recharge_pct: 100, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1 }));
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith(HELM_BOOST_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      source: 'control', detail: { holdSource: 'boost-button' }, pressed: true,
    });
    btn.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
    expect(activateSemanticAction).toHaveBeenCalledTimes(2);
    expect(activateSemanticAction).toHaveBeenLastCalledWith(HELM_BOOST_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      source: 'control', detail: { holdSource: 'boost-button' }, pressed: false,
    });
  });

  it('holding button when recharging does not dispatch action', () => {
    const activateSemanticAction = vi.fn(() => ({ handled: true }));
    const { el } = setup({ activateSemanticAction });
    el.state = { available: true, active: false, recharge_pct: 30, system_id: '', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1 }));
    expect(activateSemanticAction).not.toHaveBeenCalled();
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
    expect(btn.textContent.trim()).toBe(t('component.boost.ready'));
    expect(btn.className).toContain('available');
  });
});
