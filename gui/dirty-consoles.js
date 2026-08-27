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
 *    station→systems ownership and authoritative System-id → Console Family
 *    projection. Unmigrated ids alone use the temporary #1251 matcher
 *    fallback; issue #1252 removes it once every descriptor is populated. A
 *    coarse blackboard id that directly names a console family ('helm',
 *    'tactical', …) dirties that family's owning station too.
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
  // Rejection feedback (#761 AC3): re-push the comms console so the attempted
  // response button flashes red.
  CommsResponseRejected: Object.freeze(['comms']),
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
 * TEMPORARY #1251 fallback for unmigrated coarse blackboard ids that directly
 * name a console family. Issue #1252 gives reserved channels authoritative
 * family metadata and removes this inverse-name census with the System matcher.
 */
const CONSOLE_FAMILIES = Object.freeze(new Set([
  'captain', 'helm', 'tactical', 'sensors', 'navigation',
  'comms', 'shields', 'power', 'repair',
]));

/**
 * TEMPORARY #1251 fallback for reserved blackboard CHANNEL keys — ids no
 * `[[system]]` block declares and no station owns — mapped to the console
 * family that renders them. Issue #1252 migrates this table into authoritative
 * metadata and deletes it.
 *
 * `scan` (issue #1032) is the sensor suite's last reading. It is not a system
 * id (the commandable, damageable thing is `sensors`), so neither the fine
 * matcher nor the coarse family set below resolves it, and without this entry a
 * fresh reading would only reach a console that happened to be pushed for some
 * other reason.
 */
const CHANNEL_FAMILIES = Object.freeze({
  scan: 'sensors',
});

/**
 * The console family a blackboard system id belongs to, or null.
 * System ids resolve through authoritative metadata first. Only absent,
 * unmigrated entries reach the temporary matcher/table/identity fallbacks above;
 * issue #1252 removes all three.
 */
function familyForBlackboardId(id, systemConsoleFamilies) {
  if (typeof id !== 'string') return null;
  const fine = consoleForSystemId(id, systemConsoleFamilies);
  if (fine) return fine;
  if (CHANNEL_FAMILIES[id]) return CHANNEL_FAMILIES[id];
  return CONSOLE_FAMILIES.has(id) ? id : null;
}

/**
 * Console families a single blackboard update fans out to: its own family,
 * plus — for captain/viewscreen updates — the currentView cascade.
 */
function familiesForBlackboardId(id, systemConsoleFamilies) {
  const family = familyForBlackboardId(id, systemConsoleFamilies);
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
function owningConsoles(family, stationSystems, systemConsoleFamilies) {
  const owners = [];
  if (stationSystems) {
    for (const [stationId, systemIds] of Object.entries(stationSystems)) {
      if ((systemIds || []).some(id => consoleForSystemId(id, systemConsoleFamilies) === family)) {
        owners.push(stationId);
      }
    }
  }
  return owners.length > 0 ? owners : [family];
}

/**
 * True when a blackboard update entry belongs to a human-seeking system —
 * recognised by the presence of the `host_station` KEY, `null` included (issue
 * #984). The server writes that key unconditionally on a seeking system's
 * blackboard for exactly this test; see `NavigationBlackboard::host_station`.
 *
 * The entry is `[systemId, { kind, data }]` — an adjacently-tagged
 * `SystemBlackboard`, the same envelope gui/sim-state.js unwraps — so the
 * field sits inside `data` rather than on the entry itself.
 *
 * @param {[string, object]|string} entry
 */
function isSeekingBlackboard(entry) {
  if (!Array.isArray(entry)) return false;
  const tagged = entry[1];
  const data = tagged && typeof tagged === 'object' ? tagged.data : null;
  return !!data && typeof data === 'object' && 'host_station' in data;
}

/**
 * Console names dirtied by an inbound ServerMessage.
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
 * @param {Object<string, string[]>|null|undefined} stationSystems
 *   simState.stationSystems (station id → owned fine system ids); may be
 *   missing before Welcome.
 * @param {Object<string, string>|null|undefined} systemConsoleFamilies
 *   authoritative System id → Console Family projection. Missing entries are
 *   the only ids permitted to use #1251's temporary inference fallback.
 * @returns {Set<string>} console names to push via pushConsoleStateFor
 */
export function dirtyConsolesFor(msg, stationSystems, systemConsoleFamilies) {
  if (!msg || !msg.type) return new Set();
  if (msg.type === 'BlackboardUpdate') {
    const dirty = new Set();
    const updates = (msg.data && msg.data.updates) || [];
    for (const entry of updates) {
      const id = Array.isArray(entry) ? entry[0] : entry;
      for (const family of familiesForBlackboardId(id, systemConsoleFamilies)) {
        for (const name of owningConsoles(family, stationSystems, systemConsoleFamilies)) {
          dirty.add(name);
        }
      }
      if (isSeekingBlackboard(entry)) {
        for (const name of Object.keys(stationSystems || {})) dirty.add(name);
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
