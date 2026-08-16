/**
 * gui/console-families.js — machine-readable console → { families, shape } map
 * (issue #925).
 *
 * Two independent dimensions decide whether a hull can reuse a console for one
 * of its seats, and BOTH must match or the seat mounts a working-but-wrong or
 * silently-blank iframe:
 *
 *  1. **Covered families.** A console renders exactly the console *families* (the
 *     buckets `consoleForSystemId` in gui/console-state.js sorts fine systems
 *     into) that its HTML embeds panels for. A seat whose owned systems span a
 *     family the console does not render would drop that family's state.
 *
 *  2. **Payload shape.** `buildConsoleStateInner` (gui/console-state.js) emits a
 *     FLAT plain-builder payload for a single-family seat (fields read as
 *     `s.banks`, `s.red_alert`, `s.battery_charge`, …) and a system-id-KEYED
 *     payload (`s.systems['power-reactor']`, read via a `system(s, …)` helper)
 *     for a multi-family seat. A console is written to consume exactly one shape.
 *     A single-family seat pointed at a keyed console (or vice versa) reads the
 *     wrong shape and renders blank — the exact defect #925's first pass shipped.
 *
 * So: a single-family seat must point at a `shape: 'flat'` console covering its
 * one family; a multi-family seat must point at a `shape: 'keyed'` console
 * covering all its families. The covered set and shape for each console are the
 * ground truth of that HTML file (verified by reading it — pinned by the player
 * hull that authored the console on a seat of that family-count and coverage).
 *
 * Enforced by tests/client/npc-hull-console-coverage.test.js, which fails loudly
 * on either a family gap or a shape mismatch (issue #925, AC4).
 *
 * VISITING PANELS ARE NOT COVERAGE (issue #984). Every destroyer console now
 * carries a Nav and a Comms overlay for the human seek to park a system in, and
 * none of them declares those families below. This map answers "what may this
 * seat OWN", and a visitor is by definition not owned: it arrives for as long
 * as the seek holds it there, under `systems[<id>]` on whatever shape the
 * console already has, and leaves again. Claiming the coverage would say a hull
 * may AUTHOR comms on Helm — which would flip Helm's seat to two families and
 * so to the keyed payload its flat HTML cannot read, the exact defect this map
 * exists to prevent.
 */

/**
 * @typedef {{ families: string[], shape: 'flat'|'keyed' }} ConsoleSpec
 * @type {Object<string, ConsoleSpec>}
 */
export const CONSOLE_SPECS = Object.freeze({
  // ── Flat single-family consoles (battleship family). Each single-family seat
  //    on the four-seat Harrow hulls reuses one of these; the seat's owned
  //    family drives the flat builder (see FAMILY_BUILDERS in console-state.js),
  //    so the station id need not equal the family name.
  'gui/battleship/captain.html': Object.freeze({ families: Object.freeze(['captain']), shape: 'flat' }),
  'gui/battleship/helm.html': Object.freeze({ families: Object.freeze(['helm']), shape: 'flat' }),
  'gui/battleship/tactical.html': Object.freeze({ families: Object.freeze(['tactical']), shape: 'flat' }),
  'gui/battleship/power.html': Object.freeze({ families: Object.freeze(['power']), shape: 'flat' }),

  // Other verified flat consoles (not currently reused by an NPC hull, kept so
  // the map is authoritative for any future repoint).
  'gui/destroyer/helm.html': Object.freeze({ families: Object.freeze(['helm']), shape: 'flat' }),
  'gui/cruiser/tactical.html': Object.freeze({ families: Object.freeze(['tactical']), shape: 'flat' }),

  // ── Keyed multi-family consoles. The Requiem courier's two seats are each
  //    multi-family and reuse the courier consoles.
  'gui/courier/captain.html': Object.freeze({ families: Object.freeze(['captain', 'navigation', 'comms', 'shields', 'power', 'repair']), shape: 'keyed' }),
  'gui/courier/tactical.html': Object.freeze({ families: Object.freeze(['sensors', 'helm', 'tactical']), shape: 'keyed' }),

  // Other verified keyed consoles (the alliance_destroyer's own multi-family
  // seats — kept authoritative, not reused by NPC hulls).
  'gui/destroyer/captain.html': Object.freeze({ families: Object.freeze(['captain', 'sensors']), shape: 'keyed' }),
  'gui/destroyer/tactical.html': Object.freeze({ families: Object.freeze(['tactical', 'navigation', 'comms']), shape: 'keyed' }),
  'gui/destroyer/engineering.html': Object.freeze({ families: Object.freeze(['shields', 'power', 'repair']), shape: 'keyed' }),
});

/**
 * The console families a console URL renders, or null if the console is unknown
 * to this map (itself a coverage failure — an authored console with no declared
 * spec cannot be checked).
 * @param {string} consoleUrl
 * @returns {string[]|null}
 */
export function familiesForConsole(consoleUrl) {
  const spec = CONSOLE_SPECS[consoleUrl];
  return spec ? spec.families : null;
}

/**
 * The payload shape a console consumes ('flat' | 'keyed'), or null if unknown.
 * @param {string} consoleUrl
 * @returns {'flat'|'keyed'|null}
 */
export function shapeForConsole(consoleUrl) {
  const spec = CONSOLE_SPECS[consoleUrl];
  return spec ? spec.shape : null;
}

if (typeof window !== 'undefined') {
  window.CONSOLE_SPECS = CONSOLE_SPECS;
  window.familiesForConsole = familiesForConsole;
  window.shapeForConsole = shapeForConsole;
}
