// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { initConsole } from '../../gui/console-core.js';
import { ActionFeedbackRouter } from '../../gui/action-feedback.js';
import { renderStation as courierCaptainRender } from '../../gui/courier/captain.console.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';

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

  it('presents Captain feedback as an accessible pending-to-final status', () => {
    document.body.innerHTML = '';
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'captain', render: () => {} });
    window.__updateConsole('captain', JSON.stringify({
      red_alert: false,
      weapons_hold: false,
      red_alert_auto: false,
    }));

    expect(window.activateSemanticAction('captain.weapons-hold', {
      context: 'captain', source: 'control',
    })).toMatchObject({ claimed: true, handled: true });
    const status = document.querySelector('.semantic-action-feedback');
    expect(status).not.toBeNull();
    expect(status.getAttribute('role')).toBe('status');
    expect(status.getAttribute('aria-live')).toBe('polite');
    expect(status.dataset.state).toBe('Pending');
    expect(status.textContent).not.toBe('');

    expect(window.__updateActionFeedback({
      correlation: sent[0].correlation,
      state: 'Applied',
    })).toBe(true);
    expect(status.dataset.state).toBe('Applied');
    expect(status.textContent).not.toBe('');

    runtime.disposeSemanticActions();
    expect(document.querySelector('.semantic-action-feedback')).toBeNull();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('routes an authoritative Tactical refusal through the parent router into its originating iframe lifecycle', () => {
    document.body.innerHTML = '';
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'tactical', render: () => {} });
    window.__updateConsole('tactical', JSON.stringify({
      target_uuid: 'enemy-1',
      banks: [{ id: 'fore', fire_ready: true }],
      blasters: [],
      tubes: [],
    }));

    expect(window.activateSemanticAction('tactical.phaser-fire', {
      context: 'tactical', source: 'control', detail: { bank: 'fore' },
    })).toMatchObject({ claimed: true, handled: true });
    const status = document.querySelector('.semantic-action-feedback');
    expect(status.dataset.state).toBe('Pending');

    // This mirrors client.html: the parent response router finds the originating
    // iframe and invokes only its feedback entry point with the terminal result.
    const iframe = { contentWindow: { __updateActionFeedback: window.__updateActionFeedback } };
    const router = new ActionFeedbackRouter({
      schedule: vi.fn(() => 1),
      cancelSchedule: vi.fn(),
      deliver: (value) => iframe.contentWindow.__updateActionFeedback(value),
    });
    expect(router.track({
      correlation: sent[0].correlation,
      actionId: 'tactical.phaser-fire',
      console: 'tactical',
      inputMs: sent[0].__input_ms,
    })).toBe(true);
    const response = {
      type: 'ActionFeedback',
      data: { correlation: sent[0].correlation, outcome: 'Refused' },
    };
    expect(router.resolve(response.data)).toBe(true);
    expect(status.dataset.state).toBe('Refused');

    runtime.disposeSemanticActions();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('keeps Red Alert and View feedback visible across overlap and a cancelled View press', () => {
    document.body.innerHTML = '';
    const sent = [];
    const cues = [];
    const vibrations = [];
    const onCue = (event) => cues.push(event.detail);
    const onVibration = (event) => vibrations.push(event.detail);
    window.addEventListener('phoenix-semantic-cue', onCue);
    window.addEventListener('phoenix-vibration-intent', onVibration);
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'captain', render: () => {} });
    const update = (cameraViews) => window.__updateConsole('captain', JSON.stringify({
      red_alert: false,
      red_alert_auto: false,
      viewscreen_auto: false,
      camera_views: cameraViews,
      view_direction: 'camera_fore',
    }));
    update(['camera_fore', 'cinematic']);

    window.activateSemanticAction('captain.red-alert', { source: 'control' });
    window.activateSemanticAction('captain.view', { source: 'keyboard' });
    const status = document.querySelector('.semantic-action-feedback');
    const states = () => Object.fromEntries(
      [...status.querySelectorAll('.semantic-action-feedback__item')]
        .map((row) => [row.dataset.actionId, row.dataset.state]),
    );
    expect(states()).toEqual({
      'captain.red-alert': 'Pending',
      'captain.view': 'Pending',
    });
    expect(status.dataset.state).toBe('Mixed');
    expect(status.dataset.pendingCount).toBe('2');

    expect(window.__updateActionFeedback({
      correlation: sent[0].correlation,
      state: 'Applied',
    })).toBe(true);
    expect(states()).toEqual({
      'captain.red-alert': 'Applied',
      'captain.view': 'Pending',
    });
    expect(status.dataset.pendingCount).toBe('1');

    const viewPendingCues = () => cues.filter((value) => (
      value.actionId === 'captain.view' && value.cue === 'action-pending'
    ));
    expect(viewPendingCues()).toHaveLength(1);
    update([]);
    expect(window.activateSemanticAction('captain.view', { source: 'keyboard' }))
      .toMatchObject({ claimed: true, handled: false });
    // Cancelling the unavailable newer press promotes the original View
    // occurrence. Its Pending row returns without replaying effects.
    expect(states()).toEqual({
      'captain.red-alert': 'Applied',
      'captain.view': 'Pending',
    });
    expect(viewPendingCues()).toHaveLength(1);
    expect(vibrations).toEqual([
      expect.objectContaining({
        actionId: 'captain.red-alert',
        vibrationIntent: 'confirm',
      }),
    ]);

    runtime.disposeSemanticActions();
    window.removeEventListener('phoenix-semantic-cue', onCue);
    window.removeEventListener('phoenix-vibration-intent', onVibration);
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('cycles only the Courier camera views exposed by its visible renderer', () => {
    document.body.innerHTML = '';
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'captain', render: courierCaptainRender });
    const command = {
      camera_views: ['camera_fore', 'camera_aft', 'cinematic'],
      view_direction: 'camera_fore',
      viewscreen_auto: false,
    };
    window.__updateConsole('captain', JSON.stringify(withConsoleFamilyProjection({
      systems: { captain: command },
    })));

    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyV', bubbles: true, cancelable: true,
    }));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      action: 'set_view',
      console: 'captain',
      direction: 'cinematic',
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
