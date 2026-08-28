/**
 * gui/console-payload.js — metadata-driven readers for keyed console payloads.
 *
 * `buildConsoleStateInner` projects the host-owned SystemId → Console Family
 * map into every payload as `system_families`, and preserves the Station's
 * authored SystemId order in `system_ids`. Presentation code selects a view by
 * Console Family through that projection. It never guesses a System's family
 * from an exact id, a prefix, or a client-maintained inverse list.
 */

/**
 * Actual System ids carried by a payload, in deterministic authored order.
 * Keyed visiting systems that are not in the authored list follow in lexical
 * order, so identical client state always selects the same representative
 * when a family contains more than one System.
 *
 * @param {{system_ids?: string[], systems?: Object<string, object>}} payload
 * @returns {string[]}
 */
function orderedSystemIds(payload) {
  const authored = Array.isArray(payload?.system_ids) ? payload.system_ids : [];
  const seen = new Set(authored);
  const visiting = Object.keys(payload?.systems || {})
    .filter(id => !seen.has(id))
    .sort();
  return authored.concat(visiting);
}

/**
 * Resolve the representative view for a Console Family from authoritative
 * projected metadata, or `{}` when this payload contains no System in that
 * family. Multiple Systems in one family intentionally share one aggregate
 * view; the Station's authored System order chooses the representative.
 *
 * @param {{systems?: Object<string, object>, system_ids?: string[],
 *          system_families?: Object<string, string>}} payload
 * @param {string} family
 * @returns {object}
 */
export function familyView(payload, family) {
  if (!payload || typeof payload !== 'object' || typeof family !== 'string') return {};
  const systems = payload.systems || {};
  const families = payload.system_families || {};
  for (const id of orderedSystemIds(payload)) {
    if (families[id] === family && systems[id]) return systems[id];
  }
  return {};
}

/** Resolve the actual representative System id for a Console Family. */
export function familySystemId(payload, family) {
  if (!payload || typeof payload !== 'object' || typeof family !== 'string') return null;
  const systems = payload.systems || {};
  const families = payload.system_families || {};
  return orderedSystemIds(payload)
    .find(id => families[id] === family && systems[id]) || null;
}

/**
 * Normalise a flat single-family payload to the keyed shape consumed by the
 * shared composite renderers. The builder has already selected the family;
 * this seam merely mirrors the flat view under the actual owned System ids
 * supplied in `system_ids`, after verifying them against `system_families`.
 *
 * No client-side family→System list exists. A payload received before Welcome,
 * or one missing its authoritative projection, remains flat rather than
 * inventing ids from the iframe name.
 *
 * @param {object} payload parsed console payload (flat or keyed)
 * @returns {object} `payload` unchanged, or a shallow copy with `.systems`
 */
export function normalizeConsolePayload(payload) {
  if (!payload || typeof payload !== 'object') return payload;
  if (payload.systems) return payload;

  const families = payload.system_families || {};
  const ids = (Array.isArray(payload.system_ids) ? payload.system_ids : [])
    .filter(id => typeof families[id] === 'string' && families[id] !== '');
  if (ids.length === 0) return payload;

  const systems = {};
  for (const id of ids) systems[id] = payload;
  return { ...payload, systems };
}
