// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  CAPTAIN_RED_ALERT_ACTION_ID,
  CAPTAIN_WEAPONS_HOLD_ACTION_ID,
} from '../../gui/stations/captain-actions.js';
import '../../gui/components/ph-red-alert.js';

function setup(opts) {
  const activateSemanticAction = opts && opts.activateSemanticAction;
  if (activateSemanticAction) {
    window.activateSemanticAction = activateSemanticAction;
  }
  document.body.innerHTML = '<ph-red-alert id="test-el"></ph-red-alert>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhRedAlert', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    delete window.activateSemanticAction;
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
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { system_id: 'red-alert', active: false, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith(
      CAPTAIN_RED_ALERT_ACTION_ID,
      expect.objectContaining({ context: 'captain', source: 'control' }),
    );
  });

  it('clicking while active requests the explicit inactive state', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { system_id: 'red-alert', active: true, auto: false };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction.mock.calls[0][0]).toBe(CAPTAIN_RED_ALERT_ACTION_ID);
  });

  it('routes the visible Weapons Hold button through its semantic identity', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { active: false, hold: false, auto: false };
    el.shadowRoot.getElementById('hold-btn').click();
    expect(activateSemanticAction).toHaveBeenCalledWith(
      CAPTAIN_WEAPONS_HOLD_ACTION_ID,
      expect.objectContaining({ context: 'captain', source: 'control' }),
    );
  });

  it('clicking button when auto=true does not dispatch action', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { system_id: 'red-alert', active: false, auto: true };
    const btn = el.shadowRoot.getElementById('alert-btn');
    btn.click();
    expect(activateSemanticAction).not.toHaveBeenCalled();
  });
});
