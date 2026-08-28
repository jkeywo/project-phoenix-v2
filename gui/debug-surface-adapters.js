/**
 * Order and validate one UI module's Debug Surface adapters (#1267).
 *
 * Rust owns identity and stable iteration through the generated order. Callers
 * own presentation metadata (ids, labels, readers, renderers), supplied as
 * `[generatedSurface, adapter]` pairs. Keeping the pairs as an array lets this
 * boundary reject duplicates instead of allowing an object literal to silently
 * overwrite one; projection fails equally loudly for missing and unknown rows.
 */

import { DEBUG_SURFACE_ORDER } from './debug-surfaces.generated.js';

const KNOWN_SURFACES = new Set(DEBUG_SURFACE_ORDER);

/**
 * @param {Array<[string, Object]>} entries
 * @param {string} owner
 * @returns {ReadonlyArray<Readonly<Object & {flag: string}>>}
 */
export function projectDebugSurfaceAdapters(entries, owner) {
  if (!Array.isArray(entries)) {
    throw new TypeError(`${owner}: Debug Surface adapters must be an array`);
  }

  const bySurface = new Map();
  for (const entry of entries) {
    if (!Array.isArray(entry) || entry.length !== 2) {
      throw new TypeError(`${owner}: each Debug Surface adapter must be a pair`);
    }
    const [surface, adapter] = entry;
    if (!KNOWN_SURFACES.has(surface)) {
      throw new Error(`${owner}: unknown Debug Surface adapter ${String(surface)}`);
    }
    if (bySurface.has(surface)) {
      throw new Error(`${owner}: duplicate Debug Surface adapter ${surface}`);
    }
    if (!adapter || typeof adapter !== 'object' || Array.isArray(adapter)) {
      throw new TypeError(`${owner}: adapter for ${surface} must be an object`);
    }
    if (Object.prototype.hasOwnProperty.call(adapter, 'flag')) {
      throw new Error(`${owner}: ${surface} adapter must not author its own flag`);
    }
    bySurface.set(surface, Object.freeze({ ...adapter, flag: surface }));
  }

  const missing = DEBUG_SURFACE_ORDER.filter((surface) => !bySurface.has(surface));
  if (missing.length > 0) {
    throw new Error(`${owner}: missing Debug Surface adapters: ${missing.join(', ')}`);
  }

  return Object.freeze(DEBUG_SURFACE_ORDER.map((surface) => bySurface.get(surface)));
}
