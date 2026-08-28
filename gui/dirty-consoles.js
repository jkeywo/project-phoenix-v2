/**
 * gui/dirty-consoles.js — semantic change → dirty-console routing (#823).
 *
 * Replaces the hand-maintained fan-out in client.html handleMessage(): given
 * merged reducer results, `dirtyConsolesFor` returns the set of console
 * (station) names whose iframes should be re-pushed.
 *
 *  - Reducers report semantic domains at the point where they interpret and
 *    mutate state. This module maps those stable domains to their established
 *    presentation consumers; it never enumerates ServerMessage variants.
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

const DOMAIN_FAMILIES = Object.freeze({
  // Push pre-seeded repair teams immediately on (re)connect, before the first
  // RepairState broadcast. Other Welcome consumers mount after this fan-out.
  [CHANGE_DOMAINS.WELCOME]: Object.freeze(['repair']),
  // Radar blips are derived client-side from the 10 Hz simulation snapshot.
  [CHANGE_DOMAINS.SIMULATION_SNAPSHOT]: Object.freeze([
    'tactical', 'repair', 'sensors', 'navigation',
  ]),
  [CHANGE_DOMAINS.WORLD_ENTITY_ADDED]: Object.freeze([
    'tactical', 'helm', 'sensors', 'navigation',
  ]),
  [CHANGE_DOMAINS.WORLD_ENTITY_REMOVED]: Object.freeze([
    'tactical', 'helm', 'sensors',
  ]),
  [CHANGE_DOMAINS.WEAPONS]: Object.freeze(['tactical']),
  [CHANGE_DOMAINS.REPAIR]: Object.freeze(['repair']),
  [CHANGE_DOMAINS.POWER]: Object.freeze(['power']),
  [CHANGE_DOMAINS.SHIELDS]: Object.freeze(['shields']),
  [CHANGE_DOMAINS.COMMS]: Object.freeze(['comms']),
  // stationRatings is already fresh; controlSources follows on the next
  // SimState tick, so the Captain-family badge retains its immediate push.
  [CHANGE_DOMAINS.STATION_RATINGS]: Object.freeze(['captain']),
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
 * Console names dirtied by a merged reducer result. Routing consumes only
 * semantic domains, changed System ids, and changed blackboard keys; the
 * original ServerMessage is never accepted by this interface.
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
  changes,
  stationSystems,
  systemConsoleFamilies,
  blackboardConsoleFamilies,
) {
  const dirty = new Set();
  for (const id of changes?.changedSystems || []) {
    const family = systemConsoleFamilies?.[id];
    if (typeof family !== 'string' || family === '') continue;
    for (const name of owningConsoles(family, stationSystems, systemConsoleFamilies)) {
      dirty.add(name);
    }
  }
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
  for (const domain of changes?.changedDomains || []) {
    for (const family of DOMAIN_FAMILIES[domain] || []) {
      for (const name of owningConsoles(family, stationSystems, systemConsoleFamilies)) {
        dirty.add(name);
      }
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
