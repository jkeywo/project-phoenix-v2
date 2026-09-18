/** What a Workshop draft has done to its imported source, member by member.
 *
 * Deliberately a comparison of BYTES and paths, never of meaning. A member that
 * was not touched must come back `unchanged` even if a serializer would have
 * written it differently — that is the whole promise of exact-source authoring,
 * and a diff that normalised before comparing would quietly report edits nobody
 * made.
 *
 * A rename is recognised, not inferred from similarity: a removed member and an
 * added member carrying byte-identical content are the same member under a new
 * name. Two files that merely happen to match are reported as one rename and
 * nothing else, which is what actually happened to the workspace.
 */

const equalBytes = (a, b) => {
  if (a === b) return true;
  if (typeof a === 'string' || typeof b === 'string') return false;
  if (a instanceof Uint8Array && b instanceof Uint8Array) {
    return a.length === b.length && a.every((byte, index) => byte === b[index]);
  }
  // Native asset references: identity is the stored asset, not its bytes, which
  // this surface is never given.
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    return a.asset === b.asset && a.length === b.length;
  }
  return false;
};

/** A stable key for matching a removal to an addition. */
const identity = value => {
  if (typeof value === 'string') return `text:${value}`;
  if (value instanceof Uint8Array) return `bytes:${value.length}:${value.join(',')}`;
  if (value && typeof value === 'object') return `asset:${value.asset}:${value.length}`;
  return null;
};

/**
 * Compare two member maps.
 *
 * `from` is what the draft was imported or last exported as; `to` is what it
 * holds now. Both are `Map<path, string | Uint8Array | assetRef>` or plain
 * objects of the same shape.
 */
export function workspaceDiff(from, to) {
  const before = from instanceof Map ? from : new Map(Object.entries(from || {}));
  const after = to instanceof Map ? to : new Map(Object.entries(to || {}));
  const added = [];
  const removed = [];
  const modified = [];
  const unchanged = [];
  for (const [path, value] of after) {
    if (!before.has(path)) added.push(path);
    else if (equalBytes(before.get(path), value)) unchanged.push(path);
    else modified.push(path);
  }
  for (const path of before.keys()) if (!after.has(path)) removed.push(path);

  // Pair each removal with an addition carrying the same content. Sorted so the
  // pairing is deterministic rather than dependent on map insertion order.
  const renamed = [];
  const candidates = new Map();
  for (const path of added.sort()) {
    const key = identity(after.get(path));
    if (key === null) continue;
    if (!candidates.has(key)) candidates.set(key, []);
    candidates.get(key).push(path);
  }
  for (const path of removed.sort()) {
    const key = identity(before.get(path));
    const match = key === null ? undefined : candidates.get(key)?.shift();
    if (match === undefined) continue;
    renamed.push({ from: path, to: match });
  }
  const renamedFrom = new Set(renamed.map(entry => entry.from));
  const renamedTo = new Set(renamed.map(entry => entry.to));

  return {
    added: added.filter(path => !renamedTo.has(path)).sort(),
    removed: removed.filter(path => !renamedFrom.has(path)).sort(),
    modified: modified.sort(),
    renamed: renamed.sort((a, b) => a.from.localeCompare(b.from)),
    unchanged: unchanged.sort(),
  };
}

/** Does this diff report any change at all? */
export function workspaceDiffIsEmpty(diff) {
  return !diff.added.length && !diff.removed.length
    && !diff.modified.length && !diff.renamed.length;
}
