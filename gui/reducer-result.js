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
  // A human-seeking System changed host, so both the gaining and losing
  // Station surfaces may need rebuilding.
  STATION_HOSTING: 'station-hosting',
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
 * Merge reducer outputs without requiring every reducer to migrate at once.
 * `null`/`undefined` are intentionally accepted while #1260 moves the
 * remaining reducers onto this seam.
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
