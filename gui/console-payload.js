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
 *    under `systems[family]`, alongside the original top-level fields (so any
 *    reader that still expects the flat shape directly — every console today
 *    — keeps working unchanged). `family` is the console-family name the
 *    payload's fields belong to (`'captain'`, `'helm'`, `'tactical'`, …), the
 *    one thing a flat console's own HTML always knows about itself.
 *  - A flat payload with no known `family` (a console with no family concept,
 *    e.g. the Command console) is returned unchanged — there is nothing to
 *    key it under.
 *
 * @param {object} s        parsed console payload (flat or keyed)
 * @param {string} [family] this console's console-family name; omit for a
 *                          console with no single-family concept, or one that
 *                          already reads a genuinely keyed payload
 * @returns {object} `s` unchanged, or a shallow copy of `s` with a new
 *   `systems[family]` entry pointing at (the unmodified) `s`
 */
export function normalizeConsolePayload(s, family) {
  if (!s || typeof s !== 'object') return s;
  if (s.systems) return s; // already keyed — system ids are the ground truth
  if (!family) return s;   // no family to key a flat payload under
  const keyed = { ...s };
  keyed.systems = { [family]: s };
  return keyed;
}
