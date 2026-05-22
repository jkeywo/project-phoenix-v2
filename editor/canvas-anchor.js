/**
 * canvas-anchor.js — Pure logic for anchor markers on the World Mode canvas.
 *
 * Anchors in the world TOML are stored as:
 *   [anchors]
 *   starbase_alpha = [500.0, 0.0, 0.0]
 *
 * This module converts that flat map into renderable cross-hair marker objects,
 * supports immutable position updates, and resolves entity positions relative
 * to their anchor.
 */

import { stringifyWorldToml } from './world-toml.js';

/**
 * Convert the flat anchors map from a parsed world TOML into an array of
 * marker objects suitable for canvas rendering.
 *
 * @param {Object} anchors - The `worldState.anchors` object, e.g.
 *   `{ starbase_alpha: [500.0, 0.0, 0.0], patrol_alpha: [300.0, 0.0, -300.0] }`
 * @returns {{ name: string, x: number, z: number }[]}
 */
export function getAnchorMarkers(anchors) {
  if (!anchors || typeof anchors !== 'object') return [];
  return Object.entries(anchors)
    .filter(([, pos]) => Array.isArray(pos) && pos.length >= 3)
    .map(([name, pos]) => ({ name, x: pos[0], z: pos[2] }));
}

/**
 * Return an array of render specs for drawing cross-hair markers and labels.
 * Each spec is a pure-data object a renderer would use to draw a cross-hair
 * and name label at the anchor's position.
 *
 * @param {Object} anchors - The `worldState.anchors` flat TOML map, e.g.
 *   `{ starbase_alpha: [500.0, 0.0, 0.0] }`
 * @returns {{ name: string, x: number, z: number, size: number }[]}
 */
export function getAnchorRenderSpecs(anchors) {
  if (!anchors || typeof anchors !== 'object') return [];
  return Object.entries(anchors)
    .filter(([, pos]) => Array.isArray(pos) && pos.length >= 3)
    .map(([name, pos]) => ({ name, x: pos[0], z: pos[2], size: 10 }));
}

/**
 * Return a new worldState with the named anchor moved to (newX, newZ).
 * The Y component is preserved (defaulting to 0.0 if absent).
 * All other data is unchanged (immutable update).
 *
 * @param {Object} worldState - Parsed world TOML object.
 * @param {string} anchorName - Key in worldState.anchors to update.
 * @param {number} newX
 * @param {number} newZ
 * @returns {Object} Updated worldState (new object reference).
 */
export function moveAnchor(worldState, anchorName, newX, newZ) {
  if (!worldState || !worldState.anchors) return worldState;
  const existing = worldState.anchors[anchorName];
  const y = Array.isArray(existing) && existing.length >= 2 ? existing[1] : 0.0;
  return {
    ...worldState,
    anchors: {
      ...worldState.anchors,
      [anchorName]: [newX, y, newZ],
    },
  };
}

/**
 * Resolve the canvas position { x, z } of an entity, taking into account
 * an optional anchor reference.
 *
 * Resolution order:
 *   1. entity.transform.position ([x, y, z] array) — used as-is
 *   2. entity.transform.anchor — looked up in the anchors flat TOML map
 *   3. fallback { x: 0, z: 0 }
 *
 * @param {Object} entity   - Parsed entity object (may have .transform with .position or .anchor).
 * @param {Object} anchors  - The `worldState.anchors` flat map `{ name: [x,y,z] }`.
 * @returns {{ x: number, z: number }}
 */
export function resolveEntityPosition(entity, anchors) {
  if (!entity || !entity.transform) return { x: 0, z: 0 };

  const t = entity.transform;

  if (t.position && Array.isArray(t.position) && t.position.length >= 3) {
    return { x: t.position[0], z: t.position[2] };
  }

  if (t.anchor && anchors && typeof anchors === 'object') {
    const pos = anchors[t.anchor];
    if (Array.isArray(pos) && pos.length >= 3) {
      return { x: pos[0], z: pos[2] };
    }
  }

  return { x: 0, z: 0 };
}

/**
 * Serialize a worldState (potentially with updated anchors) to a TOML string.
 * Demonstrates the save path: call this after moveAnchor to persist changes.
 *
 * @param {Object} worldState - Parsed world TOML object (e.g. result of moveAnchor).
 * @returns {string} TOML text ready to write to disk.
 */
export function serializeWorldWithAnchor(worldState) {
  return stringifyWorldToml(worldState);
}
