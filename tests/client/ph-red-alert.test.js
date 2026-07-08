// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-red-alert.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-red-alert id="test-el"></ph-red-alert>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhRedAlert', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-red-alert')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders inactive state with STAND DOWN text and standby class', () => {
    const { el } = setup();
    el.state = { system_id: 'red-alert', active: false, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    expect(btn.textContent.trim()).toBe('STAND DOWN');
    expect(btn.className).toContain('standby');
    expect(btn.className).not.toContain('active');
  });

  it('renders active state with RED ALERT text and active class', () => {
    const { el } = setup();
    el.state = { system_id: 'red-alert', active: true, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    expect(btn.textContent.trim()).toBe('RED ALERT');
    expect(btn.className).toContain('active');
    expect(btn.className).not.toContain('standby');
  });

  it('shows AUTO badge and disables button when auto=true', () => {
    const { el } = setup();
    el.state = { system_id: 'red-alert', active: false, auto: true };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe('AUTO');
    const btn = el.shadowRoot.getElementById('alert-btn');
    expect(btn.disabled).toBe(true);
  });

  it('clicking button calls sendAction with toggle_red_alert', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { system_id: 'red-alert', active: false, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('toggle_red_alert', {});
  });

  it('clicking button when auto=true does not dispatch action', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { system_id: 'red-alert', active: false, auto: true };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
  });
});
