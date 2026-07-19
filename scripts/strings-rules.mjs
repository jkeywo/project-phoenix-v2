/**
 * scripts/strings-rules.mjs — Which TOML keys hold display text.
 *
 * Shared by extract-strings.mjs (which rewrites) and check-strings.mjs (which
 * enforces). These two must agree exactly: if the checker's idea of
 * "localisable" is wider than the extractor's, CI fails on files the extractor
 * deliberately left alone, so the rules live here rather than in both files.
 */

/** Keys whose values are shown to a player. */
export const LOCALISABLE = new Set([
  'name', 'display_name', 'label', 'description', 'message', 'text', 'title', 'from', 'speaker',
]);

/** Keys that point at an `[[entity]] name` and must track any rename. */
export const REFERENCE_KEYS = new Set(['entity', 'targets']);

/**
 * Is a `name` at this location display text, or an identifier?
 *
 * `name` is the one genuinely ambiguous key: the TOML gives no syntactic clue,
 * and guessing wrong fails silently — a lookup stops matching and whatever it
 * gated quietly stops working.
 *
 * Known lookup-by-name sites, all of which must keep their English:
 *   - `[[station]] name`        → lobby::stations_config::get_station (:66)
 *   - `[[station.rating]] name` → ShipConfig::rating_for_station (:233), which
 *     selects `automated_systems`, so breaking it disables ship automation
 *   - faction `name`            → ai::faction (:71)
 *   - `[[wave]]` / spawn-group and range-band names in combat_test.toml
 *
 * Stations, ratings and factions all carry a real `id`/`uuid` beside the name,
 * so the client localises those by deriving an id (station.<id>.name) instead
 * of putting a string id in the TOML. That keeps Rust's lookups untouched and
 * avoids rewriting ~900 station-name literals across src/.
 *
 * `name` is therefore display text in exactly two places:
 *   - world `[[entity]] name`, where references are rewritten in step with it
 *   - the top-level `name` of an entity template ("Alliance Battleship")
 *
 * @param {string} header current TOML table header ('' at top level)
 * @param {'entity'|'world'|'faction'} prefix which asset group the file is in
 */
export function isLocalisableName(header, prefix) {
  if (prefix === 'faction') return false;
  if (header === 'entity') return true;
  // Top-level `name` is a display name only in an entity template. In a world
  // file the top level belongs to the world itself, which uses `[global] title`.
  return header === '' && prefix === 'entity';
}

/**
 * Should this key/location be localised at all?
 * @param {string} key
 * @param {string} header
 * @param {'entity'|'world'|'faction'} prefix
 */
export function isLocalisable(key, header, prefix) {
  if (!LOCALISABLE.has(key)) return false;
  if (key === 'name') return isLocalisableName(header, prefix);
  return true;
}
