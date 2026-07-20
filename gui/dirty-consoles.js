/**
 * gui/dirty-consoles.js — declarative message → dirty-console mapping (#823).
 *
 * Replaces the hand-maintained fan-out in client.html handleMessage(): given
 * an inbound ServerMessage, `dirtyConsolesFor` returns the set of console
 * (station) names whose iframes should be re-pushed. The mapping is data:
 *
 *  - STATIC_MESSAGE_CONSOLES covers every non-blackboard message type with a
 *    fixed console list (the exact fan-out handleMessage used to hardcode).
 *  - BlackboardUpdate dirtiness is derived from the server-supplied
 *    station→systems ownership (simState.stationSystems): an update for
 *    system X dirties the console of whichever station owns X. Fine system
 *    ids resolve through consoleForSystemId (the same single-source-of-truth
 *    matcher buildSystemStationConsoleState uses); a coarse blackboard id
 *    that directly names a console family ('helm', 'tactical', …) dirties
 *    that family's owning station too.
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

import { consoleForSystemId } from './console-state.js';

/**
 * Non-blackboard message type → console names dirtied. Exact port of the old
 * per-case push*ConsoleState calls in handleMessage.
 */
export const STATIC_MESSAGE_CONSOLES = Object.freeze({
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
  // stationRatings is already fresh when this fires (sim-state applied the
  // message first); controlSources lags until the next SimState tick, so the
  // captain badge is pushed immediately from here.
  RatingChanged: Object.freeze(['captain']),
});

/**
 * Console names whose pushes are NOT gated on being the active console.
 * The captain push was always unconditional (BlackboardUpdate + RatingChanged
 * pushed it regardless of which console is on screen) — keep that as data
 * rather than an inline special case in the driver.
 */
export const ALWAYS_PUSH = Object.freeze(new Set(['captain']));

/**
 * Coarse blackboard ids that directly name a console family. Today's server
 * blackboard ids are coarse (helm / captain / viewscreen / tactical / power /
 * shields / repair / comms / sensors / navigation); the ones not already
 * resolved by consoleForSystemId land here via identity.
 */
const CONSOLE_FAMILIES = Object.freeze(new Set([
  'captain', 'helm', 'tactical', 'sensors', 'navigation',
  'comms', 'shields', 'power', 'repair',
]));

/**
 * The console family a blackboard system id belongs to, or null.
 * Fine ids resolve through the shared matcher; coarse ids that equal a
 * console family name resolve by identity.
 */
function familyForBlackboardId(id) {
  if (typeof id !== 'string') return null;
  const fine = consoleForSystemId(id);
  if (fine) return fine;
  return CONSOLE_FAMILIES.has(id) ? id : null;
}

/**
 * Console families a single blackboard update fans out to: its own family,
 * plus — for captain/viewscreen updates — the currentView cascade.
 */
function familiesForBlackboardId(id) {
  const family = familyForBlackboardId(id);
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
 * ids; station id == console name). Falls back to the family name itself
 * when ownership is unknown (boot race before Welcome) or no station owns
 * the family — pushes to unmounted consoles are harmless no-ops.
 */
function owningConsoles(family, stationSystems) {
  const owners = [];
  if (stationSystems) {
    for (const [stationId, systemIds] of Object.entries(stationSystems)) {
      if ((systemIds || []).some(id => consoleForSystemId(id) === family)) {
        owners.push(stationId);
      }
    }
  }
  return owners.length > 0 ? owners : [family];
}

/**
 * Console names dirtied by an inbound ServerMessage.
 *
 * @param {{ type?: string, data?: object }} msg  decoded ServerMessage
 * @param {Object<string, string[]>|null|undefined} stationSystems
 *   simState.stationSystems (station id → owned fine system ids); may be
 *   missing before Welcome.
 * @returns {Set<string>} console names to push via pushConsoleStateFor
 */
export function dirtyConsolesFor(msg, stationSystems) {
  if (!msg || !msg.type) return new Set();
  if (msg.type === 'BlackboardUpdate') {
    const dirty = new Set();
    const updates = (msg.data && msg.data.updates) || [];
    for (const entry of updates) {
      const id = Array.isArray(entry) ? entry[0] : entry;
      for (const family of familiesForBlackboardId(id)) {
        for (const name of owningConsoles(family, stationSystems)) dirty.add(name);
      }
    }
    return dirty;
  }
  return new Set(STATIC_MESSAGE_CONSOLES[msg.type] || []);
}

// Expose for the non-module inline script in client.html (same pattern as
// gui/console-state.js / gui/mount-plan.js).
if (typeof window !== 'undefined') {
  window.dirtyConsolesFor = dirtyConsolesFor;
  window.DIRTY_ALWAYS_PUSH = ALWAYS_PUSH;
}
