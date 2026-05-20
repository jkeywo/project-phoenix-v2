/**
 * anchor-rename.js — Pure module for analysing anchor rename safety.
 *
 * When a user renames an anchor in the [anchors] section, this module:
 *   - Checks the new name doesn't collide with an existing anchor in any layer
 *   - Finds all entities and trigger actions that reference the old name
 *   - Classifies references as in-layer (auto-rewritable) or cross-layer (warning)
 *   - Returns the set of rewrite pairs for layers that own the anchor
 */

/**
 * Analyze renaming an anchor across one or more layers.
 *
 * @param {string} oldName — current anchor name
 * @param {string} newName — desired new name
 * @param {Array<{ path: string, worldState: object }>} layers — all open layers
 *
 * @returns {{
 *   allowed: boolean,
 *   error: string|null,
 *   inLayerReferences: Array<{ layerPath: string, entityName: string, field: string }>,
 *   crossLayerReferences: Array<{ layerPath: string, entityName: string, field: string }>,
 *   rewritePairs: Array<{ layerPath: string, newAnchorValue: string }>
 * }}
 */
export function analyzeAnchorRename(oldName, newName, layers) {
  if (!layers || layers.length === 0) {
    return emptySafe();
  }

  if (oldName === newName) {
    return emptySafe();
  }

  const allAnchorNames = new Map();
  const ownerLayers = new Set();
  const rewritePairs = [];

  for (const layer of layers) {
    const { path, worldState } = layer;
    if (!worldState || !worldState.anchors || typeof worldState.anchors !== 'object') continue;
    for (const name of Object.keys(worldState.anchors)) {
      if (!allAnchorNames.has(name)) {
        allAnchorNames.set(name, path);
      }
      if (name === oldName) {
        ownerLayers.add(path);
        rewritePairs.push({ layerPath: path, newAnchorValue: newName });
      }
    }
  }

  if (allAnchorNames.has(newName)) {
    const existingLayer = allAnchorNames.get(newName);
    return {
      allowed: false,
      error: `Anchor "${newName}" already exists in layer "${existingLayer}". Rename blocked.`,
      inLayerReferences: [],
      crossLayerReferences: [],
      rewritePairs: [],
    };
  }

  const inLayerReferences = [];
  const crossLayerReferences = [];

  for (const layer of layers) {
    const { path, worldState } = layer;
    if (!worldState || typeof worldState !== 'object') continue;

    if (Array.isArray(worldState.entity)) {
      for (const ent of worldState.entity) {
        if (ent && ent.anchor === oldName) {
          const ref = {
            layerPath: path,
            entityName: ent.name || '(unnamed)',
            field: 'anchor',
          };
          if (ownerLayers.has(path)) {
            inLayerReferences.push(ref);
          } else {
            crossLayerReferences.push(ref);
          }
        }
      }
    }

    if (Array.isArray(worldState.trigger)) {
      for (const trigger of worldState.trigger) {
        if (!trigger || !Array.isArray(trigger.action)) continue;
        for (const action of trigger.action) {
          if (action && typeof action === 'object' && action.anchor === oldName) {
            const ref = {
              layerPath: path,
              entityName: trigger.entity || '(trigger)',
              field: 'action.anchor',
            };
            if (ownerLayers.has(path)) {
              inLayerReferences.push(ref);
            } else {
              crossLayerReferences.push(ref);
            }
          }
        }
      }
    }
  }

  return {
    allowed: true,
    error: null,
    inLayerReferences,
    crossLayerReferences,
    rewritePairs,
  };
}

function emptySafe() {
  return {
    allowed: true,
    error: null,
    inLayerReferences: [],
    crossLayerReferences: [],
    rewritePairs: [],
  };
}
