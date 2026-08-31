// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { initConsole } from '../../gui/console-core.js';

describe('console-core semantic action runtime', () => {
  it('registers Helm steering and emits the narrow action from a continuous activation', () => {
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'helm', render: () => {} });
    window.__updateConsole('helm', JSON.stringify({ helm_auto: false }));

    expect(window.activateSemanticAction('helm.steering', {
      context: 'helm', source: 'gamepad', value: 0.45,
    })).toMatchObject({ claimed: true, handled: true });
    expect(sent).toEqual([expect.objectContaining({
      action: 'set_helm_steering', console: 'helm', value: 0.45,
    })]);

    runtime.disposeSemanticActions();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('routes the second real Captain action through the same semantic runtime', () => {
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'captain', render: () => {} });
    window.__updateConsole('captain', JSON.stringify({
      red_alert: false,
      weapons_hold: false,
      red_alert_auto: false,
    }));

    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyH', bubbles: true, cancelable: true,
    }));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      action: 'set_weapons_hold', console: 'captain', held: true,
    });

    runtime.disposeSemanticActions();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('routes default and parent-remapped Captain keys through the legacy action envelope', () => {
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'captain', render: () => {} });
    window.__updateConsole('captain', JSON.stringify({
      red_alert: false,
      red_alert_auto: false,
    }));

    const defaultKey = new KeyboardEvent('keydown', {
      code: 'KeyR', bubbles: true, cancelable: true,
    });
    document.dispatchEvent(defaultKey);
    expect(defaultKey.defaultPrevented).toBe(true);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      action: 'set_red_alert', console: 'captain', active: true,
    });

    window.__updateSemanticActionBindings({
      'captain.red-alert': [{ code: 'KeyY' }, null],
    });
    const stale = new KeyboardEvent('keydown', {
      code: 'KeyR', bubbles: true, cancelable: true,
    });
    document.dispatchEvent(stale);
    expect(stale.defaultPrevented).toBe(false);
    expect(sent).toHaveLength(1);

    const remapped = new KeyboardEvent('keydown', {
      code: 'KeyY', bubbles: true, cancelable: true,
    });
    document.dispatchEvent(remapped);
    expect(remapped.defaultPrevented).toBe(true);
    expect(sent).toHaveLength(2);
    expect(sent[1]).toMatchObject({
      action: 'set_red_alert', console: 'captain', active: true,
    });
    expect(Number.isFinite(sent[1].__input_ms)).toBe(true);

    runtime.disposeSemanticActions();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('does not dispatch repeats or a key pressed in an editable target', () => {
    const send = vi.fn();
    window.__sendAction = send;
    const runtime = initConsole({ name: 'captain', render: () => {} });
    window.__updateConsole('captain', JSON.stringify({
      red_alert: false,
      red_alert_auto: false,
    }));

    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyR', repeat: true, bubbles: true, cancelable: true,
    }));
    const input = document.createElement('input');
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyR', bubbles: true, cancelable: true,
    }));
    expect(send).not.toHaveBeenCalled();

    runtime.disposeSemanticActions();
    input.remove();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });
});
