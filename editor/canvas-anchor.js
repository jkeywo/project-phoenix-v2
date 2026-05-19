/**
 * canvas-anchor.js — Pure logic for anchor markers on the Scenario Mode canvas.
 *
 * Anchors in the world TOML are stored as:
 *   [anchors]
 *   starbase_alpha = [500.0, 0.0, 0.0]
 *
 * This module converts that flat map into renderable cross-hair marker objects,
 * supports immutable position updates, and resolves entity positions relative
 * to their anchor.
 */

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
 *   1. entity.position ([x, y, z] array) — used as-is
 *   2. entity.anchor — looked up in the anchors array
 *   3. fallback { x: 0, z: 0 }
 *
 * @param {Object}   entity  - Parsed entity object (may have .position or .anchor).
 * @param {Object}   anchors - The `worldState.anchors` map (same shape as TOML).
 * @returns {{ x: number, z: number }}
 */
export function resolveEntityPosition(entity, anchors) {
  if (!entity) return { x: 0, z: 0 };

  if (entity.position && Array.isArray(entity.position) && entity.position.length >= 3) {
    return { x: entity.position[0], z: entity.position[2] };
  }

  if (entity.anchor && anchors && typeof anchors === 'object') {
    const pos = anchors[entity.anchor];
    if (Array.isArray(pos) && pos.length >= 3) {
      return { x: pos[0], z: pos[2] };
    }
  }

  return { x: 0, z: 0 };
}
