/**
 * anchor-delete.js — Pure module for checking anchor delete safety.
 *
 * Scans ALL open layers for `[[entity]]` spawns that reference a given
 * anchor. Returns the set of blockers or confirms safe deletion.
 *
 * An anchor referenced from a `[script]` body is not tracked (issue #985).
 */

/**
 * Check if an anchor can be deleted safely.
 *
 * @param {string} anchorName — the anchor to delete
 * @param {Array<{ path: string, worldState: object }>} layers — all open layers
 * @param {string} anchorOwnerLayer — the layer where this anchor is defined
 *
 * @returns {{
 *   canDelete: boolean,
 *   blockers: Array<{ layerPath: string, entityName: string|null, type: string }>
 * }}
 *   canDelete: true if no entity in ANY layer references this anchor
 *   blockers: list of referencing entities (empty if canDelete is true)
 */
export function canDeleteAnchor(anchorName, layers, anchorOwnerLayer) {
  const blockers = [];

  if (!layers || layers.length === 0) {
    return { canDelete: true, blockers: [] };
  }

  for (const layer of layers) {
    const { path, worldState } = layer;
    if (!worldState || typeof worldState !== 'object') continue;

    if (Array.isArray(worldState.entity)) {
      for (const ent of worldState.entity) {
        const entAnchor = ent && ent.transform && ent.transform.anchor;
        if (entAnchor === anchorName) {
          blockers.push({
            layerPath: path,
            entityName: ent.name || null,
            type: 'entity',
          });
        }
      }
    }
  }

  return {
    canDelete: blockers.length === 0,
    blockers,
  };
}
