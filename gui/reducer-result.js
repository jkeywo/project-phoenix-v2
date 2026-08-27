/**
 * gui/reducer-result.js — mergeable semantic output from client state reducers.
 *
 * Reducers own the interpretation of ServerMessage payloads.  Downstream
 * presentation code receives only the domains and authoritative keys that the
 * reducers actually changed, so it never needs to decode the payload again.
 * Sets preserve first-seen order while naturally coalescing the same key
 * reported by more than one reducer.
 */

/** Semantic domains that are broader than one System or blackboard key. */
export const CHANGE_DOMAINS = Object.freeze({
  // Authoritative session/bootstrap state changed. Console mounting happens
  // later in the shell; the only established immediate iframe consumer is
  // the pre-seeded Repair projection.
  WELCOME: 'welcome',
  // The periodic simulation snapshot drives the radar-bearing consoles.
  SIMULATION_SNAPSHOT: 'simulation-snapshot',
  // Entity additions/world replacement and removals deliberately retain their
  // different historical fan-out (Navigation only refreshed on additions).
  WORLD_ENTITY_ADDED: 'world-entity-added',
  WORLD_ENTITY_REMOVED: 'world-entity-removed',
  WEAPONS: 'weapons',
  REPAIR: 'repair',
  POWER: 'power',
  SHIELDS: 'shields',
  COMMS: 'comms',
  STATION_RATINGS: 'station-ratings',
  // A human-seeking System changed host, so both the gaining and losing
  // Station surfaces may need rebuilding.
  STATION_HOSTING: 'station-hosting',
  // These state domains currently have no iframe fan-out of their own. They
  // remain explicit so another reducer-result consumer can react without
  // reconstructing message meaning from the original wire variant.
  ROUND: 'round',
  PHASER_ACTIVITY: 'phaser-activity',
  TORPEDO_ACTIVITY: 'torpedo-activity',
  MODIFIERS: 'modifiers',
  COORDINATION: 'coordination',
  OBJECTIVES: 'objectives',
  SHIP_MANUAL: 'ship-manual',
  DEBUG_STATE: 'debug-state',
  LOBBY: 'lobby',
});

/** A fresh, empty result. Reducers may add to any set they own. */
export function emptyReducerResult() {
  return {
    changedDomains: new Set(),
    changedSystems: new Set(),
    changedBlackboards: new Set(),
  };
}

/**
 * Merge reducer outputs. `null`/`undefined` remain accepted so the browser's
 * brief module-loading race degrades to an empty result instead of throwing.
 */
export function mergeReducerResults(...results) {
  const merged = emptyReducerResult();
  for (const result of results) {
    if (!result) continue;
    for (const domain of result.changedDomains || []) merged.changedDomains.add(domain);
    for (const system of result.changedSystems || []) merged.changedSystems.add(system);
    for (const key of result.changedBlackboards || []) merged.changedBlackboards.add(key);
  }
  return merged;
}

// Expose the merger to client.html's non-module DOM shell. The reducers and
// dirty router import this module too, so no parallel implementation lives in
// the inline script.
if (typeof window !== 'undefined') {
  window.mergeReducerResults = mergeReducerResults;
}
