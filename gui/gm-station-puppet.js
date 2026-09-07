/**
 * Rendererless GM authentic Station puppeting (issue #1299).
 *
 * The shell mounts the exact authored `StationConfig.console` URL and feeds it
 * the same console-state builder used by player phones. Outbound envelopes go
 * through the existing action map; only resulting ControlSystem commands cross
 * the typed GM action lane.
 */

import { buildRadarRegions } from './console-state.js';
import { dispatchConsoleAction } from './action-map.js';
import {
  ACTION_FEEDBACK_STATE,
  DEFAULT_ACTION_FEEDBACK_CAPACITY,
  DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  isValidActionCorrelation,
} from './action-feedback.js';
import { ClientSimState } from './sim-state.js';

const STATION_COMMAND_OUTCOMES = new Set(['applied', 'no-op', 'refused']);
const LOCAL_INGRESS_REFUSAL = 'ingress-rejected';
const LOCAL_FEEDBACK_CAPACITY = 'feedback-capacity';
const LOCAL_FEEDBACK_TIMEOUT = 'feedback-timeout';

export const GM_STATION_PENDING_CAPACITY = DEFAULT_ACTION_FEEDBACK_CAPACITY;

function parsePayload(raw) {
  try {
    return typeof raw === 'string' ? JSON.parse(raw) : raw;
  } catch (_) {
    return undefined;
  }
}

export function parseGmStationProjection(raw) {
  const value = parsePayload(raw);
  if (!value || typeof value !== 'object'
      || !Array.isArray(value.ships) || !Array.isArray(value.activity)
      || !Array.isArray(value.results)) return undefined;
  for (const ship of value.ships) {
    if (!ship || typeof ship !== 'object'
        || typeof ship.ship_id !== 'string' || ship.ship_id.length === 0
        || typeof ship.name !== 'string' || !Array.isArray(ship.stations)
        || !ship.ship_config || typeof ship.ship_config !== 'object'
        || !Array.isArray(ship.blackboards)) return undefined;
    for (const station of ship.stations) {
      if (!station || typeof station.station_id !== 'string'
          || typeof station.name !== 'string'
          || typeof station.console !== 'string' || station.console.length === 0
          || typeof station.rating !== 'string'
          || !Array.isArray(station.operators)) return undefined;
    }
  }
  for (const result of value.results) {
    if (!result || typeof result !== 'object'
        || result.action_kind !== 'station-command'
        || typeof result.operator_id !== 'string' || result.operator_id.length === 0
        || !isValidActionCorrelation(result.correlation)
        || !STATION_COMMAND_OUTCOMES.has(result.outcome)
        || !Number.isSafeInteger(result.tick) || result.tick < 0) return undefined;
  }
  return value;
}

function rowKey(ship, station) {
  return `${ship.ship_id}\u0000${station.station_id}`;
}

function commandResultKey(operatorId, correlation) {
  return `${operatorId}\u0000${correlation}`;
}

function rowsFor(projection) {
  return projection.ships.flatMap(ship => ship.stations.map(station => ({
    ship,
    station,
    key: rowKey(ship, station),
  })));
}

function latestActivity(projection, shipId, stationId) {
  for (let index = projection.activity.length - 1; index >= 0; index -= 1) {
    const entry = projection.activity[index];
    if (entry && entry.ship === shipId && entry.station === stationId) return entry;
  }
  return null;
}

/**
 * Build the ordinary client-state shape consumed by buildConsoleState.
 *
 * The Rust payload stops at the same raw boundary as a player: the exact
 * Welcome `ShipClientConfig`, an absolute SimState entity lane, objective
 * snapshots and tagged blackboards. Fold those through ClientSimState here,
 * then use the shared radar-region builder. This deliberately avoids a second
 * GM-only config/entity/region reducer while still making a newly opened
 * iframe complete on its first projection.
 */
export function buildGmStationConsoleInput(projection, ship) {
  const state = new ClientSimState();
  const entities = (ship.entities || []).map(entity => ({ ...entity }));
  state.apply({
    type: 'Welcome',
    data: {
      state: {
        phase: 'InProgress',
        world: { entities, scenario_title: '', scenario_description: '' },
      },
      ship_config: ship.ship_config,
      station_ratings: ship.station_ratings || {},
    },
  });
  state.apply({
    type: 'SimState',
    data: {
      snapshot: {
        entity_states: ship.entity_states || [],
        navigation_waypoint: ship.navigation_waypoint || null,
        control_sources: ship.control_sources || {},
        station_puppets: [],
      },
    },
  });
  state.apply({
    type: 'ObjectiveSummary',
    data: { objectives: ship.objectives || [] },
  });
  state.apply({
    type: 'BlackboardUpdate',
    data: { updates: ship.blackboards || [] },
  });
  state.apply({
    type: 'SystemHullUpdate',
    data: { entries: ship.console_hull || [] },
  });

  const stationPuppets = {};
  for (const station of ship.stations) {
    if (!Array.isArray(station.operators) || station.operators.length === 0) continue;
    stationPuppets[station.station_id] = {
      station: station.station_id,
      operators: station.operators.slice(),
      latest_activity: latestActivity(projection, ship.ship_id, station.station_id),
    };
  }
  state.controlSources = ship.control_sources || {};
  state.stationPuppets = stationPuppets;
  state.tutorialProgress = {};

  // Regions remain a local presentation projection, just as on player
  // clients. Their only raw inputs are the authoritative entity/objective
  // replicas folded above; the shared builder owns shape semantics.
  state.regions = buildRadarRegions(state.asteroids, state.objectives);

  // ShipPhysics is the absolute fixed-tick source. Own properties intentionally
  // override ClientSimState's blackboard-backed compatibility getters so the
  // first GM projection cannot render the origin while aggregate publication
  // catches up.
  const pose = ship.ship_pose || {};
  Object.defineProperties(state, {
    shipX: { value: pose.x ?? 0, enumerable: true },
    shipY: { value: pose.y ?? 0, enumerable: true },
    shipZ: { value: pose.z ?? 0, enumerable: true },
    shipYaw: { value: pose.yaw ?? 0, enumerable: true },
    forwardSpeed: { value: pose.forward_speed ?? 0, enumerable: true },
  });
  if (Object.prototype.hasOwnProperty.call(ship, 'navigation_waypoint')) {
    state.navigationWaypoint = ship.navigation_waypoint || null;
  }
  return state;
}

export function createGmStationPuppet({
  doc = document,
  win = window,
  t = id => id,
  getOperator = () => null,
  submitStationPuppet = () => false,
  submitStationCommand = () => false,
  pendingCapacity = GM_STATION_PENDING_CAPACITY,
  feedbackTimeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  schedule = (fn, delay) => setTimeout(fn, delay),
  cancelSchedule = timer => clearTimeout(timer),
  correlation = (() => {
    let sequence = 0;
    return kind => `gm-station-${kind}-${Date.now()}-${++sequence}`;
  })(),
} = {}) {
  const pending = doc.getElementById('gm-station-pending');
  const panel = doc.getElementById('gm-station-controls');
  const select = doc.getElementById('gm-station-select');
  const button = doc.getElementById('gm-station-toggle');
  const status = doc.getElementById('gm-station-status');
  const frame = doc.getElementById('gm-station-frame');
  const activityList = doc.getElementById('gm-station-activity');
  let projection = { ships: [], activity: [] };
  let selectedKey = null;
  let selectedRow = null;
  let consoleInput = null;
  let loadedUrl = null;
  const pendingCommands = new Map();
  const boundedPendingCapacity = Math.max(1, Math.min(
    GM_STATION_PENDING_CAPACITY,
    Number.isInteger(pendingCapacity) ? pendingCapacity : GM_STATION_PENDING_CAPACITY,
  ));
  const boundedFeedbackTimeoutMs = Number.isFinite(feedbackTimeoutMs) && feedbackTimeoutMs >= 0
    ? feedbackTimeoutMs : DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS;

  function deliverCommandFeedback(command, state, reason = null) {
    const updateFeedback = command && command.frameWindow
      && command.frameWindow.__updateActionFeedback;
    if (typeof updateFeedback !== 'function') return false;
    return updateFeedback.call(command.frameWindow, {
      correlation: command.correlation,
      state,
      ...(reason ? { reason } : {}),
    }) === true;
  }

  function finishPendingCommand(key, state, reason = null) {
    const command = pendingCommands.get(key);
    if (!command) return false;
    pendingCommands.delete(key);
    if (command.timer != null) cancelSchedule(command.timer);
    return deliverCommandFeedback(command, state, reason);
  }

  function expireOldestPending() {
    const oldest = pendingCommands.keys().next().value;
    if (oldest === undefined) return false;
    return finishPendingCommand(
      oldest,
      ACTION_FEEDBACK_STATE.TIMED_OUT,
      LOCAL_FEEDBACK_CAPACITY,
    );
  }

  function trackPendingCommand(operatorId, originatingCorrelation, frameWindow) {
    while (pendingCommands.size >= boundedPendingCapacity) expireOldestPending();
    const key = commandResultKey(operatorId, originatingCorrelation);
    const command = {
      operatorId,
      correlation: originatingCorrelation,
      frameWindow,
      timer: null,
    };
    pendingCommands.set(key, command);
    const timer = schedule(
      () => finishPendingCommand(
        key,
        ACTION_FEEDBACK_STATE.TIMED_OUT,
        LOCAL_FEEDBACK_TIMEOUT,
      ),
      boundedFeedbackTimeoutMs,
    );
    if (pendingCommands.get(key) === command) command.timer = timer;
    else if (timer != null) cancelSchedule(timer);
  }

  function pushState() {
    if (!selectedRow || !frame || !frame.contentWindow
        || typeof frame.contentWindow.__updateConsole !== 'function') return false;
    const builder = win.buildConsoleState;
    if (typeof builder !== 'function') return false;
    const json = builder(selectedRow.station.station_id, consoleInput);
    frame.contentWindow.__updateConsole(selectedRow.station.station_id, json);
    return true;
  }

  function renderActivity() {
    if (!activityList) return;
    activityList.replaceChildren();
    if (!selectedRow) return;
    const entries = projection.activity.filter(entry => entry
      && entry.ship === selectedRow.ship.ship_id
      && entry.station === selectedRow.station.station_id);
    for (const entry of entries.slice(-32)) {
      const item = doc.createElement('li');
      item.dataset.operator = entry.operator_id;
      item.dataset.target = entry.target;
      item.dataset.action = entry.action;
      item.textContent = t('server.gm.station.activity', {
        operator: entry.operator_id,
        action: entry.action,
        target: entry.target,
        tick: entry.tick,
      });
      activityList.appendChild(item);
    }
  }

  function renderSelected() {
    const rows = rowsFor(projection);
    selectedRow = rows.find(row => row.key === selectedKey) || rows[0] || null;
    selectedKey = selectedRow ? selectedRow.key : null;
    if (!selectedRow) {
      if (pending) pending.hidden = false;
      if (panel) panel.hidden = true;
      renderActivity();
      return;
    }
    if (pending) pending.hidden = true;
    if (panel) panel.hidden = false;
    if (select) select.value = selectedKey;

    const operator = getOperator();
    const operatorId = operator && operator.id;
    const locallyActive = !!operatorId && selectedRow.station.operators.includes(operatorId);
    if (button) {
      button.textContent = t(locallyActive
        ? 'server.gm.station.release'
        : 'server.gm.station.take_over');
      button.dataset.active = locallyActive ? 'true' : 'false';
      button.disabled = !operatorId;
    }
    if (status) {
      if (selectedRow.station.operators.length > 0) {
        status.textContent = t('server.gm.station.operators', {
          operators: selectedRow.station.operators.join(', '),
        });
      } else {
        status.textContent = '';
      }
    }

    consoleInput = buildGmStationConsoleInput(projection, selectedRow.ship);
    if (frame && loadedUrl !== selectedRow.station.console) {
      loadedUrl = selectedRow.station.console;
      frame.dataset.station = selectedRow.station.station_id;
      frame.dataset.ship = selectedRow.ship.ship_id;
      frame.setAttribute('title', `${selectedRow.ship.name} — ${selectedRow.station.name}`);
      frame.setAttribute('src', selectedRow.station.console);
    } else {
      pushState();
    }
    renderActivity();
  }

  function rebuildOptions() {
    if (!select) return;
    const rows = rowsFor(projection);
    select.replaceChildren(...rows.map(row => {
      const option = doc.createElement('option');
      option.value = row.key;
      option.textContent = `${row.ship.name} — ${row.station.name}`;
      return option;
    }));
  }

  function update(raw) {
    const next = parseGmStationProjection(raw);
    if (!next) return false;
    projection = next;
    rebuildOptions();
    renderSelected();
    settleCommandResults(next.results);
    return true;
  }

  function settleCommandResults(results) {
    if (!Array.isArray(results)) return 0;
    let settled = 0;
    for (const result of results) {
      if (!result || result.action_kind !== 'station-command') continue;
      const key = commandResultKey(result.operator_id, result.correlation);
      const state = result.outcome === 'refused'
        ? ACTION_FEEDBACK_STATE.REFUSED
        : ACTION_FEEDBACK_STATE.APPLIED;
      if (finishPendingCommand(key, state)) settled += 1;
    }
    return settled;
  }

  function toggle() {
    if (!selectedRow) return false;
    const operator = getOperator();
    if (!operator || !operator.id) return false;
    const active = !selectedRow.station.operators.includes(operator.id);
    return submitStationPuppet({
      ship: selectedRow.ship.ship_id,
      station: selectedRow.station.station_id,
      active,
      correlation: correlation(active ? 'takeover' : 'release'),
    }) === true;
  }

  function issueConsoleAction(raw) {
    if (!selectedRow) return false;
    const action = parsePayload(raw);
    if (!action || typeof action !== 'object') return false;
    let submitted = false;
    dispatchConsoleAction(action, (type, data = {}) => {
      if (submitted || (type !== 'ControlSystem' && type !== 'ControlSystemCorrelated')
          || typeof data.target !== 'string' || !data.payload) return;
      const originatingCorrelation = isValidActionCorrelation(data.correlation)
        ? data.correlation
        : isValidActionCorrelation(action.correlation) ? action.correlation : null;
      const requestCorrelation = originatingCorrelation || correlation('command');
      const operator = getOperator();
      const frameWindow = frame && frame.contentWindow;
      if (operator && operator.id) {
        try {
          submitted = submitStationCommand({
            ship: selectedRow.ship.ship_id,
            station: selectedRow.station.station_id,
            target: data.target,
            payload: data.payload,
            correlation: requestCorrelation,
          }) === true;
        } catch (_) {
          submitted = false;
        }
      }
      if (originatingCorrelation) {
        if (submitted && operator && operator.id) {
          trackPendingCommand(operator.id, originatingCorrelation, frameWindow);
        } else {
          deliverCommandFeedback({
            correlation: originatingCorrelation,
            frameWindow,
          }, ACTION_FEEDBACK_STATE.REFUSED, LOCAL_INGRESS_REFUSAL);
        }
      }
    }, patch => {
      if (!consoleInput || !patch || typeof patch !== 'object') return;
      Object.assign(consoleInput, patch);
      pushState();
    });
    return submitted;
  }

  if (select) select.addEventListener('change', () => {
    selectedKey = select.value;
    loadedUrl = null;
    renderSelected();
  });
  if (button) button.addEventListener('click', toggle);
  if (frame) frame.addEventListener('load', pushState);
  win.addEventListener('message', event => {
    if (!frame || event.source !== frame.contentWindow
        || !event.data || event.data.type !== 'console_action') return;
    issueConsoleAction(event.data.payload);
  });

  return {
    update,
    toggle,
    issueConsoleAction,
    settleCommandResults,
    refresh: renderSelected,
    state: () => ({ projection, selectedKey, selectedRow, pendingCommands }),
  };
}
