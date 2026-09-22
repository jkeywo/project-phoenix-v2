/**
 * Rendererless GM authentic Station puppeting (issue #1299).
 *
 * The shell mounts the exact authored `StationConfig.console` URL and feeds it
 * the same console-state builder used by player phones. Outbound envelopes go
 * through the existing action map; only resulting ControlSystem commands cross
 * the typed GM action lane.
 *
 * ## Reading the puppet at the desk's own scale (issue #1430)
 *
 * `gui/accessibility-profile.js`'s shell->iframe push (`.console-section
 * iframe`) deliberately never reaches a console iframe — that document
 * "belongs to whoever is sitting at it" (its own private operator). The same
 * is true of `gui/viewscreen-presentation.js`'s endpoint record: "never a
 * console iframe". Both are correct for a PLAYER's own seat or the shared
 * Viewscreen watching one.
 *
 * `#gm-station-frame` is neither. Nobody is privately sitting at it — the GM
 * IS the one reading and operating it, through this very frame, at whatever
 * text scale and contrast the rest of the desk around it already uses. Left
 * unpropagated, the "already-corrected Station family contents" (#1423-1426)
 * would sit at their own 100% default inside the puppet no matter how far the
 * GM turns the desk's own text up — exactly the deferred readability check
 * PRD #1418 / issue #1430 names. So this module mirrors the endpoint's OWN
 * resolved presentation effects — the same `--a11y-text-scale` / contrast /
 * effect-intensity custom properties `gui/gm-workspace.css` already inherits
 * from `server.html`'s root — onto the puppeted document's root each time its
 * state is pushed: once on load, and again on every following `gm_station`
 * tick, so a mid-session change reaches an already-open puppet too.
 */

import { buildRadarRegions } from './console-state.js';
import { dispatchConsoleAction } from './action-map.js';
import {
  ACTION_FEEDBACK_STATE,
  DEFAULT_ACTION_FEEDBACK_CAPACITY,
  DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  isValidActionCorrelation,
} from './action-feedback.js';
import { emptyTutorialProgress, tutorialProgressAfterAction } from './tutorial-state.js';
import { ClientSimState } from './sim-state.js';
import { applyViewscreenEffectsToRoot } from './viewscreen-presentation.js';

/** Tutorial progress of the consoles THIS Game Master has puppeted, by ship.
 *
 * A puppeted console is a real first-run console and carries the same tutorial
 * cards; dismissing one is presentation state, handled where the console is
 * shown and never sent to the host (gui/tutorial-state.js). It is kept here
 * rather than on the projected state, which is rebuilt on every update, and
 * kept per GM session rather than persisted: the human at that Station keeps
 * their own progress, and a GM looking over their shoulder must not write it.
 */
const puppetTutorialProgress = new Map();
const tutorialProgressFor = shipId => {
  if (!puppetTutorialProgress.has(shipId)) puppetTutorialProgress.set(shipId, emptyTutorialProgress());
  return puppetTutorialProgress.get(shipId);
};

const STATION_COMMAND_OUTCOMES = new Set(['applied', 'no-op', 'refused']);
const LOCAL_INGRESS_REFUSAL = 'ingress-rejected';
const LOCAL_FEEDBACK_CAPACITY = 'feedback-capacity';
const LOCAL_FEEDBACK_TIMEOUT = 'feedback-timeout';
const HELD_COMMAND_FIELDS = new Map([
  ['SetThrust', 'value'], ['SetSteering', 'value'],
  ['LateralThrustInput', 'lateral'], ['SetBoost', 'active'],
]);

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
const consoleConfigurations = new WeakMap();
const consoleWorlds = new WeakMap();
const liveEntityFields = ['position', 'yaw', 'hull_fraction', 'shield_fraction', 'shields', 'shield_freq'];
export function buildGmStationConsoleInput(projection, ship, previous = null) {
  const state = previous || new ClientSimState();
  const configuration = JSON.stringify([projection.presentation_generation, ship.ship_id, ship.ship_config]);
  const targets = new Set(ship.objective_targets || []);
  const metadata = projection.entities || ship.entities || [];
  const worldKey = JSON.stringify([projection.presentation_generation, projection.world_revision ?? metadata, ship.objective_targets]);
  const configurationChanged = consoleConfigurations.get(state) !== configuration;
  const worldChanged = configurationChanged || consoleWorlds.get(state)?.key !== worldKey;
  const entities = worldChanged ? metadata.map(entity => projection.entities
    ? { ...entity, objective_target:targets.has(entity.uuid) } : { ...entity }) : state.world.entities;
  if (configurationChanged) {
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
    consoleConfigurations.set(state, configuration);
  } else if (worldChanged) {
    // Absolute metadata replaces the registry (including despawns and removed
    // fields), without replaying the Welcome/configuration reducer.
    state.apply({ type:'WorldSetup', data:{world:{entities, scenario_title:'', scenario_description:''}} });
  }
  if (worldChanged) consoleWorlds.set(state, {key:worldKey, metadata});
  else {
    // SimState's ordinary lane is delta-shaped. Before applying this absolute
    // presentation snapshot, restore omitted live fields from the structural
    // baseline, without replacing the registry or its stable entity objects.
    const baseline = consoleWorlds.get(state).metadata;
    for (let index = 0; index < entities.length; index++) {
      for (const field of liveEntityFields) {
        if (Object.prototype.hasOwnProperty.call(baseline[index], field)) entities[index][field] = baseline[index][field];
        else delete entities[index][field];
      }
    }
  }
  // This lane is absolute, unlike participant blackboard deltas. Clear keys
  // removed since the last snapshot rather than leaving stale readings.
  state.blackboards = {};
  state.blackboardKinds = {};
  state.blackboardPresentation = {};
  state.apply({
    type: 'SimState',
    data: {
      snapshot: {
        entity_states: projection.entity_states || ship.entity_states || [],
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
  state.stationRatings = {...ship.station_ratings};
  state.stationPuppets = stationPuppets;
  state.tutorialProgress = tutorialProgressFor(ship.ship_id);

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
    shipX: { value: pose.x ?? 0, enumerable: true, configurable: true },
    shipY: { value: pose.y ?? 0, enumerable: true, configurable: true },
    shipZ: { value: pose.z ?? 0, enumerable: true, configurable: true },
    shipYaw: { value: pose.yaw ?? 0, enumerable: true, configurable: true },
    forwardSpeed: { value: pose.forward_speed ?? 0, enumerable: true, configurable: true },
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
  requestInterest = () => false,
  submitStationPuppet = () => false,
  submitStationCommand = () => false,
  confirmAction = (request) => request.accept(),
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
  const connection = doc.createElement('p'); connection.id = 'gm-station-connection'; connection.setAttribute('role', 'status');
  panel?.append(connection);
  let frame = doc.getElementById('gm-station-frame');
  const activityList = doc.getElementById('gm-station-activity');
  let projection = { ships: [], activity: [] };
  let selectedKey = null;
  let selectedRow = null;
  let consoleInput = null;
  let loadedUrl = null;
  let mountedKey = null;
  let mountGeneration = 0;
  let lastInterest = null;
  function publishInterest() {
    const request = {consumer:'console', ship:selectedRow?.ship.ship_id || '', station:selectedRow?.station.station_id || '',
      visible:visible && !!selectedRow, mount_generation:mountGeneration, world_generation:projection.presentation_generation || 0};
    const signature = JSON.stringify(request);
    if (signature !== lastInterest && requestInterest(request) !== false) lastInterest = signature;
  }
  function hasCurrentDetail() {
    if (!Array.isArray(projection.detail_ships)) return true;
    const interest = projection.console_interest;
    return projection.detail_ships.includes(selectedRow?.ship.ship_id)
      && interest?.ship === selectedRow?.ship.ship_id && interest?.station === selectedRow?.station.station_id
      && interest?.mount_generation === mountGeneration && interest?.world_generation === projection.presentation_generation;
  }
  // Whether the document currently in the frame is the one this puppet asked
  // for. A frame that loads again after that — because the dock moved the
  // panel, and moving a node between parents re-creates its document — is a
  // REMOUNT, not the mount we were waiting for.
  let mountSettled = false;
  let visible = false;
  let optionsSignature = '', activitySignature = '';
  let releasePending = null;
  let recovery = '';
  let loadTimer = null, loadAttempts = 0, loadFailed = false;
  const appliedPresentation = new WeakMap();
  function cancelLoadWatch() {
    if (loadTimer != null) win.clearTimeout(loadTimer);
    loadTimer = null;
  }
  function watchLoad() {
    if (!visible || loadTimer != null || loadFailed) return;
    loadTimer = win.setTimeout(() => {
      loadTimer = null;
      if (pushState()) return;
      if (++loadAttempts >= 100) {
        loadFailed = true;
        connection.textContent = t('server.gm.station.load_failed');
      } else watchLoad();
    }, 100);
  }
  const activeHere = () => !!getOperator()?.id && getOperator()?.connected !== false
    && selectedRow?.station.operators.includes(getOperator().id);

  function releaseThen(continuation) {
    if (releasePending) return false;
    if (!activeHere()) { continuation(); return true; }
    recovery = t('server.gm.station.releasing');
    const request = { ship: selectedRow.ship.ship_id, station: selectedRow.station.station_id,
      active: false, correlation: correlation('release') };
    const waiting = { continuation, key: selectedKey, correlation: request.correlation, operator: getOperator().id, timer: null };
    releasePending = waiting;
    if (submitStationPuppet(request) !== true) {
      releasePending = null; recovery = t('server.gm.station.release_failed'); renderSelected(); return false;
    }
    waiting.timer = schedule(() => {
      if (releasePending !== waiting) return;
      releasePending = null; recovery = t('server.gm.station.release_failed'); renderSelected();
    }, boundedFeedbackTimeoutMs);
    renderSelected();
    return false;
  }
  const pendingCommands = new Map();
  const boundedPendingCapacity = Math.max(1, Math.min(
    GM_STATION_PENDING_CAPACITY,
    Number.isInteger(pendingCapacity) ? pendingCapacity : GM_STATION_PENDING_CAPACITY,
  ));
  const boundedFeedbackTimeoutMs = Number.isFinite(feedbackTimeoutMs) && feedbackTimeoutMs >= 0
    ? feedbackTimeoutMs : DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS;

  function deliverCommandFeedback(command, state, reason = null) {
    if (!command || command.mountGeneration !== mountGeneration) return false;
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
      mountGeneration,
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

  /**
   * Mirror this endpoint's own resolved presentation onto the puppeted
   * document's root (see the module note above). Late-bound off
   * `win.__serverSettings`, mounted after this module by `server.html`, so an
   * early tick or a test double simply leaves the puppeted document at its
   * own default rather than throwing.
   */
  function applyPresentation() {
    let idoc = null;
    try { idoc = frame && frame.contentDocument; } catch (_) { /* cross-origin — never true here */ }
    if (!idoc || !idoc.documentElement) return;
    const presentation = win.__serverSettings && win.__serverSettings.presentation;
    const effects = typeof presentation?.effects === 'function' ? presentation.effects() : null;
    if (effects) {
      const signature = JSON.stringify(effects);
      if (appliedPresentation.get(idoc.documentElement) !== signature) {
        applyViewscreenEffectsToRoot(idoc.documentElement, effects);
        appliedPresentation.set(idoc.documentElement, signature);
      }
    }
  }

  function pushState() {
    if (!visible || !selectedRow || !consoleInput || !hasCurrentDetail() || !frame || !frame.contentWindow
        || typeof frame.contentWindow.__updateConsole !== 'function') return false;
    const builder = win.buildConsoleState;
    if (typeof builder !== 'function') return false;
    applyPresentation();
    const json = builder(selectedRow.station.station_id, consoleInput);
    frame.contentWindow.__updateConsole(selectedRow.station.station_id, json);
    cancelLoadWatch(); loadFailed = false; loadAttempts = 0;
    const label = t(activeHere() ? 'server.gm.station.controlling' : 'server.gm.station.observing');
    if (connection.textContent !== label) connection.textContent = label;
    frame.style.pointerEvents = activeHere() && !releasePending ? '' : 'none';
    frame.tabIndex = activeHere() && !releasePending ? 0 : -1;
    frame.setAttribute('aria-disabled', String(!activeHere() || !!releasePending));
    return true;
  }

  function renderActivity() {
    if (!activityList) return;
    const entries = !selectedRow ? [] : projection.activity.filter(entry => entry
      && entry.ship === selectedRow.ship.ship_id
      && entry.station === selectedRow.station.station_id).slice(-32);
    const signature = JSON.stringify(entries);
    if (signature === activitySignature) return;
    activitySignature = signature;
    activityList.replaceChildren();
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
      publishInterest();
      if (mountedKey !== null) replaceFrame(null);
      consoleInput = null;
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
      const label = t(locallyActive
        ? 'server.gm.station.release'
        : 'server.gm.station.take_over');
      if (button.textContent !== label) button.textContent = label;
      button.dataset.active = locallyActive ? 'true' : 'false';
      button.disabled = !operatorId || operator.connected === false || !!releasePending;
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

    if (status && recovery) status.textContent = recovery;
    if (!visible) { publishInterest(); return; }
    let replaced = false;
    if (frame && (mountedKey !== selectedKey || loadedUrl !== selectedRow.station.console)) {
      // A new browsing context is the identity boundary. Navigating the same
      // iframe retains its WindowProxy, allowing queued messages from its old
      // document to masquerade as commands for the newly selected Ship.
      replaceFrame(selectedKey);
      replaced = true;
      loadedUrl = selectedRow.station.console;
      mountSettled = false;
      frame.dataset.station = selectedRow.station.station_id;
      frame.dataset.ship = selectedRow.ship.ship_id;
      frame.setAttribute('title', `${selectedRow.ship.name} — ${selectedRow.station.name}`);
      frame.setAttribute('src', selectedRow.station.console);
      connection.textContent = t('server.gm.station.loading');
      watchLoad();
    }
    publishInterest();
    if (!hasCurrentDetail()) {
      consoleInput = null;
      if (button) button.disabled = true;
      connection.textContent = t('server.gm.station.loading');
      return;
    }
    consoleInput = buildGmStationConsoleInput(projection, selectedRow.ship, consoleInput);
    if (!replaced) {
      if (!pushState()) {
        connection.textContent = t(loadFailed ? 'server.gm.station.load_failed' : 'server.gm.station.loading');
        watchLoad();
      }
    }
    renderActivity();
  }

  function replaceFrame(key) {
    if (!frame) return;
    const replacement = frame.cloneNode(false);
    replacement.removeAttribute('src');
    replacement.removeAttribute('data-ship');
    replacement.removeAttribute('data-station');
    frame.replaceWith(replacement);
    frame = replacement;
    mountedKey = key;
    loadedUrl = null;
    invalidateMount();
    frame.addEventListener('load', onFrameLoad);
  }

  /** Feedback belongs to its originating interface; a later mount cannot
   * inherit its timers or correlation, even if it uses the same URL. */
  function invalidateMount() {
    cancelLoadWatch(); loadAttempts = 0; loadFailed = false;
    mountGeneration += 1;
    mountSettled = false;
    for (const command of pendingCommands.values()) {
      if (command.timer != null) cancelSchedule(command.timer);
    }
    pendingCommands.clear();
  }

  function onFrameLoad() {
    // The first load after we set `src` is the mount we asked for. Any load
    // after that is a document we did not ask for — the dock reparented the
    // panel — and the commands the previous document sent must not have their
    // feedback delivered into it.
    if (mountSettled) invalidateMount();
    publishInterest();
    mountSettled = true;
    if (!pushState()) watchLoad();
  }

  function rebuildOptions() {
    if (!select) return;
    const rows = rowsFor(projection);
    const signature = JSON.stringify(rows.map(row => [row.key, row.ship.name, row.station.name]));
    if (signature === optionsSignature) return;
    optionsSignature = signature;
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
    if ((next.presentation_generation ?? 0) < (projection.presentation_generation ?? 0)) return false;
    if (next.presentation_generation !== projection.presentation_generation) {
      consoleInput = null;
      if (projection.presentation_generation != null) invalidateMount();
    }
    projection = next;
    rebuildOptions();
    renderSelected();
    if (releasePending && selectedKey === releasePending.key && !activeHere()) {
      const waiting = releasePending; releasePending = null; recovery = '';
      cancelSchedule(waiting.timer); waiting.continuation();
    }
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
    if (!selectedRow || releasePending) return false;
    recovery = '';
    const operator = getOperator();
    if (!operator || !operator.id || operator.connected === false) return false;
    const active = !selectedRow.station.operators.includes(operator.id);
    const target = selectedRow;
    const generation = mountGeneration;
    const category = !active ? 'station.release'
      : target.station.rating === 'Backfill' ? 'station.takeover' : 'station.takeover-human';
    const description = t('settings.gm.confirmation.station', {
      action: t(`settings.gm.confirmation.${category}`),
      ship: target.ship.name, station: target.station.name,
    });
    return confirmAction({ category, description, preview: () => description,
      accept: () => visible && !releasePending && getOperator()?.connected !== false
        && getOperator()?.id === operator.id && selectedKey === target.key
        && mountGeneration === generation && submitStationPuppet({
        ship: target.ship.ship_id, station: target.station.station_id, active,
        correlation: correlation(active ? 'takeover' : 'release'),
      }) === true,
    });
  }

  function issueConsoleAction(raw) {
    if (!selectedRow || !consoleInput || !hasCurrentDetail()) return false;
    const action = parsePayload(raw);
    if (!action || typeof action !== 'object') return false;
    // The console's tutorial bookkeeping runs here exactly as it does on a
    // player client: a dismiss is recorded and shown, never forwarded; every
    // other action records the control as used and flows on unchanged.
    const shipId = selectedRow.ship.ship_id;
    const hull = selectedRow.ship.ship_config?.hull_id;
    const folded = tutorialProgressAfterAction(tutorialProgressFor(shipId), action, hull);
    if (folded.changed) {
      puppetTutorialProgress.set(shipId, folded.progress);
      renderSelected();
    }
    if (folded.handled) return true;
    let submitted = false;
    dispatchConsoleAction(action, (type, data = {}) => {
      if (submitted || (type !== 'ControlSystem' && type !== 'ControlSystemCorrelated')
          || typeof data.target !== 'string' || !data.payload) return;
      const originatingCorrelation = isValidActionCorrelation(data.correlation)
        ? data.correlation
        : isValidActionCorrelation(action.correlation) ? action.correlation : null;
      const operator = getOperator();
      const frameWindow = frame && frame.contentWindow;
      const requestGeneration = mountGeneration;
      const targetRow = selectedRow;
      let attempted = false;
      const refuseLocal = () => {
        if (originatingCorrelation) deliverCommandFeedback({
          correlation: originatingCorrelation, frameWindow, mountGeneration: requestGeneration,
        }, ACTION_FEEDBACK_STATE.REFUSED, LOCAL_INGRESS_REFUSAL);
      };
      const acceptCommand = () => {
        attempted = true;
        if (!visible || releasePending || !activeHere() || !operator?.id || getOperator()?.id !== operator.id
            || mountGeneration !== requestGeneration
            || frame?.contentWindow !== frameWindow || selectedRow?.key !== targetRow.key) {
          refuseLocal();
          return false;
        }
        let accepted = false;
        try {
          accepted = submitStationCommand({
            ship: targetRow.ship.ship_id,
            station: targetRow.station.station_id,
            target: data.target,
            payload: data.payload,
            correlation: originatingCorrelation || correlation('command'),
          }) === true;
        } catch (_) {
          accepted = false;
        }
        if (accepted && originatingCorrelation) {
          trackPendingCommand(operator.id, originatingCorrelation, frameWindow);
        } else if (!accepted) refuseLocal();
        return accepted;
      };
      const description = t('settings.gm.confirmation.station_command', {
        ship: targetRow.ship.name, station: targetRow.station.name,
      });
      const heldField = HELD_COMMAND_FIELDS.get(data.payload.type);
      const heldValue = data.payload.data?.[heldField];
      submitted = confirmAction({ category: 'station.command',
        key: heldField
          ? `${operator?.id}:${targetRow.key}:${data.target}:${data.payload.type}` : null,
        controlRelease: !!heldField && (heldValue === 0 || heldValue === false),
        description, preview: () => description,
        accept: acceptCommand, onCancel: refuseLocal,
      }) !== false;
      if (!submitted && !attempted) refuseLocal();
    }, patch => {
      if (!consoleInput || !patch || typeof patch !== 'object') return;
      Object.assign(consoleInput, patch);
      pushState();
    });
    return submitted;
  }

  if (select) select.addEventListener('change', () => {
    const next = select.value; select.value = selectedKey;
    releaseThen(() => { selectedKey = next; recovery = ''; renderSelected(); });
  });
  if (button) button.addEventListener('click', toggle);
  if (frame) frame.addEventListener('load', onFrameLoad);
  const onMessage = event => {
    if (!frame || event.source !== frame.contentWindow
        || !event.data || event.data.type !== 'console_action') return;
    issueConsoleAction(event.data.payload);
  };
  win.addEventListener('message', onMessage);

  return {
    update,
    toggle,
    issueConsoleAction,
    settleCommandResults,
    settleLifecycleResults(rows = []) {
      const refusal = releasePending && rows.find(row => row.operator_id === releasePending.operator
        && row.correlation === releasePending.correlation && row.outcome === 'refused');
      if (!refusal) return;
      cancelSchedule(releasePending.timer); releasePending = null;
      recovery = t('server.gm.station.release_failed'); renderSelected();
    },
    refresh: renderSelected,
    setVisible(value) {
      if (visible === (value === true)) return;
      visible = value === true;
      if (!visible) cancelLoadWatch();
      renderSelected();
    },
    mayHide(retry) {
      if (!activeHere() && !releasePending) return true;
      return releaseThen(retry);
    },
    dispose() {
      visible = false; publishInterest();
      invalidateMount();
      if (releasePending) cancelSchedule(releasePending.timer);
      win.removeEventListener('message', onMessage);
    },
    reset() {
      if (releasePending) cancelSchedule(releasePending.timer);
      releasePending = null; recovery = '';
      projection = { ships: [], activity: [], results: [] };
      selectedKey = null; selectedRow = null; consoleInput = null;
      invalidateMount(); rebuildOptions(); renderSelected();
    },
    focusStation(shipId, stationId) {
      const row = rowsFor(projection).find(candidate => candidate.ship.ship_id === shipId
        && candidate.station.station_id === stationId);
      if (!row) return false;
      const focus = () => { selectedKey = row.key; visible = true; recovery = ''; renderSelected(); button?.focus?.(); };
      if (selectedKey === row.key) { focus(); return true; }
      return releaseThen(focus);
    },
    state: () => ({ projection, selectedKey, selectedRow, pendingCommands }),
  };
}
