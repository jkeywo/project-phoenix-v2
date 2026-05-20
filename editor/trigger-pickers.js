/**
 * trigger-pickers.js
 *
 * Pure-logic module that provides dropdown data sources for the trigger editor.
 *
 * Entity-name, objective-id, AI-state, modifier-slot, and flag-kind options
 * are derived from the parsed world TOML state and from the action schema.
 *
 * No DOM manipulation is performed here; fully testable in Node.
 */

import { MODIFIER_SLOTS, FLAG_KINDS } from './action-schema.js';

/**
 * Get options for entity-name dropdown (every named entity across open layers).
 *
 * @param {Array<{ path: string, worldState: object }>} layers
 * @returns {Array<{ value: string, label: string, layerPath: string }>}
 *   label includes layer-path suffix when names collide across layers
 */
export function getEntityNameOptions(layers) {
  const nameToLayers = new Map();

  for (const layer of layers) {
    const { path, worldState } = layer;
    if (!worldState || typeof worldState !== 'object') continue;
    if (!Array.isArray(worldState.entity)) continue;

    for (const ent of worldState.entity) {
      if (!ent.name) continue;
      if (!nameToLayers.has(ent.name)) {
        nameToLayers.set(ent.name, []);
      }
      const paths = nameToLayers.get(ent.name);
      if (!paths.includes(path)) {
        paths.push(path);
      }
    }
  }

  const options = [];
  for (const [name, paths] of nameToLayers) {
    const multiple = paths.length > 1;
    for (const layerPath of paths) {
      options.push({
        value: name,
        label: multiple ? `${name} (${layerPath})` : name,
        layerPath,
      });
    }
  }

  return options;
}

function _scanAddObjectiveIds(worldState) {
  const ids = [];

  if (Array.isArray(worldState.trigger)) {
    for (const trigger of worldState.trigger) {
      if (!Array.isArray(trigger.action)) continue;
      for (const action of trigger.action) {
        if (action.type === 'add_objective' && action.id) {
          ids.push(action.id);
        }
      }
    }
  }

  if (Array.isArray(worldState.comms)) {
    for (const comms of worldState.comms) {
      if (!Array.isArray(comms.response)) continue;
      for (const resp of comms.response) {
        const actions = Array.isArray(resp.action) ? resp.action : [];
        for (const action of actions) {
          if (action.type === 'add_objective' && action.id) {
            ids.push(action.id);
          }
        }
      }
    }
  }

  return ids;
}

/**
 * Get options for objective-id dropdown (from add_objective actions in
 * triggers and comms responses in the same world).
 *
 * @param {object} worldState — parsed world TOML object
 * @returns {Array<{ value: string, label: string }>}
 */
export function getObjectiveIdOptions(worldState) {
  const ids = _scanAddObjectiveIds(worldState);
  const seen = new Set();
  const options = [];
  for (const id of ids) {
    if (!seen.has(id)) {
      seen.add(id);
      options.push({ value: id, label: id });
    }
  }
  return options;
}

/**
 * Get options for AI state dropdown (from a target entity's behaviour block).
 *
 * @param {object} worldState — parsed world TOML
 * @param {string} entityName — the entity whose behaviour states to list
 * @returns {Array<{ value: string, label: string }>}
 */
export function getAiStateOptions(worldState, entityName) {
  if (!worldState || typeof worldState !== 'object') return [];
  if (!Array.isArray(worldState.entity)) return [];

  const entity = worldState.entity.find((e) => e.name === entityName);
  if (!entity) return [];

  const behaviour = entity.behaviour;
  if (!behaviour || typeof behaviour !== 'object') return [];
  if (!Array.isArray(behaviour.state)) return [];

  return behaviour.state
    .filter((s) => s.name)
    .map((s) => ({ value: s.name, label: s.name }));
}

/**
 * Get modifier slot options.
 * @returns {Array<{ value: string, label: string }>}
 */
export function getModifierSlotOptions() {
  return MODIFIER_SLOTS.map((slot) => ({ value: slot, label: slot }));
}

/**
 * Get flag kind options.
 * @returns {Array<{ value: string, label: string }>}
 */
export function getFlagKindOptions() {
  return FLAG_KINDS.map((kind) => ({ value: kind, label: kind }));
}
