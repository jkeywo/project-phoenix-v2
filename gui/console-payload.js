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
