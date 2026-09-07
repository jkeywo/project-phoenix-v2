/**
 * Context-scoped Navigation semantic actions.
 *
 * The Navigation family appears as a direct Station, inside the Cruiser's
 * Comms surface, and inside the Courier's Captain surface.  Those are real
 * overlapping input contexts: a binding which is safe on the direct chart may
 * still collide with a Captain command on the compact hull.  The action
 * metadata therefore names all three host contexts while the console runtime
 * registers these adapters only on surfaces which actually render Navigation.
 *
 * Waypoint and civilian state stay authoritative.  Adapters read the latest
 * Navigation projection, emit the existing narrow console actions, and never
 * patch a waypoint or compliance row locally.
 */

import { ACTION_FEEDBACK_STATE } from '../action-feedback.js';
import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const NAVIGATION_ACTION_CONTEXT = 'navigation';
export const NAVIGATION_ACTION_CONTEXTS = Object.freeze([
  NAVIGATION_ACTION_CONTEXT,
  'comms',
  'captain',
]);

export const NAVIGATION_CHART_ACTION_ID = 'navigation.chart';
export const NAVIGATION_WAYPOINT_PLACE_ACTION_ID = 'navigation.waypoint-place';
export const NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID = 'navigation.waypoint-anchor';
export const NAVIGATION_WAYPOINT_CLEAR_ACTION_ID = 'navigation.waypoint-clear';
export const NAVIGATION_CONTACT_ACTION_ID = 'navigation.contact-selection';
export const NAVIGATION_CIVILIAN_ORDER_ACTION_ID = 'navigation.civilian-order';
export const NAVIGATION_PAN_LEFT_ACTION_ID = 'navigation.map-pan-left';
export const NAVIGATION_PAN_RIGHT_ACTION_ID = 'navigation.map-pan-right';
export const NAVIGATION_PAN_UP_ACTION_ID = 'navigation.map-pan-up';
export const NAVIGATION_PAN_DOWN_ACTION_ID = 'navigation.map-pan-down';
export const NAVIGATION_ZOOM_IN_ACTION_ID = 'navigation.map-zoom-in';
export const NAVIGATION_ZOOM_OUT_ACTION_ID = 'navigation.map-zoom-out';

export const NAVIGATION_MAP_PRESENTATION_ACTION_IDS = Object.freeze([
  NAVIGATION_PAN_LEFT_ACTION_ID,
  NAVIGATION_PAN_RIGHT_ACTION_ID,
  NAVIGATION_PAN_UP_ACTION_ID,
  NAVIGATION_PAN_DOWN_ACTION_ID,
  NAVIGATION_ZOOM_IN_ACTION_ID,
  NAVIGATION_ZOOM_OUT_ACTION_ID,
]);

const key = (code, modifiers = {}) => Object.freeze({
  type: 'keyboard', code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
  ...modifiers,
});
const button = (control) => Object.freeze({ type: 'gamepad', input: 'button', control });
const axis = (control, direction) => Object.freeze({
  type: 'gamepad', input: 'axis', control, direction, threshold: 0.5,
});

function action(
  id, label, keyboard, gamepad, feedback = 'authoritative', contexts = NAVIGATION_ACTION_CONTEXTS,
) {
  return Object.freeze({
    id,
    contexts,
    labelId: `semantic_action.navigation.${label}.label`,
    accessibilityLabelId: `semantic_action.navigation.${label}.accessibility`,
    ...(feedback === 'local' ? { feedback: 'local' } : { authoritativeFeedback: true }),
    bindings: Object.freeze([keyboard, gamepad]),
  });
}

// Navigation is embedded inside real Captain and Comms documents.  The parent
// gamepad runtime still addresses those host contexts, so these defaults must
// be collision-free across the whole composite rather than merely within the
// map overlay. The direct station alone ships chart/civilian controls; keeping
// those identities in that real context leaves the paired triggers available
// for vertical pan beside the horizontal stick axis. Zoom remains paired on
// the right stick's vertical axis.
export const NAVIGATION_ACTIONS = Object.freeze([
  action(
    NAVIGATION_CHART_ACTION_ID,
    'chart',
    key('KeyJ'),
    axis('left-stick-y', 'negative'),
    'authoritative',
    Object.freeze([NAVIGATION_ACTION_CONTEXT]),
  ),
  action(NAVIGATION_WAYPOINT_PLACE_ACTION_ID, 'waypoint_place', key('KeyP'), button('select')),
  action(NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID, 'waypoint_anchor', key('KeyA'), button('start')),
  action(NAVIGATION_WAYPOINT_CLEAR_ACTION_ID, 'waypoint_clear', key('KeyC'), button('left-stick-button')),
  action(NAVIGATION_CONTACT_ACTION_ID, 'contact', key('KeyN'), button('right-stick-button'), 'local'),
  action(
    NAVIGATION_CIVILIAN_ORDER_ACTION_ID,
    'civilian_order',
    key('KeyD'),
    axis('left-stick-y', 'positive'),
    'authoritative',
    Object.freeze([NAVIGATION_ACTION_CONTEXT]),
  ),
  action(NAVIGATION_PAN_LEFT_ACTION_ID, 'pan_left', key('ArrowLeft', { ctrlKey: true }), axis('left-stick-x', 'negative'), 'local'),
  action(NAVIGATION_PAN_RIGHT_ACTION_ID, 'pan_right', key('ArrowRight', { ctrlKey: true }), axis('left-stick-x', 'positive'), 'local'),
  action(NAVIGATION_PAN_UP_ACTION_ID, 'pan_up', key('ArrowUp', { ctrlKey: true }), button('left-trigger'), 'local'),
  action(NAVIGATION_PAN_DOWN_ACTION_ID, 'pan_down', key('ArrowDown', { ctrlKey: true }), button('right-trigger'), 'local'),
  action(NAVIGATION_ZOOM_IN_ACTION_ID, 'zoom_in', key('Equal'), axis('right-stick-y', 'negative'), 'local'),
  action(NAVIGATION_ZOOM_OUT_ACTION_ID, 'zoom_out', key('Minus'), axis('right-stick-y', 'positive'), 'local'),
]);

export function navigationActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, NAVIGATION_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

/** Flatten one authoritative civilian option into action-map arguments. */
export function civilianOrderActionArgs(target, order) {
  if (!target || !order || typeof order.verb !== 'string') return null;
  if (order.verb === 'hold') return { target, verb: 'hold' };
  if (order.verb === 'divert') {
    const hasRoute = typeof order.route === 'string' && order.route.length > 0;
    const hasAnchor = typeof order.anchor === 'string' && order.anchor.length > 0;
    if (hasRoute === hasAnchor) return null;
    return hasRoute
      ? { target, verb: 'divert', route: order.route }
      : { target, verb: 'divert', anchor: order.anchor };
  }
  if (order.verb === 'dock'
      && typeof order.structure === 'string' && order.structure.length > 0) {
    return { target, verb: 'dock', structure: order.structure };
  }
  return null;
}

function correlatedPayload(actionId, correlation, inputMs, payload) {
  if (typeof correlation !== 'string' || !correlation) return null;
  return { ...payload, correlation, semantic_action: actionId, __input_ms: inputMs };
}

function finitePosition(value) {
  return value && Number.isFinite(value.x) && Number.isFinite(value.z)
    ? { x: value.x, z: value.z }
    : null;
}

function surfaceFor(provided, getSurface) {
  return provided || (typeof getSurface === 'function' ? getSurface() : null);
}

function publishedBlips(view) {
  return Array.isArray(view && view.blips) ? view.blips : [];
}

function anchoredWaypoint(view, surface, detail) {
  const requested = detail && (detail.source_uuid || detail.uuid);
  const selected = requested || (surface && typeof surface.navigationSelectedUuid === 'function'
    ? surface.navigationSelectedUuid() : null);
  if (typeof selected !== 'string' || !selected) return null;
  const blip = publishedBlips(view).find((entry) => entry && entry.uuid === selected);
  if (!blip || !Number.isFinite(blip.world_x) || !Number.isFinite(blip.world_z)) return null;
  return { x: blip.world_x, z: blip.world_z, source_uuid: blip.uuid };
}

function civilianOrders(view) {
  const rows = Array.isArray(view && view.civilians) ? view.civilians : [];
  const result = [];
  for (const row of rows) {
    if (!row || typeof row.uuid !== 'string' || !row.uuid) continue;
    for (const option of (Array.isArray(row.order_options) ? row.order_options : [])) {
      const args = civilianOrderActionArgs(row.uuid, option && option.order);
      if (args) result.push(args);
    }
  }
  return result;
}

function sameCivilianOrder(left, right) {
  if (!left || !right || left.target !== right.target || left.verb !== right.verb) return false;
  if (left.verb === 'hold') return true;
  if (left.verb === 'divert') {
    return (left.route || null) === (right.route || null)
      && (left.anchor || null) === (right.anchor || null);
  }
  return left.structure === right.structure;
}

/** Register every shipped Navigation intent on one actual Navigation surface. */
export function registerNavigationActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('navigation action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const getSurface = typeof options.getSurface === 'function' ? options.getSurface : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const supportsChart = options.supportsChart !== false;
  const supportsCivilianOrders = options.supportsCivilianOrders !== false;
  const send = (actionId, correlation, inputMs, name, payload) => {
    const correlated = correlatedPayload(actionId, correlation, inputMs, payload);
    if (!correlated || !sendAction) return false;
    sendAction(name, correlated);
    return true;
  };

  registry.register(NAVIGATION_ACTIONS[0], ({ actionId, correlation, inputMs } = {}) => {
    const view = navigationActionView(getState());
    if (!supportsChart || !view || view.navigation_auto || !sendAction) return false;
    return send(actionId, correlation, inputMs, 'set_navigation_chart', {});
  });

  registry.register(NAVIGATION_ACTIONS[1], ({
    actionId, correlation, inputMs, detail, surface,
  } = {}) => {
    const view = navigationActionView(getState());
    if (!view || view.navigation_auto) return false;
    const owner = surfaceFor(surface, getSurface);
    const position = finitePosition(detail)
      || (owner && typeof owner.navigationPlacement === 'function'
        ? finitePosition(owner.navigationPlacement()) : null);
    return position
      ? send(actionId, correlation, inputMs, 'set_navigation_waypoint', position)
      : false;
  });

  registry.register(NAVIGATION_ACTIONS[2], ({
    actionId, correlation, inputMs, detail, surface,
  } = {}) => {
    const view = navigationActionView(getState());
    if (!view || view.navigation_auto) return false;
    const waypoint = anchoredWaypoint(view, surfaceFor(surface, getSurface), detail);
    return waypoint
      ? send(actionId, correlation, inputMs, 'set_navigation_waypoint', waypoint)
      : false;
  });

  registry.register(NAVIGATION_ACTIONS[3], ({ actionId, correlation, inputMs } = {}) => {
    const view = navigationActionView(getState());
    const owner = surfaceFor(null, getSurface);
    if (!view || view.navigation_auto || !view.waypoint || !owner) return false;
    return send(actionId, correlation, inputMs, 'clear_navigation_waypoint', {});
  });

  registry.register(NAVIGATION_ACTIONS[4], ({ detail, surface, settleFeedback } = {}) => {
    const owner = surfaceFor(surface, getSurface);
    if (!owner || typeof owner.navigationSelect !== 'function') return false;
    const handled = owner.navigationSelect(detail || { direction: 1 });
    if (!handled) return false;
    if (typeof settleFeedback === 'function') settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    return true;
  });

  registry.register(NAVIGATION_ACTIONS[5], ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = navigationActionView(getState());
    if (!supportsCivilianOrders || !view || view.navigation_auto) return false;
    const available = civilianOrders(view);
    const requested = detail && available.find((entry) => sameCivilianOrder(entry, detail));
    const order = detail ? requested : available[0];
    return order
      ? send(actionId, correlation, inputMs, 'order_civilian', order)
      : false;
  });

  const registerLocalMapAction = (definition, method, defaultDetail) => {
    registry.register(definition, ({ detail, surface, settleFeedback } = {}) => {
      const owner = surfaceFor(surface, getSurface);
      if (!owner || typeof owner[method] !== 'function') return false;
      const handled = owner[method](detail || defaultDetail);
      if (!handled) return false;
      if (typeof settleFeedback === 'function') settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
      return true;
    });
  };
  registerLocalMapAction(NAVIGATION_ACTIONS[6], 'navigationPan', { direction: 'left' });
  registerLocalMapAction(NAVIGATION_ACTIONS[7], 'navigationPan', { direction: 'right' });
  registerLocalMapAction(NAVIGATION_ACTIONS[8], 'navigationPan', { direction: 'up' });
  registerLocalMapAction(NAVIGATION_ACTIONS[9], 'navigationPan', { direction: 'down' });
  registerLocalMapAction(NAVIGATION_ACTIONS[10], 'navigationZoom', { direction: 'in' });
  registerLocalMapAction(NAVIGATION_ACTIONS[11], 'navigationZoom', { direction: 'out' });
  return registry;
}

export function createNavigationActionRegistry(options = {}) {
  return registerNavigationActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') window.registerNavigationActions = registerNavigationActions;
