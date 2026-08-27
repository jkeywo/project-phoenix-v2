/**
 * gui/dirty-consoles.js — declarative message → dirty-console mapping (#823).
 *
 * Replaces the hand-maintained fan-out in client.html handleMessage(): given
 * an inbound ServerMessage, `dirtyConsolesFor` returns the set of console
 * (station) names whose iframes should be re-pushed. The mapping is data:
 *
 *  - STATIC_MESSAGE_FAMILIES covers every non-blackboard message type with a
 *    fixed Console Family list. The authoritative topology then resolves each
 *    family to the actual Station ids that own its Systems.
 *  - BlackboardUpdate dirtiness is derived from reducer-reported changed keys,
 *    the server-supplied station→systems ownership, authoritative System-id →
 *    Console Family projection, and the separate reserved-blackboard-key
 *    projection. The router never re-reads the wire payload. No id spelling
 *    participates in routing, and reserved keys never masquerade as Systems.
 *
 * On the battleship (identity stations) this reproduces the old hardcoded
 * routing exactly. On composite stations (courier 'pilot', destroyer
 * 'engineering'/'tactical', cruiser 'science'/'comms') it routes to the
 * owning station's console — an intended improvement; the old cascade could
 * never reach 'pilot' or 'engineering'.
 *
 * Deliberately NOT modelled: builders that read *other* systems' blackboards
 * (cross-reads) are served by the 10 Hz SimState tick, exactly as before.
 * Adding new cross-read edges here would change push cadence — don't, unless
 * the old imperative fan-out had the edge too.
 *
 * Pure module: no DOM, no window reads inside the functions, unit-tested in
 * tests/client/dirty-consoles.test.js.
 */

import { CHANGE_DOMAINS } from './reducer-result.js';

/**
 * Non-blackboard message type → Console Families dirtied. The list preserves
 * the old per-message fan-out, while `dirtyConsolesFor` resolves those families
 * through the current hull's actual ownership.
 */
export const STATIC_MESSAGE_FAMILIES = Object.freeze({
  // Push the pre-seeded repair teams so the Repair console renders its rows
  // immediately on (re)connect, before the first RepairState broadcast.
  Welcome: Object.freeze(['repair']),
  // Every radar-bearing console refreshes each 10 Hz tick: radar blips are
  // built client-side from ship_x/z/yaw + the entity array, so consoles that
  // only refreshed on their sparse own-event cadence would crawl at ~1 Hz.
  SimState: Object.freeze(['tactical', 'repair', 'sensors', 'navigation']),
  WorldSetup: Object.freeze(['tactical', 'helm', 'sensors', 'navigation']),
  EntitySpawned: Object.freeze(['tactical', 'helm', 'sensors', 'navigation']),
  AsteroidSpawned: Object.freeze(['tactical', 'helm', 'sensors', 'navigation']),
  TargetLock: Object.freeze(['tactical']),
  WeaponsUpdate: Object.freeze(['tactical']),
  // BeamStarted/BeamEnded entries removed in #825: sim-state no longer
  // mutates on those messages (the weaponsFiring flag they fed is gone),
  // so a push would rebuild an unchanged payload.
  SystemHullUpdate: Object.freeze(['repair']),
  RepairState: Object.freeze(['repair']),
  PowerState: Object.freeze(['power']),
  ShieldStatus: Object.freeze(['shields']),
  AsteroidDestroyed: Object.freeze(['tactical', 'helm', 'sensors']),
  EntityDespawned: Object.freeze(['tactical', 'helm', 'sensors']),
  CommsState: Object.freeze(['comms']),
  // Rejection feedback (#761 AC3): re-push the comms console so the attempted
  // response button flashes red.
  CommsResponseRejected: Object.freeze(['comms']),
  // stationRatings is already fresh when this fires (sim-state applied the
  // message first); controlSources lags until the next SimState tick, so the
  // captain badge is pushed immediately from here.
  RatingChanged: Object.freeze(['captain']),
});

/**
 * Console Families whose owning Stations are NOT gated on being active.
 * The captain push was always unconditional (BlackboardUpdate + RatingChanged
 * pushed it regardless of which console is on screen) — keep that as data
 * rather than an inline special case in the driver.
 */
export const ALWAYS_PUSH_FAMILIES = Object.freeze(new Set(['captain']));

/**
 * The console family a blackboard system id belongs to, or null.
 * Actual System ids and non-System blackboard keys deliberately use separate
 * authoritative projections.
 */
function familyForBlackboardId(id, systemConsoleFamilies, blackboardConsoleFamilies) {
  if (typeof id !== 'string') return null;
  const reserved = blackboardConsoleFamilies && blackboardConsoleFamilies[id];
  if (typeof reserved === 'string' && reserved !== '') return reserved;
  const system = systemConsoleFamilies && systemConsoleFamilies[id];
  return typeof system === 'string' && system !== '' ? system : null;
}

/**
 * Console families a single blackboard update fans out to: its own family,
 * plus — for captain/viewscreen updates — the currentView cascade.
 */
function familiesForBlackboardId(id, systemConsoleFamilies, blackboardConsoleFamilies) {
  const family = familyForBlackboardId(id, systemConsoleFamilies, blackboardConsoleFamilies);
  if (!family) return [];
  if (family === 'captain') {
    // Helm/Sensors/Comms/Navigation each derive their own "on screen" button
    // state from simState.currentView, so a captain/viewscreen change must
    // refresh all of them too, not just the captain console — otherwise the
    // button that requested the view change never reflects the toggle (see
    // #on-screen-btn on Helm).
    return ['captain', 'helm', 'sensors', 'comms', 'navigation'];
  }
  return [family];
}

/**
 * Resolve a console family to the console (station) names that render it,
 * from the server-supplied stationSystems (station id → owned fine system
 * ids); station id and Console Family are independent namespaces. Before
 * Welcome there is no topology to route through, so the result is empty.
 */
function owningConsoles(family, stationSystems, systemConsoleFamilies) {
  const owners = [];
  if (stationSystems) {
    for (const stationId of Object.keys(stationSystems).sort()) {
      const systemIds = stationSystems[stationId];
      if ((systemIds || []).some(id => systemConsoleFamilies?.[id] === family)) {
        owners.push(stationId);
      }
    }
  }
  return owners;
}

/**
 * Resolve the unconditional Console Families to actual owning Station ids.
 * The driver uses this after Welcome so an arbitrarily named Station owning
 * Captain Systems keeps the same immediate refresh behavior as `captain`.
 */
export function alwaysPushConsoles(stationSystems, systemConsoleFamilies) {
  const consoles = new Set();
  for (const family of ALWAYS_PUSH_FAMILIES) {
    for (const name of owningConsoles(family, stationSystems, systemConsoleFamilies)) {
      consoles.add(name);
    }
  }
  return consoles;
}

/**
 * Console names dirtied by an inbound ServerMessage and its merged reducer
 * result. Blackboard routing consumes only semantic changed keys/domains; the
 * original BlackboardUpdate payload is never inspected here.
 *
 * A seeking system's blackboard dirties EVERY station, not just the station
 * that authors it (issue #984). This is the one deliberate cross-read edge the
 * module note above warns about, and it is deliberate because the seek is a
 * ship-wide fact: which console shows Comms can change without anything the
 * hosting station owns having changed, and the console that LOSES it has no
 * event of its own to learn that from. The cost is bounded by the driver, which
 * pushes only consoles that are active (`client.html`): a phone renders one
 * console, so the fan-out is at most one extra push of the console the player
 * is actually looking at — which is the console that has to be right.
 *
 * @param {{ type?: string, data?: object }} msg  decoded ServerMessage
 * @param {{ changedDomains?: Set<string>, changedSystems?: Set<string>,
 *           changedBlackboards?: Set<string> }} changes merged reducer result
 * @param {Object<string, string[]>|null|undefined} stationSystems
 *   simState.stationSystems (station id → owned fine system ids); may be
 *   missing before Welcome.
 * @param {Object<string, string>|null|undefined} systemConsoleFamilies
 *   authoritative System id → Console Family projection.
 * @param {Object<string, string>|null|undefined} blackboardConsoleFamilies
 *   authoritative reserved/aggregate blackboard key → Console Family
 *   projection; these keys are not Systems.
 * @returns {Set<string>} console names to push via pushConsoleStateFor
 */
export function dirtyConsolesFor(
  msg,
  changes,
  stationSystems,
  systemConsoleFamilies,
  blackboardConsoleFamilies,
) {
  const dirty = new Set();
  for (const id of changes?.changedBlackboards || []) {
    for (const family of familiesForBlackboardId(
      id,
      systemConsoleFamilies,
      blackboardConsoleFamilies,
    )) {
      for (const name of owningConsoles(family, stationSystems, systemConsoleFamilies)) {
        dirty.add(name);
      }
    }
  }
  if (changes?.changedDomains?.has(CHANGE_DOMAINS.STATION_HOSTING)) {
    for (const name of Object.keys(stationSystems || {}).sort()) dirty.add(name);
  }
  for (const family of STATIC_MESSAGE_FAMILIES[msg?.type] || []) {
    for (const name of owningConsoles(family, stationSystems, systemConsoleFamilies)) {
      dirty.add(name);
    }
  }
  return dirty;
}

// Expose for the non-module inline script in client.html (same pattern as
// gui/console-state.js / gui/mount-plan.js).
if (typeof window !== 'undefined') {
  window.dirtyConsolesFor = dirtyConsolesFor;
  window.alwaysPushConsoles = alwaysPushConsoles;
}
