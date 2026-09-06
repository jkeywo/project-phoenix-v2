import { describe, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import {
  createNavigationActionRegistry,
  NAVIGATION_ACTION_CONTEXT,
  NAVIGATION_ACTION_CONTEXTS,
  NAVIGATION_ACTIONS,
  NAVIGATION_CHART_ACTION_ID,
  NAVIGATION_CIVILIAN_ORDER_ACTION_ID,
  NAVIGATION_CONTACT_ACTION_ID,
  NAVIGATION_MAP_PRESENTATION_ACTION_IDS,
  NAVIGATION_PAN_DOWN_ACTION_ID,
  NAVIGATION_PAN_LEFT_ACTION_ID,
  NAVIGATION_PAN_RIGHT_ACTION_ID,
  NAVIGATION_PAN_UP_ACTION_ID,
  NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID,
  NAVIGATION_WAYPOINT_CLEAR_ACTION_ID,
  NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
  NAVIGATION_ZOOM_IN_ACTION_ID,
  NAVIGATION_ZOOM_OUT_ACTION_ID,
} from '../../gui/stations/navigation-actions.js';

let sequence = 0;
function registry(options = {}) {
  return createNavigationActionRegistry({
    ...options,
    actionFeedback: new ActionFeedbackLifecycle({
      now: () => 321,
      correlation: () => `navigation-${++sequence}`,
      onTransition: options.onTransition || (() => {}),
    }),
  });
}

const navigationView = () => ({
  navigation_auto: false,
  waypoint: { name: 'Existing' },
  blips: [
    { uuid: 'contact-a', world_x: 10, world_z: 20 },
    { uuid: 'contact-b', world_x: 30, world_z: 40 },
  ],
  civilians: [
    {
      uuid: 'civilian-a',
      order_options: [
        { order: { verb: 'hold' } },
        { order: { verb: 'divert', route: 'safe-lane' } },
      ],
    },
  ],
});

// #1286 is integrated beside independently-authored Comms, Engineering and
// Sensors families. These are those families' complete authored identities,
// contexts and defaults, rather than a hand-picked set of controls Navigation
// happens to avoid. Registering them beside the real parent catalogue makes
// semantic-action-registry perform the same exhaustive pairwise validation as
// the integrated client while the issue tracks remain independently staged.
const authoredKey = (code, modifiers = {}) => ({
  type: 'keyboard', code,
  ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
  ...modifiers,
});
const authoredButton = (control) => ({ type: 'gamepad', input: 'button', control });
const authoredDpad = (control) => ({ type: 'gamepad', input: 'dpad', control });
const authoredAxis = (control, direction) => ({
  type: 'gamepad', input: 'axis', control, direction, threshold: 0.5,
});
const authoredAction = (id, contexts, keyboard, gamepad) => ({
  id,
  contexts,
  labelId: `integration.${id}.label`,
  accessibilityLabelId: `integration.${id}.accessibility`,
  authoritativeFeedback: true,
  bindings: [keyboard, gamepad],
});

const CONCURRENT_TRACK_ACTIONS = Object.freeze([
  authoredAction('comms.hail', ['comms'], authoredKey('KeyH'), authoredDpad('dpad-left')),
  authoredAction('comms.select-message', ['comms'], authoredKey('KeyM'), authoredDpad('dpad-up')),
  authoredAction('comms.respond', ['comms'], authoredKey('KeyR'), authoredButton('face-bottom')),
  authoredAction('comms.clear', ['comms'], authoredKey('KeyC', { shiftKey: true }), authoredDpad('dpad-down')),
  authoredAction('comms.show-on-screen', ['comms'], authoredKey('KeyV'), authoredDpad('dpad-right')),
  authoredAction(
    'power.decrease-allocation',
    ['power', 'engineering', 'captain'],
    authoredKey('KeyQ'),
    authoredAxis('right-stick-x', 'negative'),
  ),
  authoredAction(
    'power.increase-allocation',
    ['power', 'engineering', 'captain'],
    authoredKey('KeyE'),
    authoredAxis('right-stick-x', 'positive'),
  ),
  authoredAction(
    'repair.dispatch-team',
    ['repair', 'engineering', 'captain'],
    authoredKey('KeyD', { shiftKey: true }),
    authoredAxis('left-stick-y', 'negative'),
  ),
  authoredAction(
    'repair.prioritise-system',
    ['repair', 'engineering', 'captain'],
    authoredKey('KeyP', { shiftKey: true }),
    authoredAxis('left-stick-y', 'positive'),
  ),
  authoredAction(
    'engineering.tractor', ['engineering'], authoredKey('KeyT'), authoredButton('face-bottom'),
  ),
  authoredAction(
    'engineering.umbilical', ['engineering'], authoredKey('KeyU'), authoredDpad('dpad-left'),
  ),
  authoredAction(
    'repair.external-dispatch',
    ['repair', 'engineering'],
    authoredKey('KeyX'),
    authoredDpad('dpad-right'),
  ),
  authoredAction(
    'sensors.target-selection',
    ['sensors', 'science', 'captain', 'tactical'],
    authoredKey('KeyS'),
    authoredButton('face-left'),
  ),
  authoredAction(
    'sensors.scan', ['captain'], authoredKey('KeyN', { shiftKey: true }), authoredButton('face-top'),
  ),
  authoredAction(
    'sensors.viewscreen',
    ['sensors', 'science', 'captain', 'tactical'],
    authoredKey('KeyU'),
    authoredButton('face-right'),
  ),
  authoredAction(
    'sensors.cancel-impulse', ['sensors'], authoredKey('KeyX'), authoredButton('face-bottom'),
  ),
  authoredAction(
    'science.shield-focus',
    ['science', 'captain', 'engineering', 'shields'],
    authoredKey('KeyF'),
    authoredButton('left-shoulder'),
  ),
]);

function integratedClientCatalogue() {
  const catalogue = createClientSemanticActionRegistry();
  for (const action of CONCURRENT_TRACK_ACTIONS) {
    // Once the parallel family is integrated, keep using its production
    // definition; before integration, register the exact cross-track contract.
    if (!catalogue.action(action.id)) catalogue.register(action);
  }
  return catalogue;
}

function idsFor(catalogue, context) {
  return catalogue.list(context).map((action) => action.id).sort();
}

describe('Navigation semantic actions', () => {
  it('publishes every shipped Navigation intent in its real host contexts with two device slots', () => {
    expect(NAVIGATION_ACTIONS).toHaveLength(12);
    expect(NAVIGATION_ACTION_CONTEXTS).toEqual(['navigation', 'comms', 'captain']);
    for (const action of NAVIGATION_ACTIONS) {
      if (action.id === NAVIGATION_CHART_ACTION_ID
          || action.id === NAVIGATION_CIVILIAN_ORDER_ACTION_ID) {
        expect(action.contexts).toEqual([NAVIGATION_ACTION_CONTEXT]);
      } else {
        expect(action.contexts).toEqual(NAVIGATION_ACTION_CONTEXTS);
      }
      expect(action.bindings).toHaveLength(2);
      expect(action.bindings[0]?.type).toBe('keyboard');
      expect(action.bindings[1]?.type).toBe('gamepad');
      if (action.id === NAVIGATION_CONTACT_ACTION_ID
          || NAVIGATION_MAP_PRESENTATION_ACTION_IDS.includes(action.id)) {
        expect(action.feedback).toBe('local');
      } else {
        expect(action.authoritativeFeedback).toBe(true);
      }
    }
    const byId = Object.fromEntries(NAVIGATION_ACTIONS.map((action) => [action.id, action]));
    expect(byId[NAVIGATION_WAYPOINT_PLACE_ACTION_ID].bindings[1].control).toBe('select');
    expect(byId[NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID].bindings[1].control).toBe('start');
    expect(byId[NAVIGATION_WAYPOINT_CLEAR_ACTION_ID].bindings[1].control).toBe('left-stick-button');
    expect(byId[NAVIGATION_CONTACT_ACTION_ID].bindings[1].control).toBe('right-stick-button');
    expect(NAVIGATION_MAP_PRESENTATION_ACTION_IDS).toEqual([
      NAVIGATION_PAN_LEFT_ACTION_ID,
      NAVIGATION_PAN_RIGHT_ACTION_ID,
      NAVIGATION_PAN_UP_ACTION_ID,
      NAVIGATION_PAN_DOWN_ACTION_ID,
      NAVIGATION_ZOOM_IN_ACTION_ID,
      NAVIGATION_ZOOM_OUT_ACTION_ID,
    ]);
    expect(NAVIGATION_MAP_PRESENTATION_ACTION_IDS.map((id) => (
      byId[id].bindings.map((binding) => binding.type)
    ))).toEqual(Array.from({ length: 6 }, () => ['keyboard', 'gamepad']));
    expect(byId[NAVIGATION_PAN_LEFT_ACTION_ID].bindings[0]).toMatchObject({
      code: 'ArrowLeft', ctrlKey: true,
    });
    expect(byId[NAVIGATION_PAN_LEFT_ACTION_ID].bindings[1]).toMatchObject({
      control: 'left-stick-x', direction: 'negative',
    });
    expect(byId[NAVIGATION_PAN_UP_ACTION_ID].bindings[1]).toMatchObject({
      control: 'left-trigger', input: 'button',
    });
    expect(byId[NAVIGATION_PAN_DOWN_ACTION_ID].bindings[1]).toMatchObject({
      control: 'right-trigger', input: 'button',
    });
    expect(byId[NAVIGATION_ZOOM_IN_ACTION_ID].bindings[1]).toMatchObject({
      control: 'right-stick-y', direction: 'negative',
    });
    expect(byId[NAVIGATION_ZOOM_OUT_ACTION_ID].bindings[1]).toMatchObject({
      control: 'right-stick-y', direction: 'positive',
    });

    expect(byId[NAVIGATION_CHART_ACTION_ID].bindings[1]).toMatchObject({
      control: 'left-stick-y', direction: 'negative',
    });
    expect(byId[NAVIGATION_CIVILIAN_ORDER_ACTION_ID].bindings[1]).toMatchObject({
      control: 'left-stick-y', direction: 'positive',
    });
  });

  it('registers the exhaustive Captain/Courier, Comms, Engineering and Sensors catalogue', () => {
    let catalogue;
    expect(() => { catalogue = integratedClientCatalogue(); }).not.toThrow();

    expect(idsFor(catalogue, 'captain')).toEqual([
      'captain.objective-priority',
      'captain.red-alert',
      'captain.view',
      'captain.weapons-hold',
      'navigation.contact-selection',
      'navigation.map-pan-down',
      'navigation.map-pan-left',
      'navigation.map-pan-right',
      'navigation.map-pan-up',
      'navigation.map-zoom-in',
      'navigation.map-zoom-out',
      'navigation.waypoint-anchor',
      'navigation.waypoint-clear',
      'navigation.waypoint-place',
      'power.decrease-allocation',
      'power.increase-allocation',
      'repair.dispatch-team',
      'repair.prioritise-system',
      'repair.recall-team',
      'science.shield-focus',
      'sensors.scan',
      'sensors.target-selection',
      'sensors.viewscreen',
    ].sort());
    expect(idsFor(catalogue, 'comms')).toEqual([
      'comms.clear',
      'comms.hail',
      'comms.respond',
      'comms.select-message',
      'comms.show-on-screen',
      'navigation.contact-selection',
      'navigation.map-pan-down',
      'navigation.map-pan-left',
      'navigation.map-pan-right',
      'navigation.map-pan-up',
      'navigation.map-zoom-in',
      'navigation.map-zoom-out',
      'navigation.waypoint-anchor',
      'navigation.waypoint-clear',
      'navigation.waypoint-place',
    ].sort());
    expect(idsFor(catalogue, 'engineering')).toEqual([
      'engineering.tractor',
      'engineering.umbilical',
      'power.decrease-allocation',
      'power.increase-allocation',
      'repair.dispatch-team',
      'repair.external-dispatch',
      'repair.prioritise-system',
      'repair.recall-team',
      'science.shield-focus',
    ].sort());
    expect(idsFor(catalogue, 'sensors')).toEqual([
      'sensors.cancel-impulse',
      'sensors.target-selection',
      'sensors.viewscreen',
    ].sort());
  });

  it('uses the keyboard place binding and the map keyboard cursor through one correlated operation', () => {
    const sendAction = vi.fn();
    const surface = { navigationPlacement: vi.fn(() => ({ x: 12.5, z: -8 })) };
    const actions = registry({ getState: navigationView, getSurface: () => surface, sendAction });
    const event = {
      type: 'keydown', code: 'KeyP', cancelable: true, preventDefault: vi.fn(),
    };

    const result = actions.dispatchKeyboardEvent(event, NAVIGATION_ACTION_CONTEXT);

    expect(result).toMatchObject({
      claimed: true, handled: true, actionId: NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
      correlation: expect.any(String),
    });
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(surface.navigationPlacement).toHaveBeenCalledOnce();
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', {
      x: 12.5,
      z: -8,
      correlation: result.correlation,
      semantic_action: NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
      __input_ms: 321,
    });
  });

  it('keeps pointer coordinates exact and resolves an anchored waypoint from the live blip projection', () => {
    const sendAction = vi.fn();
    const actions = registry({ getState: navigationView, sendAction });

    expect(actions.activate(NAVIGATION_WAYPOINT_PLACE_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
      source: 'control',
      detail: { x: -50, z: 75 },
    })).toMatchObject({ handled: true });
    expect(actions.activate(NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
      source: 'control',
      detail: { source_uuid: 'contact-b', x: 999, z: 999 },
    })).toMatchObject({ handled: true });

    expect(sendAction.mock.calls.map(([name, payload]) => [name, {
      x: payload.x, z: payload.z, source_uuid: payload.source_uuid,
    }])).toEqual([
      ['set_navigation_waypoint', { x: -50, z: 75, source_uuid: undefined }],
      ['set_navigation_waypoint', { x: 30, z: 40, source_uuid: 'contact-b' }],
    ]);
  });

  it('matches visible eligibility for clear/chart and cannot bypass Navigation Auto', () => {
    const sendAction = vi.fn();
    let view = navigationView();
    const actions = registry({ getState: () => view, getSurface: () => ({}), sendAction });

    expect(actions.activate(NAVIGATION_CHART_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
    })).toMatchObject({ handled: true });
    expect(actions.activate(NAVIGATION_WAYPOINT_CLEAR_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
    })).toMatchObject({ handled: true });

    view = { ...view, navigation_auto: true };
    for (const actionId of [
      NAVIGATION_CHART_ACTION_ID,
      NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
      NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID,
      NAVIGATION_WAYPOINT_CLEAR_ACTION_ID,
      NAVIGATION_CIVILIAN_ORDER_ACTION_ID,
    ]) {
      expect(actions.activate(actionId, {
        context: NAVIGATION_ACTION_CONTEXT,
        detail: actionId === NAVIGATION_WAYPOINT_PLACE_ACTION_ID
          ? { x: 1, z: 2 }
          : actionId === NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID
            ? { source_uuid: 'contact-a' }
            : undefined,
      })).toMatchObject({ claimed: true, handled: false });
    }
    expect(sendAction).toHaveBeenCalledTimes(2);

    view = { ...view, navigation_auto: false, waypoint: null };
    expect(actions.activate(NAVIGATION_WAYPOINT_CLEAR_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
    })).toMatchObject({ handled: false });
  });

  it('selects a contact locally and settles the shared feedback lifecycle without a wire command', () => {
    const transitions = [];
    const sendAction = vi.fn();
    const surface = { navigationSelect: vi.fn(() => true) };
    const actions = registry({
      getState: navigationView,
      getSurface: () => surface,
      sendAction,
      onTransition: (value) => transitions.push(value),
    });

    const result = actions.activate(NAVIGATION_CONTACT_ACTION_ID, {
      context: 'captain', source: 'gamepad', detail: { direction: -1 },
    });

    expect(result).toMatchObject({ handled: true, correlation: expect.any(String) });
    expect(surface.navigationSelect).toHaveBeenCalledWith({ direction: -1 });
    expect(sendAction).not.toHaveBeenCalled();
    expect(transitions.map((entry) => entry.state)).toEqual(['Pressed', 'Pending', 'Applied']);
  });

  it('routes every pan and zoom identity through the map and shared local feedback', () => {
    const transitions = [];
    const sendAction = vi.fn();
    const surface = {
      navigationPan: vi.fn(() => true),
      navigationZoom: vi.fn(() => true),
    };
    const actions = registry({
      getState: navigationView,
      getSurface: () => surface,
      sendAction,
      onTransition: (value) => transitions.push(value),
    });

    for (const actionId of NAVIGATION_MAP_PRESENTATION_ACTION_IDS) {
      expect(actions.activate(actionId, {
        context: NAVIGATION_ACTION_CONTEXT,
        source: 'gamepad',
      })).toMatchObject({ claimed: true, handled: true });
    }

    expect(surface.navigationPan.mock.calls.map(([detail]) => detail.direction)).toEqual([
      'left', 'right', 'up', 'down',
    ]);
    expect(surface.navigationZoom.mock.calls.map(([detail]) => detail.direction)).toEqual([
      'in', 'out',
    ]);
    expect(transitions.map((entry) => entry.state)).toEqual(
      NAVIGATION_MAP_PRESENTATION_ACTION_IDS.flatMap(() => ['Pressed', 'Pending', 'Applied']),
    );
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('uses only currently published civilian options and supports unqualified gamepad activation', () => {
    const sendAction = vi.fn();
    const actions = registry({ getState: navigationView, sendAction });

    expect(actions.activate(NAVIGATION_CIVILIAN_ORDER_ACTION_ID, {
      context: 'comms', source: 'gamepad',
    })).toMatchObject({ claimed: false, handled: false });
    expect(actions.activate(NAVIGATION_CIVILIAN_ORDER_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
      detail: { target: 'civilian-a', verb: 'divert', route: 'safe-lane' },
    })).toMatchObject({ handled: true });
    expect(actions.activate(NAVIGATION_CIVILIAN_ORDER_ACTION_ID, {
      context: NAVIGATION_ACTION_CONTEXT,
      detail: { target: 'civilian-a', verb: 'divert', route: 'invented-lane' },
    })).toMatchObject({ handled: false });

    expect(sendAction.mock.calls.map(([name, payload]) => [name, {
      target: payload.target,
      verb: payload.verb,
      route: payload.route,
    }])).toEqual([
      ['order_civilian', { target: 'civilian-a', verb: 'divert', route: 'safe-lane' }],
    ]);
  });
});
