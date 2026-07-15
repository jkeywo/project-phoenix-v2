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

  describe('Ctrl keybind', () => {
    const ctrlDown = (init) => document.dispatchEvent(
      new KeyboardEvent('keydown', Object.assign({ code: 'ControlLeft', bubbles: true }, init)));

    it('starts the charge when ready', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { state: 'ready', charge_pct: 0, auto: false };
      ctrlDown();
      expect(sendAction).toHaveBeenCalledWith('start_impulse_charge', {});
    });

    it('cancels the charge when already charging', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { state: 'charging', charge_pct: 40, auto: false };
      ctrlDown();
      expect(sendAction).toHaveBeenCalledWith('cancel_impulse', {});
    });

    it('ignores auto-repeat so a held Ctrl does not start/cancel repeatedly', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { state: 'ready', charge_pct: 0, auto: false };
      ctrlDown();
      ctrlDown({ repeat: true });
      ctrlDown({ repeat: true });
      expect(sendAction).toHaveBeenCalledTimes(1);
    });

    it('does nothing under AUTO or on cooldown', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { state: 'ready', charge_pct: 0, auto: true };
      ctrlDown();
      el.state = { state: 'cooldown', charge_pct: 0, auto: false };
      ctrlDown();
      expect(sendAction).not.toHaveBeenCalled();
    });

    it('ignores other keys and keys typed into a text field', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { state: 'ready', charge_pct: 0, auto: false };
      document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyW', bubbles: true }));
      const input = document.createElement('input');
      document.body.appendChild(input);
      input.dispatchEvent(new KeyboardEvent('keydown', { code: 'ControlLeft', bubbles: true }));
      expect(sendAction).not.toHaveBeenCalled();
    });

    it('stops listening once removed from the page', () => {
      const sendAction = vi.fn();
      const { el } = setup({ sendAction });
      el.state = { state: 'ready', charge_pct: 0, auto: false };
      el.remove();
      ctrlDown();
      expect(sendAction).not.toHaveBeenCalled();
    });
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

  it('renders charging state and fills the button itself proportionally', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 42, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.style.getPropertyValue('--charge')).toBe('0.42');
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

  it('resets the charge fill to 0 when not charging', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.style.getPropertyValue('--charge')).toBe('0');
  });
});
