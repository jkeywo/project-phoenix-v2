import { parse, stringify } from 'smol-toml';

/**
 * override-editor.js
 *
 * Pure-logic module for editing per-spawn entity overrides in the world editor.
 *
 * An entity instance in a world TOML may carry an inline `[override]` block
 * that deep-merges on top of the template entity config at spawn time.  This
 * module lets the editor:
 *
 *   1. Show a "resolved" view (template + all current overrides deep-merged).
 *   2. Set / clear individual field overrides at arbitrary dotted paths
 *      (e.g. "hull.max" or "radar_appearance.colour").
 *   3. Enumerate all overridden paths as a flat summary.
 *   4. Serialize the override block back to TOML for writing into the world file.
 *
 * No DOM manipulation is performed here; the class is fully testable in Node.
 */

// ── Deep-merge helpers ────────────────────────────────────────────────────────

/**
 * Deep-merge `override` on top of `template`.
 * - Plain objects (records) are merged recursively; keys in `override` win.
 * - All other values (primitives, arrays) are replaced wholesale by `override`.
 * - Keys present only in `template` are preserved.
 * - Keys present only in `override` are added.
 *
 * Neither argument is mutated; a new object is always returned.
 *
 * @param {*} template
 * @param {*} override
 * @returns {*}
 */
export function deepMerge(template, override) {
  if (isPlainObject(template) && isPlainObject(override)) {
    const result = { ...template };
    for (const [key, val] of Object.entries(override)) {
      result[key] = key in result ? deepMerge(result[key], val) : val;
    }
    return result;
  }
  return override;
}

/**
 * Return true if `v` is a plain JS object (record / map), not an array,
 * function, Date, null, etc.
 * @param {*} v
 * @returns {boolean}
 */
function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

// ── Dotted-path helpers ───────────────────────────────────────────────────────

/**
 * Read a value from `obj` at a dotted path like "hull.max".
 * Returns `undefined` if any segment is missing.
 *
 * @param {object} obj
 * @param {string} path  dot-separated key path
 * @returns {*}
 */
function getPath(obj, path) {
  const keys = path.split('.');
  let cur = obj;
  for (const k of keys) {
    if (!isPlainObject(cur) || !(k in cur)) return undefined;
    cur = cur[k];
  }
  return cur;
}

/**
 * Return a new object that is a deep copy of `obj` with the value at the
 * dotted `path` set to `value`.  Intermediate objects are created as needed.
 * Does not mutate `obj`.
 *
 * @param {object} obj
 * @param {string} path
 * @param {*} value
 * @returns {object}
 */
function setPath(obj, path, value) {
  const keys = path.split('.');
  function recurse(cur, depth) {
    const k = keys[depth];
    const rest = isPlainObject(cur) ? { ...cur } : {};
    if (depth === keys.length - 1) {
      rest[k] = value;
    } else {
      rest[k] = recurse(isPlainObject(cur?.[k]) ? cur[k] : {}, depth + 1);
    }
    return rest;
  }
  return recurse(obj, 0);
}

/**
 * Return a new object that is a deep copy of `obj` with the leaf at the
 * dotted `path` removed.  Intermediate objects that become empty after removal
 * are pruned.  Does not mutate `obj`.
 *
 * @param {object} obj
 * @param {string} path
 * @returns {object}
 */
function deletePath(obj, path) {
  const keys = path.split('.');
  function recurse(cur, depth) {
    if (!isPlainObject(cur)) return cur;
    const k = keys[depth];
    if (!(k in cur)) return cur; // path didn't exist — nothing to remove
    if (depth === keys.length - 1) {
      const copy = { ...cur };
      delete copy[k];
      return copy;
    }
    const nested = recurse(cur[k], depth + 1);
    const copy = { ...cur, [k]: nested };
    // Prune empty intermediate objects
    if (isPlainObject(nested) && Object.keys(nested).length === 0) {
      delete copy[k];
    }
    return copy;
  }
  return recurse(obj, 0);
}

// ── Flat path enumeration ─────────────────────────────────────────────────────

/**
 * Enumerate every leaf in `obj` as an array of `{ path, value }` entries
 * where `path` is a dot-separated string.
 *
 * @param {object} obj
 * @param {string} [prefix='']
 * @returns {Array<{path: string, value: *}>}
 */
function flattenPaths(obj, prefix = '') {
  const entries = [];
  for (const [k, v] of Object.entries(obj)) {
    const fullPath = prefix ? `${prefix}.${k}` : k;
    if (isPlainObject(v)) {
      entries.push(...flattenPaths(v, fullPath));
    } else {
      entries.push({ path: fullPath, value: v });
    }
  }
  return entries;
}

// ── Deep-clone helper ─────────────────────────────────────────────────────────

/**
 * Deep-clone any JSON-compatible value.  Used to snapshot the template on
 * construction so callers cannot mutate the editor's internal state via the
 * original reference.
 *
 * @param {*} v
 * @returns {*}
 */
function deepClone(v) {
  if (Array.isArray(v)) return v.map(deepClone);
  if (isPlainObject(v)) {
    const out = {};
    for (const [k, val] of Object.entries(v)) out[k] = deepClone(val);
    return out;
  }
  return v;
}

// ── OverrideEditor ────────────────────────────────────────────────────────────

/**
 * Editor for a single entity-spawn's override block.
 *
 * Usage:
 *   const ed = new OverrideEditor(templateObj);
 *   ed.setOverride('hull.max', 150);
 *   ed.setOverride('radar_appearance.colour', [1.0, 0.0, 0.0]);
 *   const resolved = ed.getResolvedView();   // template merged with overrides
 *   const summary  = ed.getOverridesSummary(); // [{ path, value }, ...]
 *   const toml     = ed.toOverridesToml();     // TOML string for the override block
 *   ed.clearOverride('hull.max');
 */
export class OverrideEditor {
  /**
   * @param {object} template  The base entity config object (already parsed from TOML).
   *                           The constructor takes a shallow snapshot; the original
   *                           is not mutated.
   */
  constructor(template) {
    /** @type {object} Immutable snapshot of the base template. */
    this._template = deepClone(template);

    /**
     * Current overrides as a nested plain object.
     * @type {object}
     */
    this._overrides = {};
  }

  // ── Mutation ──────────────────────────────────────────────────────────────

  /**
   * Set an override at the given dotted path.
   * The path may address fields that do not exist in the template.
   *
   * @param {string} path   e.g. "hull.max" or "radar_appearance.colour"
   * @param {*}      value  Any JSON-compatible value (number, string, boolean, array, object).
   */
  setOverride(path, value) {
    this._overrides = setPath(this._overrides, path, value);
  }

  /**
   * Remove the override at the given dotted path.
   * No-op if the path was not overridden.
   * Intermediate keys that become empty after removal are pruned.
   *
   * @param {string} path
   */
  clearOverride(path) {
    this._overrides = deletePath(this._overrides, path);
  }

  // ── Queries ───────────────────────────────────────────────────────────────

  /**
   * Return the resolved entity config: template deep-merged with all current
   * overrides.  A new object is returned on every call; callers may mutate it
   * freely without affecting internal state.
   *
   * @returns {object}
   */
  getResolvedView() {
    return deepMerge(this._template, this._overrides);
  }

  /**
   * Return a flat list of every path that has been overridden.
   * Each entry is `{ path: string, value: * }`.
   * Array values appear as a single entry with the array as the value.
   *
   * @returns {Array<{path: string, value: *}>}
   */
  getOverridesSummary() {
    return flattenPaths(this._overrides);
  }

  /**
   * Serialise the current override block as TOML.
   * Returns an empty string when there are no overrides.
   *
   * @returns {string}
   */
  toOverridesToml() {
    if (Object.keys(this._overrides).length === 0) return '';
    return stringify(this._overrides);
  }

  // ── Convenience getters ───────────────────────────────────────────────────

  /**
   * Return the raw override object (deep copy).
   * Useful for persisting the override block back to the world TOML.
   *
   * @returns {object}
   */
  getOverrides() {
    return deepClone(this._overrides);
  }

  /**
   * Return the template (deep copy).
   *
   * @returns {object}
   */
  getTemplate() {
    return deepClone(this._template);
  }
}
