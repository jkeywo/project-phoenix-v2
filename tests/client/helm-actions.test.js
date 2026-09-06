import { describe, expect, it, vi } from 'vitest';
import {
  clientSettingsSemanticActions,
  createClientSemanticActionRegistry,
} from '../../gui/client-semantic-actions.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import {
  HELM_ACTIONS,
  HELM_ACTION_CONTEXT,
  HELM_BOOST_ACTION,
  HELM_BOOST_ACTION_ID,
  HELM_DOCK_ACTION,
  HELM_DOCK_ACTION_ID,
  HELM_IMPULSE_ACTION,
  HELM_IMPULSE_ACTION_ID,
  HELM_LATERAL_ACTION,
  HELM_LATERAL_ACTION_ID,
  HELM_STEERING_ACTION,
  HELM_STEERING_ACTION_ID,
  HELM_THRUST_ACTION,
  HELM_THRUST_ACTION_ID,
  HELM_VIEWSCREEN_ACTION,
  HELM_VIEWSCREEN_ACTION_ID,
  registerHelmActions,
} from '../../gui/stations/helm-actions.js';

function feedback() {
  let serial = 0;
  return {
    press: vi.fn(() => ({ correlation: `helm-${++serial}`, inputMs: serial })),
    pending: vi.fn(), cancel: vi.fn(), settle: vi.fn(),
  };
}

function helmRegistry(options = {}) {
  return registerHelmActions(createSemanticActionRegistry({ actionFeedback: feedback() }), options);
}

describe('Helm semantic actions', () => {
  it('registers every shipped command family with stable two-slot metadata', () => {
    expect(HELM_ACTIONS).toEqual([
      HELM_THRUST_ACTION,
      HELM_STEERING_ACTION,
      HELM_LATERAL_ACTION,
      HELM_IMPULSE_ACTION,
      HELM_BOOST_ACTION,
      HELM_VIEWSCREEN_ACTION,
      HELM_DOCK_ACTION,
    ]);
    for (const action of HELM_ACTIONS) {
      expect(action.contexts).toEqual([HELM_ACTION_CONTEXT]);
      expect(action.bindings).toHaveLength(2);
    }
  });

  it('authors one undirected standard axis with range, cadence, and tuning defaults', () => {
    expect(HELM_STEERING_ACTION).toMatchObject({
      id: HELM_STEERING_ACTION_ID,
      contexts: [HELM_ACTION_CONTEXT],
      continuous: { min: -1, max: 1, neutral: 0, cadenceMs: 100 },
      tuning: { deadzone: 0.1, inverted: false },
      bindings: [
        { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
        null,
      ],
    });
  });

  it('authors thrust and ship-specific lateral thrust as independently bindable axes', () => {
    expect(HELM_THRUST_ACTION).toMatchObject({
      id: HELM_THRUST_ACTION_ID,
      continuous: { min: -1, max: 1, neutral: 0, cadenceMs: 100 },
      bindings: [{ type: 'gamepad', input: 'axis', control: 'left-stick-y' }, null],
    });
    expect(HELM_LATERAL_ACTION).toMatchObject({
      id: HELM_LATERAL_ACTION_ID,
      continuous: { min: -1, max: 1, neutral: 0, cadenceMs: 100 },
      bindings: [{ type: 'gamepad', input: 'axis', control: 'shoulder-pair' }, null],
    });
  });

  it('inverts only the thrust axis so stick-forward drives forward, not reverse', () => {
    // Standard left-stick-y reads UP as -1; forward thrust is +1, so thrust
    // must be inverted while steering (right = +1 = starboard) must not be.
    expect(HELM_THRUST_ACTION.tuning.inverted).toBe(true);
    expect(HELM_STEERING_ACTION.tuning.inverted).toBe(false);
    expect(HELM_LATERAL_ACTION.tuning.inverted).toBe(false);
  });

  it('keeps keyboard/modifier and gamepad defaults in the two discrete slots', () => {
    expect(HELM_IMPULSE_ACTION.bindings).toEqual([
      expect.objectContaining({ type: 'keyboard', code: 'ControlLeft' }),
      { type: 'gamepad', input: 'button', control: 'face-right' },
    ]);
    expect(HELM_BOOST_ACTION).toMatchObject({
      id: HELM_BOOST_ACTION_ID, hold: true, authoritativeFeedback: true,
      bindings: [
        expect.objectContaining({ type: 'keyboard', code: 'ShiftLeft' }),
        { type: 'gamepad', input: 'button', control: 'face-bottom' },
      ],
    });
    expect(HELM_VIEWSCREEN_ACTION.bindings).toEqual([
      expect.objectContaining({ type: 'keyboard', code: 'KeyR' }),
      { type: 'gamepad', input: 'button', control: 'face-top' },
    ]);
    expect(HELM_DOCK_ACTION.bindings).toEqual([
      expect.objectContaining({ type: 'keyboard', code: 'KeyK' }),
      { type: 'gamepad', input: 'button', control: 'face-left' },
    ]);
  });

  it('adapts validated values to only the narrow steering console action', () => {
    const sendAction = vi.fn();
    const registry = registerHelmActions(createSemanticActionRegistry(), {
      getState: () => ({ helm_auto: false, steering_system_id: 'yaw-port' }),
      sendAction,
    });
    expect(registry.activate(HELM_STEERING_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      source: 'gamepad',
      value: -0.35,
    })).toMatchObject({ claimed: true, handled: true });
    expect(sendAction).toHaveBeenCalledOnce();
    expect(sendAction).toHaveBeenCalledWith('set_helm_steering', {
      value: -0.35, control_system_id: 'yaw-port',
    });
  });

  it('does not locally operate Helm while authoritative control source is Backfill', () => {
    const sendAction = vi.fn();
    const registry = registerHelmActions(createSemanticActionRegistry(), {
      getState: () => ({ helm_auto: true }),
      sendAction,
    });
    expect(registry.activate(HELM_STEERING_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      value: 0.8,
    })).toMatchObject({ claimed: true, handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('routes thrust and only authored ship-specific lateral control', () => {
    const sendAction = vi.fn();
    const registry = helmRegistry({
      getState: () => ({
        helm_auto: false,
        lateral_auto: false,
        thrust_system_id: 'main-drive-port',
        lateral_system_id: 'translation-ring',
      }),
      hasLateralControl: () => true,
      sendAction,
    });
    registry.activate(HELM_THRUST_ACTION_ID, { context: HELM_ACTION_CONTEXT, value: 0.6 });
    registry.activate(HELM_LATERAL_ACTION_ID, { context: HELM_ACTION_CONTEXT, value: -0.4 });
    expect(sendAction.mock.calls).toEqual([
      ['set_helm_thrust', { value: 0.6, control_system_id: 'main-drive-port' }],
      ['set_helm_lateral', { value: -0.4, control_system_id: 'translation-ring' }],
    ]);
    const absent = helmRegistry({
      getState: () => ({
        helm_auto: false, lateral_auto: false, lateral_system_id: 'translation-ring',
      }),
      hasLateralControl: () => false,
      sendAction,
    });
    expect(absent.activate(HELM_LATERAL_ACTION_ID, {
      context: HELM_ACTION_CONTEXT, value: 0.5,
    })).toMatchObject({ handled: false });
  });

  it('chooses impulse start/cancel from current authoritative state', () => {
    let charge = 0;
    const sendAction = vi.fn();
    const registry = helmRegistry({
      getState: () => ({
        helm_auto: false,
        impulse_charge_progress: charge,
        impulse_system_id: 'jump-coil-a',
      }),
      sendAction,
    });
    registry.activate(HELM_IMPULSE_ACTION_ID, { context: HELM_ACTION_CONTEXT });
    charge = 0.4;
    registry.activate(HELM_IMPULSE_ACTION_ID, { context: HELM_ACTION_CONTEXT });
    expect(sendAction.mock.calls.map(([name, data]) => [name, data.semantic_action])).toEqual([
      ['start_impulse_charge', HELM_IMPULSE_ACTION_ID],
      ['cancel_impulse', HELM_IMPULSE_ACTION_ID],
    ]);
    expect(sendAction.mock.calls.every(([, data]) => data.correlation)).toBe(true);
    expect(sendAction.mock.calls.every(([, data]) => (
      data.control_system_id === 'jump-coil-a'
    ))).toBe(true);
  });

  it('composes boost hold sources and always releases the final source', () => {
    const sendAction = vi.fn();
    const registry = helmRegistry({
      getState: () => ({
        helm_auto: false,
        boost_enabled: true,
        boost_active: false,
        boost_battery: 10,
        boost_system_id: 'overburner-starboard',
      }),
      sendAction,
    });
    const activate = (source, pressed) => registry.activate(HELM_BOOST_ACTION_ID, {
      context: HELM_ACTION_CONTEXT,
      source: 'control',
      detail: { holdSource: source },
      pressed,
    });
    expect(activate('pointer', true).handled).toBe(true);
    expect(activate('keyboard', true).handled).toBe(true);
    expect(activate('pointer', false).handled).toBe(true);
    expect(activate('keyboard', false).handled).toBe(true);
    expect(sendAction.mock.calls.map(([name, data]) => [name, data.active])).toEqual([
      ['set_boost', true], ['set_boost', false],
    ]);
    expect(sendAction.mock.calls.every(([, data]) => (
      data.control_system_id === 'overburner-starboard'
    ))).toBe(true);
  });

  it('routes viewscreen and contextual dock variants with correlations', () => {
    let docked = false;
    const sendAction = vi.fn();
    const registry = helmRegistry({
      getState: () => ({
        viewscreen_system_id: 'forward-display',
        dock: { system_id: 'berthing-clamps', available: true, engaged: false, docked },
      }),
      hasDockControl: () => true,
      sendAction,
    });
    registry.activate(HELM_VIEWSCREEN_ACTION_ID, { context: HELM_ACTION_CONTEXT });
    registry.activate(HELM_DOCK_ACTION_ID, { context: HELM_ACTION_CONTEXT });
    docked = true;
    registry.activate(HELM_DOCK_ACTION_ID, { context: HELM_ACTION_CONTEXT });
    expect(sendAction.mock.calls.map(([name, data]) => [name, data.control_system_id])).toEqual([
      ['set_radar_view', 'forward-display'],
      ['dock', 'berthing-clamps'],
      ['undock', 'berthing-clamps'],
    ]);
  });

  it('refuses commands whose rendered ship variant has no matching physical control', () => {
    const sendAction = vi.fn();
    const registry = helmRegistry({
      getState: () => ({
        helm_auto: false,
        lateral_auto: false,
        boost_enabled: true,
        boost_battery: 10,
        thrust_system_id: 'drive',
        steering_system_id: 'rudder',
        lateral_system_id: 'lateral',
        impulse_system_id: 'impulse',
        boost_system_id: 'boost',
        viewscreen_system_id: 'screen',
        dock: { system_id: 'dock', available: true, docked: false },
      }),
      hasThrustControl: () => false,
      hasSteeringControl: () => false,
      hasLateralControl: () => false,
      hasImpulseControl: () => false,
      hasBoostControl: () => false,
      hasViewscreenControl: () => false,
      hasDockControl: () => false,
      sendAction,
    });
    for (const [actionId, input] of [
      [HELM_THRUST_ACTION_ID, { value: 0.5 }],
      [HELM_STEERING_ACTION_ID, { value: -0.5 }],
      [HELM_LATERAL_ACTION_ID, { value: 1 }],
      [HELM_IMPULSE_ACTION_ID, {}],
      [HELM_BOOST_ACTION_ID, { pressed: true }],
      [HELM_VIEWSCREEN_ACTION_ID, {}],
      [HELM_DOCK_ACTION_ID, {}],
    ]) {
      expect(registry.activate(actionId, {
        context: HELM_ACTION_CONTEXT, ...input,
      })).toMatchObject({ claimed: true, handled: false });
    }
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('publishes play actions while retaining hidden editor bindings in the parent catalogue', () => {
    const registry = createClientSemanticActionRegistry();
    const ids = registry.list().map((action) => action.id);
    expect(ids).toEqual([
      'captain.red-alert',
      'captain.view',
      'captain.objective-priority',
      ...HELM_ACTIONS.map((action) => action.id),
      'tactical.target-selection',
      'tactical.phaser-mode',
      'tactical.phaser-fire',
      'tactical.blaster-charge',
      'tactical.blaster-fire',
      'tactical.blaster-cancel',
      'tactical.torpedo-volley-down',
      'tactical.torpedo-volley-up',
      'tactical.torpedo-fire',
      'comms.hail',
      'comms.select-message',
      'comms.respond',
      'comms.clear',
      'comms.show-on-screen',
      'sensors.target-selection',
      'sensors.scan',
      'sensors.viewscreen',
      'sensors.cancel-impulse',
      'science.shield-focus',
      'navigation.chart',
      'navigation.waypoint-place',
      'navigation.waypoint-anchor',
      'navigation.waypoint-clear',
      'navigation.contact-selection',
      'navigation.civilian-order',
      'navigation.map-pan-left',
      'navigation.map-pan-right',
      'navigation.map-pan-up',
      'navigation.map-pan-down',
      'navigation.map-zoom-in',
      'navigation.map-zoom-out',
      'power.decrease-allocation',
      'power.increase-allocation',
      'repair.dispatch-team',
      'repair.recall-team',
      'repair.prioritise-system',
      'engineering.tractor',
      'engineering.umbilical',
      'repair.external-dispatch',
      'editor.mod.import',
      'editor.mod.validate',
      'editor.mod.export',
    ]);
    expect(clientSettingsSemanticActions(registry).map((action) => action.id)).toEqual([
      'captain.red-alert',
      'captain.view',
      'captain.objective-priority',
      ...HELM_ACTIONS.map((action) => action.id),
      'tactical.target-selection',
      'tactical.phaser-mode',
      'tactical.phaser-fire',
      'tactical.blaster-charge',
      'tactical.blaster-fire',
      'tactical.blaster-cancel',
      'tactical.torpedo-volley-down',
      'tactical.torpedo-volley-up',
      'tactical.torpedo-fire',
      'comms.hail',
      'comms.select-message',
      'comms.respond',
      'comms.clear',
      'comms.show-on-screen',
      'sensors.target-selection',
      'sensors.scan',
      'sensors.viewscreen',
      'sensors.cancel-impulse',
      'science.shield-focus',
      'navigation.chart',
      'navigation.waypoint-place',
      'navigation.waypoint-anchor',
      'navigation.waypoint-clear',
      'navigation.contact-selection',
      'navigation.civilian-order',
      'navigation.map-pan-left',
      'navigation.map-pan-right',
      'navigation.map-pan-up',
      'navigation.map-pan-down',
      'navigation.map-zoom-in',
      'navigation.map-zoom-out',
      'power.decrease-allocation',
      'power.increase-allocation',
      'repair.dispatch-team',
      'repair.recall-team',
      'repair.prioritise-system',
      'engineering.tractor',
      'engineering.umbilical',
      'repair.external-dispatch',
    ]);
  });
});
