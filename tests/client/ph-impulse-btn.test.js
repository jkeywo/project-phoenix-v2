// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-impulse-btn.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-impulse-btn id="test-el"></ph-impulse-btn>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhImpulseBtn', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-impulse-btn')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders ready state with IMPULSE text and enabled button', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('IMPULSE');
    expect(btn.className).toContain('ready');
    expect(btn.disabled).toBe(false);
  });

  it('renders charging state as a tappable CANCEL button showing percentage', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 67, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    // Pressing IMPULSE again while charging cancels it, so the button stays
    // enabled and reads CANCEL rather than being an inert CHARGING label.
    expect(btn.textContent.trim()).toBe('CANCEL 67%');
    expect(btn.className).toContain('charging');
    expect(btn.disabled).toBe(false);
  });

  it('disables the charging button under AUTO so the operator cannot cancel', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 67, system_id: 'helm-impulse', auto: true };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.disabled).toBe(true);
  });

  it('renders charging state and shows progress bar at correct width', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 42, system_id: 'helm-impulse', auto: false };
    const progressWrap = el.shadowRoot.getElementById('progress-wrap');
    const progressFill = el.shadowRoot.getElementById('progress-fill');
    expect(progressWrap.style.display).not.toBe('none');
    expect(progressFill.style.width).toBe('42%');
  });

  it('renders cooldown state with COOLDOWN text and disabled button', () => {
    const { el } = setup();
    el.state = { state: 'cooldown', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe('COOLDOWN');
    expect(btn.className).toContain('cooldown');
    expect(btn.disabled).toBe(true);
  });

  it('shows AUTO badge and disables button when auto=true', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: true };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe('AUTO');
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.disabled).toBe(true);
  });

  it('clicking button when ready calls sendAction with start_impulse_charge', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('start_impulse_charge', {});
  });

  it('clicking button when charging dispatches cancel_impulse', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { state: 'charging', charge_pct: 50, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('cancel_impulse', {});
  });

  it('hides progress bar when not charging', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const progressWrap = el.shadowRoot.getElementById('progress-wrap');
    expect(progressWrap.style.display).toBe('none');
  });
});
