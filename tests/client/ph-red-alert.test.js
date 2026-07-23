// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    expect(btn.textContent.trim()).toBe(t('component.red_alert.standby'));
    expect(btn.className).toContain('standby');
    expect(btn.className).not.toContain('active');
  });

  it('renders active state with RED ALERT text and active class', () => {
    const { el } = setup();
    el.state = { system_id: 'red-alert', active: true, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    expect(btn.textContent.trim()).toBe(t('component.red_alert.active'));
    expect(btn.className).toContain('active');
    expect(btn.className).not.toContain('standby');
  });

  it('shows AUTO badge and disables button when auto=true', () => {
    const { el } = setup();
    el.state = { system_id: 'red-alert', active: false, auto: true };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe(t('console.common.auto'));
    const btn = el.shadowRoot.getElementById('alert-btn');
    expect(btn.disabled).toBe(true);
  });

  it('clicking while inactive requests the explicit active state', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { system_id: 'red-alert', active: false, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', { active: true });
  });

  it('clicking while active requests the explicit inactive state', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { system_id: 'red-alert', active: true, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    // Desired = opposite of displayed; the host assigns, so a stale/duplicate
    // click stays idempotent rather than flipping.
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', { active: false });
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
