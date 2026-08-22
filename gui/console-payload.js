/**
 * gui/console-payload.js — Shared helpers for reading a keyed console payload
 * (issue #1231, T4.C0 of the console-seam programme; see
 * `pasm/spec/` architecture-deepening context).
 *
 * A `SystemStationConsolePayload` (built by `buildSystemStationConsoleState`
 * in `gui/console-state.js`, post issue #825) carries `systems`, an object
 * keyed by fine system id (`'power-reactor'`, `'shields-system'`, `'repair'`,
 * ...). A panel usually wants "whichever of these ids this station actually
 * owns" — a cruiser's engineering station has no `shields-system`, a
 * destroyer's does — so it asks for a short candidate list, in preference
 * order, and takes the first one present.
 *
 * That exact helper — `function system(s, ...ids) { ... }` — was copy-pasted
 * verbatim into 9 console HTML documents under gui/ (one per hull's station,
 * e.g. gui/cruiser/engineering.html) rather than imported, because those
 * documents predate this module. This file gives it one
 * definition and one export so a later phase can swap each copy for an
 * import (see `systemView` below) without changing behaviour.
 */

/**
 * Resolve the first present system view among `ids`, or `{}` if the payload
 * carries none of them.
 *
 * Matches the semantics of the `function system(s, ...ids)` helper as
 * authored in gui/cruiser/engineering.html and its 8 siblings exactly:
 * a missing `s.systems` object, a missing key, and a present-but-falsy value
 * (`undefined`, `null`, `0`, `''`) are all treated as "not present" and fall
 * through to the next candidate id.
 *
 * @param {{systems?: Object<string, object>}} s   a SystemStationConsolePayload
 * @param {...string} ids                          candidate system ids, in preference order
 * @returns {object} the first present `s.systems[id]`, else `{}`
 */
export function systemView(s, ...ids) {
  for (const id of ids) {
    if (s.systems && s.systems[id]) return s.systems[id];
  }
  return {};
}

/**
 * Normalise a console payload to the keyed shape (issue #1233, T4.C1.5 of the
 * console-seam programme).
 *
 * `buildConsoleStateInner` (gui/console-state.js) emits a FLAT plain-builder
 * payload for a single-family station (fields read straight off `s`, e.g.
 * `s.red_alert`) and a system-id-KEYED `SystemStationConsolePayload` (fields
 * read via `systemView`/`system(s, ...ids)`) for a multi-family station. A
 * console written against the keyed accessor gets nothing back — `systemView`
 * falls through every candidate id to `{}` — when it is fed a flat payload
 * directly: the exact #925 defect (wrong shape in ⇒ silently blank console).
 *
 * `gui/console-core.js` calls this on every inbound payload before handing it
 * to a console's `render`, so a console reading through `systemView` never has
 * to know whether the wire payload arrived flat or keyed:
 *
 *  - A payload that already carries a `.systems` object is a genuine
 *    multi-family `SystemStationConsolePayload` (or one this function already
 *    normalised) and is returned unchanged — its `systems` is keyed by fine
 *    system id, exactly as `systemView` expects.
 *  - A flat payload (no `.systems`) is wrapped: the SAME object is nested
 *    under EACH fine system id its `family` owns (`FAMILY_SYSTEM_IDS`),
 *    alongside the original top-level fields (so any reader that still expects
 *    the flat shape directly — every flat console today — keeps working
 *    unchanged). `family` is the console-family name the payload's fields
 *    belong to (`'captain'`, `'helm'`, `'tactical'`, …), the one thing a flat
 *    console's own HTML always knows about itself.
 *  - A flat payload with no known `family` (a console with no family concept,
 *    e.g. the Command console) is returned unchanged — there is nothing to
 *    key it under.
 *
 * Keying by FINE system id — not by the family NAME — is the correctness fix
 * of issue #1233's review: `buildSystemStationConsoleState` keys a KEYED
 * payload's `systems` by fine id (`'power-reactor'`, `'shields-system'`,
 * `'helm-thrust'`, `'tactical-radar'`, …), and every shipped reader asks for
 * those fine ids via `systemView(s, '<fine-id>', …)` — NEVER the family name.
 * Wrapping a flat payload under the family name (`systems['power']`) left it
 * invisible to `systemView(s, 'power-reactor', …)` for the four families whose
 * fine id differs from the family name (power, shields, helm, tactical): the
 * exact #925 blank-console defect this seam exists to kill. Keying under the
 * fine ids instead makes `systemView` resolve a flat payload identically to a
 * keyed one, for every family.
 *
 * @param {object} s        parsed console payload (flat or keyed)
 * @param {string} [family] this console's console-family name; omit for a
 *                          console with no single-family concept, or one that
 *                          already reads a genuinely keyed payload
 * @returns {object} `s` unchanged, or a shallow copy of `s` whose `systems`
 *   maps each of `family`'s fine system ids to (the unmodified) `s`
 */
export function normalizeConsolePayload(s, family) {
  if (!s || typeof s !== 'object') return s;
  if (s.systems) return s; // already keyed — system ids are the ground truth
  if (!family) return s;   // no family to key a flat payload under
  // Key the flat view under the FINE system ids this family's readers query
  // (FAMILY_SYSTEM_IDS), mirroring how `buildSystemStationConsoleState` keys a
  // keyed payload — so `systemView(s, '<fine-id>', …)` resolves a flat payload
  // exactly as it resolves a keyed one. An unknown family falls back to keying
  // under its own name: never worse than the pre-fix behaviour.
  const ids = FAMILY_SYSTEM_IDS[family] || [family];
  const systems = {};
  for (const id of ids) systems[id] = s;
  const keyed = { ...s };
  keyed.systems = systems;
  return keyed;
}

/**
 * Console family → the fine system ids that family's console readers pass to
 * `systemView`. The inverse of `consoleForSystemId` (gui/console-state.js),
 * restricted to the concrete ids shipped consoles actually query as candidates
 * — the "real reader contract" a normalised FLAT payload must satisfy so that
 * `systemView` resolves it identically to a keyed `SystemStationConsolePayload`
 * (issue #1233).
 *
 * A family whose fine id IS its name (`captain`, `sensors`, `navigation`,
 * `comms`, `repair`, `tractor`, `umbilical`) still lists that name — as a fine
 * id, not a family alias — so nothing that resolved `systems[<name>]` for those
 * regresses. The four families whose fine id differs from the name (power,
 * shields, helm, tactical) are exactly the ones the family-name wrap broke.
 *
 * Prefix families (helm-*, shield-arc-*, phaser-*, blaster-*) are open-ended in
 * the TOML, so this lists the concrete instances shipped readers name — add a
 * new candidate here when a reader starts querying a new fine id.
 */
export const FAMILY_SYSTEM_IDS = Object.freeze({
  captain:    Object.freeze(['captain', 'viewscreen', 'red-alert']),
  helm:       Object.freeze(['helm-thrust', 'helm-joystick', 'helm-steering']),
  tactical:   Object.freeze(['tactical-radar', 'phaser-control', 'phaser-omni', 'blaster-fore', 'blaster-port', 'blaster-starboard']),
  sensors:    Object.freeze(['sensors', 'sensor-radar']),
  navigation: Object.freeze(['navigation']),
  comms:      Object.freeze(['comms']),
  shields:    Object.freeze(['shields-system', 'shield-arc-fore', 'shield-arc-aft']),
  power:      Object.freeze(['power-reactor', 'power-battery']),
  repair:     Object.freeze(['repair']),
  tractor:    Object.freeze(['tractor']),
  umbilical:  Object.freeze(['umbilical']),
});
