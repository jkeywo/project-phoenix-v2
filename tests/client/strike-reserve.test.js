// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import '../../gui/components/ph-strike-reserve.js';
import { createTacticalActionRegistry } from '../../gui/stations/tactical-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';

describe('Strike reserve controls', () => {
  it('reports empty reserve and global shutoff on both Stations without optimistic state', () => {
    document.body.innerHTML = '<ph-strike-reserve toggle></ph-strike-reserve><ph-strike-reserve></ph-strike-reserve>';
    const [gunnery, power] = document.querySelectorAll('ph-strike-reserve');
    for (const element of [gunnery, power]) {
      element.state = { charge: 0, capacity: 90, enabled: false, depleted: true };
      expect(element.shadowRoot.getElementById('status').textContent).toBe(t('dynasty.boost.depleted'));
      expect(element.shadowRoot.getElementById('charge').getAttribute('aria-valuenow')).toBe('0');
    }
    expect(power.shadowRoot.getElementById('toggle').hidden).toBe(true);
    window.activateSemanticAction = vi.fn();
    gunnery.shadowRoot.getElementById('toggle').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith('tactical.strike-boost', { context: 'tactical' });
    expect(gunnery.shadowRoot.getElementById('toggle').getAttribute('aria-pressed')).toBe('false');
    gunnery.remove();
    document.body.append(gunnery);
    window.activateSemanticAction.mockClear();
    gunnery.shadowRoot.getElementById('toggle').click();
    expect(window.activateSemanticAction).toHaveBeenCalledTimes(1);
    delete window.activateSemanticAction;
  });

  it('uses keyboard and gamepad/profile activation through the same correlated ordinary command', () => {
    const sendAction = vi.fn();
    let enabled = false;
    let sequence = 0;
    const registry = createTacticalActionRegistry({
      getState: () => ({ strike_reserve: { enabled } }), sendAction,
      actionFeedback: new ActionFeedbackLifecycle({ now: () => 1, correlation: () => `boost-${++sequence}` }),
    });
    expect(registry.dispatchKeyboardEvent({ type: 'keydown', code: 'KeyV', preventDefault() {} }, 'tactical').handled).toBe(true);
    expect(sendAction).toHaveBeenLastCalledWith('set_strike_boost', expect.objectContaining({ enabled: true, correlation: 'boost-1' }));
    enabled = true;
    registry.activate('tactical.strike-boost', { context: 'tactical', source: 'gamepad' });
    expect(sendAction).toHaveBeenLastCalledWith('set_strike_boost', expect.objectContaining({ enabled: false }));
  });
});
