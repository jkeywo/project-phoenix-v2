// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { initConsole } from '../../gui/console-core.js';
import { ActionFeedbackRouter } from '../../gui/action-feedback.js';
import { renderStation as courierCaptainRender } from '../../gui/courier/captain.console.js';
import { supportsCourierTacticalAction } from '../../gui/courier/tactical.console.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { createGamepadInputRuntime } from '../../gui/gamepad-input.js';
import {
  NAVIGATION_CHART_ACTION_ID,
  NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
} from '../../gui/stations/navigation-actions.js';

function disposeRuntime(runtime) {
  runtime.disposeSemanticActions();
  document.body.innerHTML = '';
  delete window.__sendAction;
  delete window.__updateConsole;
  delete window.__updateActionFeedback;
  delete window.__updateSemanticActionBindings;
  delete window.activateSemanticAction;
  delete window.sendAction;
}

function standardPad(pressed = []) {
  const buttons = Array.from({ length: 17 }, () => ({ pressed: false, value: 0 }));
  for (const index of pressed) buttons[index] = { pressed: true, value: 1 };
  return { index: 0, mapping: 'standard', buttons, axes: [0, 0, 0, 0] };
}

describe('console-core semantic action runtime', () => {
  it('keeps Courier Tactical and Helm as independent active keyboard families', () => {
    document.body.innerHTML = [
      '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>',
      '<ph-blasters-controls id="blasters"></ph-blasters-controls>',
      '<ph-helm-joystick id="helm"></ph-helm-joystick>',
      '<ph-lateral-thrust-joystick id="lateral"></ph-lateral-thrust-joystick>',
      '<ph-impulse-btn id="impulse"></ph-impulse-btn>',
      '<ph-boost-btn id="boost"></ph-boost-btn>',
    ].join('');
    const sent = [];
    let context = 'tactical';
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'tactical',
      render: () => {},
      actionFamilies: ['helm'],
      getActionContext: () => context,
      supportsSemanticAction: (actionId, actionContext) => (
        supportsCourierTacticalAction(actionId, actionContext, document)
      ),
    });
    window.__updateConsole('tactical', JSON.stringify({
      systems: {
        'tactical-radar': {
          blips: [{ uuid: 'enemy-1' }], target_uuid: null, tactical_auto: false,
          blasters: [{ id: 'fore', fire_ready: true }],
        },
        'helm-joystick': {
          helm_auto: false, lateral_auto: false, impulse_charge_progress: 0,
          boost_enabled: true, boost_active: false, boost_battery: 1,
          thrust_system_id: 'helm-thrust', steering_system_id: 'helm-steering',
          lateral_system_id: 'helm-lateral', impulse_system_id: 'helm-impulse',
          boost_system_id: 'helm-boost',
        },
      },
      system_ids: ['tactical-radar', 'helm-joystick'],
      system_families: { 'tactical-radar': 'tactical', 'helm-joystick': 'helm' },
    }));
    window.__updateSemanticActionBindings({
      'tactical.target-selection': [{ type: 'keyboard', code: 'KeyU' }, null],
      'helm.impulse': [{ type: 'keyboard', code: 'KeyU' }, null],
    });

    expect(runtime.semanticActions.action('tactical.target-selection')).not.toBeNull();
    expect(runtime.semanticActions.action('helm.impulse')).not.toBeNull();
    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyU', bubbles: true, cancelable: true,
    }));
    context = 'helm';
    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyU', bubbles: true, cancelable: true,
    }));

    expect(sent.map((entry) => [entry.action, entry.console])).toEqual([
      ['set_target', 'tactical'],
      ['start_impulse_charge', 'tactical'],
    ]);
    expect(window.__supportsSemanticAction('helm.thrust', 'helm')).toBe(true);
    expect(window.__supportsSemanticAction('helm.steering', 'helm')).toBe(true);
    expect(window.__supportsSemanticAction('helm.lateral-thrust', 'helm')).toBe(true);
    expect(window.__supportsSemanticAction('helm.impulse', 'helm')).toBe(true);
    expect(window.__supportsSemanticAction('helm.boost', 'helm')).toBe(true);
    expect(window.__supportsSemanticAction('helm.viewscreen', 'helm')).toBe(false);
    expect(window.__supportsSemanticAction('helm.dock', 'helm')).toBe(false);

    runtime.disposeSemanticActions();
    document.body.innerHTML = '';
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.__semanticActionContext;
    delete window.__supportsSemanticActionContext;
    delete window.__supportsSemanticAction;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('releases a Courier Helm hold on blur and dispose after returning to Tactical', () => {
    document.body.innerHTML = [
      '<ph-helm-joystick id="helm"></ph-helm-joystick>',
      '<ph-lateral-thrust-joystick id="lateral"></ph-lateral-thrust-joystick>',
      '<ph-impulse-btn id="impulse"></ph-impulse-btn>',
      '<ph-boost-btn id="boost"></ph-boost-btn>',
    ].join('');
    const sent = [];
    let context = 'helm';
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'tactical',
      render: () => {},
      actionFamilies: ['helm'],
      getActionContext: () => context,
    });
    window.__updateConsole('tactical', JSON.stringify({
      systems: {
        helm: {
          helm_auto: false, boost_enabled: true, boost_active: false, boost_battery: 1,
          boost_system_id: 'helm-boost',
        },
      },
      system_ids: ['helm'],
      system_families: { helm: 'helm' },
    }));

    const pressBoost = () => document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'ShiftLeft', shiftKey: true, bubbles: true, cancelable: true,
    }));
    pressBoost();
    context = 'tactical';
    window.dispatchEvent(new Event('blur'));
    context = 'helm';
    pressBoost();
    context = 'tactical';
    runtime.disposeSemanticActions();

    expect(sent.filter((entry) => entry.action === 'set_boost').map((entry) => entry.active))
      .toEqual([true, false, true, false]);
    document.body.innerHTML = '';
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.__semanticActionContext;
    delete window.__supportsSemanticActionContext;
    delete window.__supportsSemanticAction;
    delete window.activateSemanticAction;
    delete window.sendAction;
  });

  it('registers Helm steering and emits the narrow action from a continuous activation', () => {
    document.body.innerHTML = '<ph-helm-joystick id="helm"></ph-helm-joystick>';
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'helm', render: () => {} });
    window.__updateConsole('helm', JSON.stringify({
      helm_auto: false, steering_system_id: 'helm-steering',
    }));

    expect(window.activateSemanticAction('helm.steering', {
      context: 'helm', source: 'gamepad', value: 0.45,
    })).toMatchObject({ claimed: true, handled: true });
    expect(sent).toEqual([expect.objectContaining({
      action: 'set_helm_steering', console: 'helm', value: 0.45,
    })]);

    runtime.disposeSemanticActions();
    document.body.innerHTML = '';
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
    expect(window.__supportsSemanticAction('captain.red-alert', 'captain')).toBe(true);
    expect(window.__supportsSemanticAction('power.increase-allocation', 'captain')).toBe(false);
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
    expect(runtime.semanticActions.action('power.increase-allocation')).toBeNull();
    expect(runtime.semanticActions.action('repair.dispatch-team')).toBeNull();

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

  it('keeps Captain, Comms, Power, and Repair semantic families live in Courier Captain', () => {
    document.body.innerHTML = '';
    const sent = [];
    let context = 'captain';
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'captain',
      render: courierCaptainRender,
      actionFamilies: ['comms', 'power', 'repair'],
      getActionContext: () => context,
    });
    window.__updateConsole('captain', JSON.stringify(withConsoleFamilyProjection({
      systems: {
        captain: { red_alert: false, red_alert_auto: false, weapons_hold: false },
        comms: {
          contacts: [{ uuid: 'ally-1', in_range: true }],
          messages: [],
        },
        'power-reactor': {
          power_auto: false,
          locked: false,
          consoles: [{
            id: 'helm', commanded_level: 2, level: 2, min_level: 1, max_level: 4,
          }],
        },
        repair: {
          repair_auto: false,
          teams: [{ id: 0, status: 'idle' }],
          dispatch_targets: [{ id: 'core' }],
          damaged_systems: [],
        },
      },
    })));

    for (const actionId of [
      'captain.red-alert', 'comms.hail',
      'power.increase-allocation', 'repair.dispatch-team',
    ]) {
      expect(runtime.semanticActions.action(actionId)).not.toBeNull();
    }
    expect(window.__semanticActionContext()).toBe('captain');
    expect(window.__supportsSemanticActionContext('power')).toBe(true);
    expect(window.__supportsSemanticAction('captain.red-alert', 'captain')).toBe(true);
    expect(window.__supportsSemanticAction('power.increase-allocation', 'captain')).toBe(true);
    expect(window.__supportsSemanticAction('power.increase-allocation', 'power')).toBe(true);
    expect(window.__supportsSemanticAction('power.increase-allocation', 'comms')).toBe(false);

    context = 'comms';
    window.activateSemanticAction('comms.hail', {
      context, source: 'control', detail: { target_uuid: 'ally-1' },
    });
    context = 'power';
    window.activateSemanticAction('power.increase-allocation', {
      context, source: 'control', detail: { target: 'helm', level: 3 },
    });
    context = 'repair';
    window.activateSemanticAction('repair.dispatch-team', {
      context, source: 'control', detail: { team_idx: 0, target: 'core' },
    });
    context = 'captain';
    window.activateSemanticAction('captain.red-alert', { context, source: 'control' });

    expect(sent.map((entry) => entry.action)).toEqual([
      'hail', 'set_power', 'dispatch_repair_team', 'set_red_alert',
    ]);
    expect(sent.every((entry) => entry.console === 'captain')).toBe(true);

    runtime.disposeSemanticActions();
    delete window.__sendAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    delete window.__updateSemanticActionBindings;
    delete window.__semanticActionContext;
    delete window.__supportsSemanticActionContext;
    delete window.__supportsSemanticAction;
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

  it('registers direct Navigation keyboard placement with correlated terminal feedback', () => {
    document.body.innerHTML = '<ph-navigation-map id="navigation-map"></ph-navigation-map>';
    const map = document.getElementById('navigation-map');
    map.navigationPlacement = vi.fn(() => ({ x: 145, z: -230 }));
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({ name: 'navigation', render: () => {} });
    window.__updateConsole('navigation', JSON.stringify({
      navigation_auto: false,
      waypoint: null,
      blips: [],
      civilians: [],
    }));

    const key = new KeyboardEvent('keydown', {
      code: 'KeyP', bubbles: true, cancelable: true,
    });
    document.dispatchEvent(key);

    expect(key.defaultPrevented).toBe(true);
    expect(map.navigationPlacement).toHaveBeenCalledOnce();
    expect(sent).toEqual([expect.objectContaining({
      action: 'set_navigation_waypoint',
      console: 'navigation',
      x: 145,
      z: -230,
      correlation: expect.any(String),
      semantic_action: NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
    })]);
    const status = document.querySelector('.semantic-action-feedback');
    expect(status.dataset.state).toBe('Pending');
    expect(window.__updateActionFeedback({
      correlation: sent[0].correlation,
      state: 'Applied',
    })).toBe(true);
    expect(status.dataset.state).toBe('Applied');

    disposeRuntime(runtime);
  });

  it('adds Navigation beside Captain on the Courier without changing the transport console identity', () => {
    document.body.innerHTML = '<ph-navigation-map id="nav"></ph-navigation-map>';
    const map = document.getElementById('nav');
    map.navigationPlacement = () => ({ x: 9, z: 12 });
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'captain',
      render: () => {},
      actionFamilies: ['captain', 'navigation'],
      navigationActions: { supportsChart: false, supportsCivilianOrders: false },
    });
    window.__updateConsole('captain', JSON.stringify(withConsoleFamilyProjection({
      systems: {
        captain: { red_alert: false, red_alert_auto: false },
        navigation: { navigation_auto: false, waypoint: null, blips: [] },
      },
    })));

    expect(window.activateSemanticAction(NAVIGATION_WAYPOINT_PLACE_ACTION_ID, {
      source: 'gamepad', surface: map,
    })).toMatchObject({ claimed: true, handled: true });
    expect(window.activateSemanticAction('captain.red-alert', {
      source: 'control',
    })).toMatchObject({ claimed: true, handled: true });
    expect(window.activateSemanticAction(NAVIGATION_CHART_ACTION_ID, {
      source: 'control',
    })).toMatchObject({ claimed: false, handled: false });

    expect(sent).toEqual([
      expect.objectContaining({
        action: 'set_navigation_waypoint', console: 'captain', x: 9, z: 12,
      }),
      expect.objectContaining({
        action: 'set_red_alert', console: 'captain', active: true,
      }),
    ]);

    disposeRuntime(runtime);
  });

  it('chooses the open Cruiser Comms Navigation map for a remapped placement action', () => {
    document.body.innerHTML = [
      '<ph-navigation-map id="inline"></ph-navigation-map>',
      '<div class="open"><ph-navigation-map id="overlay"></ph-navigation-map></div>',
    ].join('');
    const inline = document.getElementById('inline');
    const overlay = document.getElementById('overlay');
    inline.navigationPlacement = vi.fn(() => ({ x: 1, z: 2 }));
    overlay.navigationPlacement = vi.fn(() => ({ x: 30, z: 40 }));
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'comms',
      render: () => {},
      actionFamilies: ['comms', 'navigation'],
      navigationActions: { supportsChart: false, supportsCivilianOrders: false },
    });
    window.__updateConsole('comms', JSON.stringify(withConsoleFamilyProjection({
      systems: {
        comms: { comms_auto: false },
        navigation: { navigation_auto: false, waypoint: null, blips: [] },
      },
    })));
    window.__updateSemanticActionBindings({
      [NAVIGATION_WAYPOINT_PLACE_ACTION_ID]: [{ code: 'KeyU' }, null],
    });

    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyU', bubbles: true, cancelable: true,
    }));

    expect(inline.navigationPlacement).not.toHaveBeenCalled();
    expect(overlay.navigationPlacement).toHaveBeenCalledOnce();
    expect(sent).toEqual([expect.objectContaining({
      action: 'set_navigation_waypoint', console: 'comms', x: 30, z: 40,
    })]);

    disposeRuntime(runtime);
  });

  it('routes the real parent Comms gamepad context into its Navigation-capable owning iframe', () => {
    document.body.innerHTML = '<ph-navigation-map id="inline"></ph-navigation-map>';
    const map = document.getElementById('inline');
    map.navigationPlacement = vi.fn(() => ({ x: 70, z: -90 }));
    map.getClientRects = () => [{ width: 300, height: 300 }];
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'comms',
      render: () => {},
      actionFamilies: ['comms', 'navigation'],
      navigationActions: { supportsChart: false, supportsCivilianOrders: false },
    });
    window.__updateConsole('comms', JSON.stringify(withConsoleFamilyProjection({
      systems: {
        comms: { comms_auto: false },
        navigation: { navigation_auto: false, waypoint: null, blips: [] },
      },
    })));

    const parentRegistry = createClientSemanticActionRegistry();
    let pads = [standardPad()];
    const parentGamepad = createGamepadInputRuntime({
      getGamepads: () => pads,
      getContext: () => 'comms',
      getActions: (context) => parentRegistry.list(context),
      isTransportLive: () => true,
      // Same call client.html makes on the iframe resolved from activeConsole.
      activate: (actionId, options) => window.activateSemanticAction(actionId, options),
    });
    parentGamepad.select(0);
    parentGamepad.poll(pads);
    pads = [standardPad([8])]; // standard Select → Navigation place
    parentGamepad.poll(pads);

    expect(map.navigationPlacement).toHaveBeenCalledOnce();
    expect(sent).toEqual([expect.objectContaining({
      action: 'set_navigation_waypoint',
      console: 'comms',
      x: 70,
      z: -90,
      semantic_action: NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
    })]);

    disposeRuntime(runtime);
  });

  it('refuses parent Captain gamepad Navigation input while the Courier overlay is hidden', () => {
    document.body.innerHTML = [
      '<section class="overlay" id="nav-overlay">',
      '<ph-navigation-map id="nav"></ph-navigation-map>',
      '</section>',
    ].join('');
    const map = document.getElementById('nav');
    map.navigationPlacement = vi.fn(() => ({ x: 7, z: 8 }));
    map.getClientRects = () => [];
    const sent = [];
    window.__sendAction = (json) => sent.push(JSON.parse(json));
    const runtime = initConsole({
      name: 'captain',
      render: () => {},
      actionFamilies: ['captain', 'navigation'],
      navigationActions: { supportsChart: false, supportsCivilianOrders: false },
    });
    window.__updateConsole('captain', JSON.stringify(withConsoleFamilyProjection({
      systems: {
        captain: { red_alert: false, red_alert_auto: false },
        navigation: { navigation_auto: false, waypoint: null, blips: [] },
      },
    })));

    const parentRegistry = createClientSemanticActionRegistry();
    let pads = [standardPad()];
    const parentGamepad = createGamepadInputRuntime({
      getGamepads: () => pads,
      getContext: () => 'captain',
      getActions: (context) => parentRegistry.list(context),
      isTransportLive: () => true,
      activate: (actionId, options) => window.activateSemanticAction(actionId, options),
    });
    parentGamepad.select(0);
    parentGamepad.poll(pads);
    pads = [standardPad([8])];
    parentGamepad.poll(pads);

    expect(map.navigationPlacement).not.toHaveBeenCalled();
    expect(sent).toEqual([]);

    disposeRuntime(runtime);
  });
});
