/** Identity-free boot ratings, frozen with the fleet before any ship spawns. */
export const MAX_FLEET_CREW_SEATS = 128;
export const MAX_FLEET_CREW_FIELD_BYTES = 128;
const encoder = new TextEncoder();

/** Match Rust's UTF-8 lexical order, including non-ASCII authored identifiers. */
function compareIds(left, right) {
  const a = encoder.encode(left), b = encoder.encode(right);
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

/** Omitted historical data is empty; malformed present data is refused. */
export function canonicalStationRatings(value = []) {
  if (!Array.isArray(value) || value.length > MAX_FLEET_CREW_SEATS) return null;
  const seen = new Set(), result = [];
  const valid = text => typeof text === 'string' && text.length > 0
    && !/[\u0000-\u001f\u007f-\u009f\ud800-\udfff]/u.test(text)
    && encoder.encode(text).length <= MAX_FLEET_CREW_FIELD_BYTES;
  for (const pair of value) {
    if (!Array.isArray(pair) || pair.length !== 2 || !pair.every(valid) || seen.has(pair[0])) return null;
    seen.add(pair[0]);
    result.push([pair[0], pair[1]]);
  }
  return result.sort((a, b) => compareIds(a[0], b[0]));
}
