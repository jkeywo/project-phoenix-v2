// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { CAPTAIN_RED_ALERT_ACTION_ID } from '../../gui/stations/captain-actions.js';
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

  // Issue #1398: the component carries ONE button. The Weapons Hold beside it
  // (issue #1041) is retired — restraint is a power order from Engineering — so
  // the markup, the CSS and the second feedback lane went with it.
  it('renders no weapons-hold control at all', () => {
    const { el } = setup();
    el.state = { active: false, auto: false };
    expect(el.shadowRoot.getElementById('hold-btn')).toBeNull();
    expect(el.shadowRoot.querySelectorAll('button')).toHaveLength(1);
  });

  it('marks the Red Alert control busy only while its own action is pending', () => {
    const { el } = setup();
    el.state = { active: false, auto: false };
    const alertButton = el.shadowRoot.getElementById('alert-btn');

    window.dispatchEvent(new CustomEvent('phoenix-action-feedback', {
      detail: {
        actionId: CAPTAIN_RED_ALERT_ACTION_ID,
        state: 'Pending',
        statusId: 'action_feedback.pending',
        isCurrent: true,
      },
    }));
    expect(alertButton.getAttribute('aria-busy')).toBe('true');
    expect(el.shadowRoot.getElementById('feedback-status').textContent)
      .toBe(t('action_feedback.pending'));

    window.dispatchEvent(new CustomEvent('phoenix-action-feedback', {
      detail: {
        actionId: CAPTAIN_RED_ALERT_ACTION_ID,
        state: 'Applied',
        statusId: 'action_feedback.applied',
        isCurrent: true,
      },
    }));
    expect(alertButton.hasAttribute('aria-busy')).toBe(false);
  });

  it('ignores feedback addressed to another action', () => {
    const { el } = setup();
    el.state = { active: false, auto: false };
    window.dispatchEvent(new CustomEvent('phoenix-action-feedback', {
      detail: {
        actionId: 'captain.view',
        state: 'Pending',
        statusId: 'action_feedback.pending',
        isCurrent: true,
      },
    }));
    expect(el.shadowRoot.getElementById('alert-btn').hasAttribute('aria-busy')).toBe(false);
    expect(el.shadowRoot.getElementById('feedback-status').textContent).toBe('');
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
