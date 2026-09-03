import { describe, expect, it, vi } from 'vitest';
import {
  clientSettingsSemanticActions,
  createClientSemanticActionRegistry,
} from '../../gui/client-semantic-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { createDefaultOperatorProfile } from '../../gui/operator-profile.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import {
  SCIENCE_SHIELD_FOCUS_ACTION,
  SCIENCE_SHIELD_FOCUS_ACTION_ID,
  SENSOR_SCIENCE_ACTIONS,
  SENSORS_SCAN_ACTION,
  SENSORS_CANCEL_IMPULSE_ACTION_ID,
  SENSORS_SCAN_ACTION_ID,
  SENSORS_TARGET_ACTION_ID,
  SENSORS_VIEWSCREEN_ACTION_ID,
  createSensorScienceActionRegistry,
} from '../../gui/stations/sensors-actions.js';

let correlationSequence = 0;

function sensorRegistry(options) {
  return createSensorScienceActionRegistry({
    ...options,
    actionFeedback: new ActionFeedbackLifecycle({
      correlation: () => `sensors-test-${++correlationSequence}`,
      now: () => 123,
    }),
  });
}

function activate(registry, id, context, detail) {
  return registry.activate(id, {
    context,
    detail,
  });
}

function keyedState({ sensors, shields, tactical } = {}) {
  const systems = {};
  const system_families = {};
  const system_ids = [];
  for (const [id, family, view] of [
    ['sensor-radar', 'sensors', sensors],
    ['shield-arc-fore', 'shields', shields],
    ['tactical-radar', 'tactical', tactical],
  ]) {
    if (!view) continue;
    systems[id] = view;
    system_families[id] = family;
    system_ids.push(id);
  }
  return { systems, system_families, system_ids };
}

describe('Sensors and Science semantic definitions', () => {
  it('declares stable identities, real contexts and exactly two device slots', () => {
    expect(SENSOR_SCIENCE_ACTIONS.map((entry) => entry.id)).toEqual([
      SENSORS_TARGET_ACTION_ID,
      SENSORS_SCAN_ACTION_ID,
      SENSORS_VIEWSCREEN_ACTION_ID,
      SENSORS_CANCEL_IMPULSE_ACTION_ID,
      SCIENCE_SHIELD_FOCUS_ACTION_ID,
    ]);
    for (const action of SENSOR_SCIENCE_ACTIONS) {
      expect(action.contexts.length).toBeGreaterThan(0);
      expect(action.authoritativeFeedback).toBe(true);
      expect(action.bindings).toHaveLength(2);
      expect(action.bindings[0]).toMatchObject({ type: 'keyboard' });
      expect(action.bindings[1]).toMatchObject({ type: 'gamepad' });
    }
    expect(SENSOR_SCIENCE_ACTIONS[0].contexts).toEqual([
      'sensors', 'science', 'captain', 'tactical',
    ]);
    expect(SENSOR_SCIENCE_ACTIONS[1].contexts).toEqual(['captain']);
  });

  it('registers conflict-free beside Captain and remains in the private play catalogue', () => {
    let registry;
    expect(() => { registry = createClientSemanticActionRegistry(); }).not.toThrow();
    const playIds = clientSettingsSemanticActions(registry).map((action) => action.id);
    const profile = createDefaultOperatorProfile(registry);
    for (const action of SENSOR_SCIENCE_ACTIONS) {
      expect(registry.action(action.id)).toMatchObject({ id: action.id });
      expect(playIds).toContain(action.id);
      expect(profile.bindings[action.id]).toHaveLength(2);
    }
  });

  it('keeps Captain defaults distinct from Navigation contact and clear controls', () => {
    expect(SENSORS_SCAN_ACTION.bindings[0]).toEqual({
      type: 'keyboard', code: 'KeyN',
      ctrlKey: false, shiftKey: true, altKey: false, metaKey: false,
    });
    expect(SCIENCE_SHIELD_FOCUS_ACTION.bindings[1]).toEqual({
      type: 'gamepad', input: 'button', control: 'left-shoulder',
    });

    const navigationSentinels = [
      {
        id: 'navigation.contact-selection',
        contexts: ['navigation', 'comms', 'captain'],
        bindings: [
          { type: 'keyboard', code: 'KeyN' },
          { type: 'gamepad', input: 'dpad', control: 'dpad-left' },
        ],
      },
      {
        id: 'navigation.waypoint-clear',
        contexts: ['navigation', 'comms', 'captain'],
        bindings: [
          { type: 'keyboard', code: 'KeyC' },
          { type: 'gamepad', input: 'dpad', control: 'dpad-down' },
        ],
      },
    ];
    expect(() => {
      const crossTrack = createSemanticActionRegistry();
      for (const definition of navigationSentinels) {
        crossTrack.register({
          ...definition,
          labelId: `semantic_action.${definition.id}.label`,
          accessibilityLabelId: `semantic_action.${definition.id}.accessibility`,
        });
      }
      for (const action of SENSOR_SCIENCE_ACTIONS) crossTrack.register(action);
    }).not.toThrow();
  });
});

describe('Sensors authority-preserving adapters', () => {
  it.each(['sensors', 'science', 'captain', 'tactical'])(
    'selects the same visible contact through the %s ship variant',
    (context) => {
      const sendAction = vi.fn();
      const registry = sensorRegistry({
        getState: () => keyedState({
          sensors: { blips: [{ uuid: 'visible' }], target_uuid: null },
        }),
        sendAction,
      });
      expect(activate(registry, SENSORS_TARGET_ACTION_ID, context, { uuid: 'visible' }))
        .toMatchObject({ claimed: true, handled: true });
      expect(sendAction).toHaveBeenCalledWith('set_sensors_target', {
        uuid: 'visible',
        correlation: expect.any(String),
        semantic_action: SENSORS_TARGET_ACTION_ID,
        __input_ms: expect.any(Number),
      });
    },
  );

  it('cycles only contacts in the Sensors projection and cannot select Tactical-only data', () => {
    const sendAction = vi.fn();
    const registry = sensorRegistry({
      getState: () => keyedState({
        sensors: { blips: [{ uuid: 'science-a' }, { uuid: 'science-b' }], target_uuid: 'science-a' },
        tactical: { blips: [{ uuid: 'protected-tactical-only' }] },
      }),
      sendAction,
    });

    expect(activate(registry, SENSORS_TARGET_ACTION_ID, 'science'))
      .toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenLastCalledWith(
      'set_sensors_target', expect.objectContaining({ uuid: 'science-b' }),
    );
    expect(activate(registry, SENSORS_TARGET_ACTION_ID, 'science', {
      uuid: 'protected-tactical-only',
    })).toMatchObject({ claimed: true, handled: false });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });

  it('does not fall back from a keyed payload with no Sensors family', () => {
    const sendAction = vi.fn();
    const registry = sensorRegistry({
      getState: () => keyedState({ tactical: { blips: [{ uuid: 'not-science' }] } }),
      sendAction,
    });
    expect(activate(registry, SENSORS_TARGET_ACTION_ID, 'captain'))
      .toMatchObject({ claimed: true, handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('scans only the authoritative current target when the suite reports capable', () => {
    const sendAction = vi.fn();
    let sensors = {
      blips: [{ uuid: 'selected' }], target_uuid: 'selected', scan: { capable: true },
    };
    const registry = sensorRegistry({
      getState: () => keyedState({ sensors }),
      sendAction,
    });

    expect(activate(registry, SENSORS_SCAN_ACTION_ID, 'captain', { uuid: 'selected' }))
      .toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenLastCalledWith('scan_target', expect.objectContaining({
      uuid: 'selected', semantic_action: SENSORS_SCAN_ACTION_ID,
    }));

    expect(activate(registry, SENSORS_SCAN_ACTION_ID, 'captain', { uuid: 'other' }))
      .toMatchObject({ handled: false });
    sensors = { ...sensors, scan: { capable: false } };
    expect(activate(registry, SENSORS_SCAN_ACTION_ID, 'captain'))
      .toMatchObject({ handled: false });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });

  it('still sends an available scan while Sensors is AI-held so admission owns Refused', () => {
    const sendAction = vi.fn();
    const registry = sensorRegistry({
      getState: () => ({
        target_uuid: 'selected', scan: { capable: true }, sensors_auto: true,
      }),
      sendAction,
    });
    expect(activate(registry, SENSORS_SCAN_ACTION_ID, 'captain'))
      .toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenCalledOnce();
  });

  it('shows only the Sensors projection on the viewscreen', () => {
    const sendAction = vi.fn();
    const registry = sensorRegistry({
      getState: () => ({ blips: [] }),
      sendAction,
    });
    expect(activate(registry, SENSORS_VIEWSCREEN_ACTION_ID, 'sensors'))
      .toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenCalledWith('set_view', expect.objectContaining({
      direction: 'SensorsRadar', semantic_action: SENSORS_VIEWSCREEN_ACTION_ID,
    }));
  });

  it('mirrors the shipped Cancel Impulse enablement without changing impulse state', () => {
    const sendAction = vi.fn();
    let state = { impulse_charge_progress: 0.25, sensors_auto: false };
    const registry = sensorRegistry({ getState: () => state, sendAction });
    expect(activate(registry, SENSORS_CANCEL_IMPULSE_ACTION_ID, 'sensors'))
      .toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenLastCalledWith('cancel_impulse', expect.objectContaining({
      semantic_action: SENSORS_CANCEL_IMPULSE_ACTION_ID,
    }));
    state = { ...state, sensors_auto: true };
    expect(activate(registry, SENSORS_CANCEL_IMPULSE_ACTION_ID, 'sensors'))
      .toMatchObject({ handled: false });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });

  it('focuses only an authored Shields projection facing and preserves toggle detail', () => {
    const sendAction = vi.fn();
    const registry = sensorRegistry({
      getState: () => keyedState({
        shields: {
          facings: [{ arc_id: 'fore', label: 'Fore' }, { arc_id: 'aft', label: 'Aft' }],
          focused_facing: 'fore',
          shields_auto: false,
        },
      }),
      sendAction,
    });
    expect(activate(registry, SCIENCE_SHIELD_FOCUS_ACTION_ID, 'science', {
      arc_id: 'fore', focused: false,
    })).toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenCalledWith('set_shield_focus', expect.objectContaining({
      arc_id: 'fore', focused: false, semantic_action: SCIENCE_SHIELD_FOCUS_ACTION_ID,
    }));
    expect(activate(registry, SCIENCE_SHIELD_FOCUS_ACTION_ID, 'science', {
      arc_id: 'unpublished', focused: true,
    })).toMatchObject({ handled: false });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });
});
