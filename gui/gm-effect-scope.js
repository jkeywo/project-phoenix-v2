/**
 * The one browser vocabulary for a directed effect's SCOPE (issue #1311).
 *
 * Rust's `GmDirectEffectScope` narrows one mechanic — whole entity, one
 * Station's authored Systems, or exactly one System — and three browser
 * surfaces have to agree about it: the panel that composes a press, the
 * attributed result rows that come back on `gm_entity`, and the activity feed
 * that renders the same fact for every GM. This module is that agreement, for
 * `gui/gm-action-reasons.js`'s reason: three parsers for one enum is three
 * chances to disagree about what a GM just did.
 *
 * # Two shapes, deliberately
 *
 * The scope crosses the wire INBOUND (a projection or feed row Rust
 * serialised) as serde's external tagging — `{"station": "helm"}`,
 * `{"system": "impulse-drive"}`, or absent for the whole entity — and OUTBOUND
 * (a request the browser composes) as the flat `{scope, scope_id}` pair
 * `decode_gm_action_request` accepts. The asymmetry is the ingress boundary's:
 * a flat pair has exactly one spelling a browser can send, where a nested
 * object would give a client a second place to put a field nobody validated.
 * Both spellings are produced here so no surface has to remember which is
 * which.
 *
 * Internally a scope is `null` for the whole entity — what every pre-#1311 row
 * meant — or `{ kind: 'station' | 'system', id }`.
 */

export const GM_EFFECT_SCOPE_KINDS = Object.freeze(['entity', 'station', 'system']);

/** The select-option value for a scope, and the key a picker stores it under. */
export function gmEffectScopeKey(scope) {
  return scope ? `${scope.kind}:${scope.id}` : 'entity';
}

/**
 * Read back a value produced by {@link gmEffectScopeKey}.
 *
 * Returns `null` for the whole entity and `undefined` for anything this build
 * does not implement, so a caller can tell "no narrowing" from "not a scope".
 */
export function gmEffectScopeFromKey(key) {
  if (key === 'entity' || key == null || key === '') return null;
  if (typeof key !== 'string') return undefined;
  const split = key.indexOf(':');
  if (split <= 0) return undefined;
  const kind = key.slice(0, split);
  const id = key.slice(split + 1);
  if ((kind !== 'station' && kind !== 'system') || id.length === 0) return undefined;
  return { kind, id };
}

/**
 * Parse the `scope` field of an inbound result or activity row.
 *
 * Absent (or the explicit `"entity"` spelling) is `null`: the whole hull, which
 * is what every pre-#1311 fact meant. A malformed value is `undefined` and its
 * caller rejects the row rather than quietly widening a Station hit into a
 * whole-ship one.
 */
export function parseGmEffectScope(value) {
  if (value == null || value === 'entity') return null;
  if (typeof value !== 'object' || Array.isArray(value)) return undefined;
  const keys = Object.keys(value);
  if (keys.length !== 1) return undefined;
  const [kind] = keys;
  if (kind !== 'station' && kind !== 'system') return undefined;
  const id = value[kind];
  return typeof id === 'string' && id.length > 0 ? { kind, id } : undefined;
}

/** The flat `{scope, scope_id}` pair the typed ingress accepts. */
export function gmEffectScopeRequestFields(scope) {
  return scope
    ? { scope: scope.kind, scope_id: scope.id }
    : { scope: 'entity', scope_id: null };
}

/**
 * One localised sentence naming a scope, given the display name to use for it.
 *
 * `name` is the authored display name the projection published for that
 * Station or System — never an id the browser invented — so the picker reads
 * the way the ship was authored. Falls back to the id when nothing named it,
 * which keeps the diagnostic identity visible rather than rendering a blank.
 */
export function gmEffectScopeLabel(t, scope, name) {
  if (!scope) return t('server.gm.effect.scope_entity');
  return t(`server.gm.effect.scope_${scope.kind}`, { name: name || scope.id });
}

/**
 * The Stations and Systems a projected entity offers as scopes, in hull order.
 *
 * Hull order, not a name sort and not a map order: it is the order the weighted
 * distribution consumes and the order the ship was authored in, so the picker
 * and the mechanic agree about what "first" means. A Station appears once, at
 * the position of the first System it owns; a System the config assigns to no
 * Station contributes no Station row but is still offered on its own.
 */
export function gmEffectScopeOptions(entity) {
  const systems = entity && entity.status && Array.isArray(entity.status.systems)
    ? entity.status.systems : [];
  const stations = [];
  const seen = new Set();
  const systemOptions = [];
  for (const row of systems) {
    // A row whose totals will not parse is offered by nobody: the picker and
    // {@link gmEffectScopeTotals} read the same rows by the same rule, so an
    // option can never name a scope the panel would then have to disable.
    if (!row || typeof row.system_id !== 'string' || row.system_id.length === 0
        || !Number.isSafeInteger(row.current_milli_hp)
        || !Number.isSafeInteger(row.max_milli_hp)) continue;
    const name = typeof row.name === 'string' && row.name.length > 0 ? row.name : row.system_id;
    systemOptions.push({ scope: { kind: 'system', id: row.system_id }, name });
    const station = typeof row.station_id === 'string' && row.station_id.length > 0
      ? row.station_id : null;
    if (station && !seen.has(station)) {
      seen.add(station);
      // The Station's authored display name, by the same rule the System name
      // one line above follows: both halves of the one `<select>` then read as
      // the ship was authored, and the id shows through only when the
      // projection named nothing.
      const stationName = typeof row.station_name === 'string' && row.station_name.length > 0
        ? row.station_name : station;
      stations.push({ scope: { kind: 'station', id: station }, name: stationName });
    }
  }
  return [{ scope: null, name: null }, ...stations, ...systemOptions];
}

/**
 * The `(current, max)` milli-HP a scope covers on a projected entity.
 *
 * `null` when the entity carries no hull totals at all, or when the scope names
 * nothing this entity's projection published — which is the browser's advance
 * warning of the `unknown-station` / `unknown-system` / `target-not-damageable`
 * refusal the authoritative reducer will settle on. The preview is a preview:
 * the projection can be stale, and the tick this resolves at has not happened
 * yet either.
 */
export function gmEffectScopeTotals(entity, scope) {
  const status = entity && entity.status;
  if (!status) return null;
  if (!scope) {
    return Number.isSafeInteger(status.hull_current_milli_hp)
      && Number.isSafeInteger(status.hull_max_milli_hp)
      ? { current: status.hull_current_milli_hp, max: status.hull_max_milli_hp }
      : null;
  }
  const systems = Array.isArray(status.systems) ? status.systems : [];
  let current = 0;
  let max = 0;
  let matched = false;
  for (const row of systems) {
    if (!row || !Number.isSafeInteger(row.current_milli_hp)
        || !Number.isSafeInteger(row.max_milli_hp)) continue;
    const hit = scope.kind === 'system'
      ? row.system_id === scope.id
      : row.station_id === scope.id;
    if (!hit) continue;
    matched = true;
    current += row.current_milli_hp;
    max += row.max_milli_hp;
  }
  return matched ? { current, max } : null;
}
