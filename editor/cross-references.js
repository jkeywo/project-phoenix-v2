export class CrossReferenceIndex {
  constructor() {
    this._references = new Map();
    this._entityNames = new Map();
    this._anchorNames = new Map();
    this._objectiveIds = new Map();
  }

  indexLayers(layers) {
    this._references.clear();
    this._entityNames.clear();
    this._anchorNames.clear();
    this._objectiveIds.clear();

    for (const layer of layers) {
      const { path, worldState } = layer;
      if (!worldState || typeof worldState !== 'object') continue;

      this._collectEntityNames(worldState, path);
      this._collectAnchorNames(worldState, path);
      this._scanTriggers(worldState, path);
      this._scanComms(worldState, path);
    }
  }

  _collectEntityNames(worldState, path) {
    if (!Array.isArray(worldState.entity)) return;
    for (const ent of worldState.entity) {
      if (ent.name && !this._entityNames.has(ent.name)) {
        this._entityNames.set(ent.name, path);
      }
    }
  }

  _collectAnchorNames(worldState, path) {
    if (!worldState.anchors || typeof worldState.anchors !== 'object') return;
    for (const name of Object.keys(worldState.anchors)) {
      if (!this._anchorNames.has(name)) {
        this._anchorNames.set(name, path);
      }
    }
  }

  _scanTriggers(worldState, path) {
    if (!Array.isArray(worldState.trigger)) return;
    for (const trigger of worldState.trigger) {
      if (trigger.entity) {
        this._addRef(trigger.entity, path, 'trigger',
          `trigger[${trigger.condition || '*'}] entity`);
      }
      if (Array.isArray(trigger.action)) {
        for (const action of trigger.action) {
          if (action.target_entity) {
            this._addRef(action.target_entity, path, 'action',
              `trigger.action target_entity`);
          }
          // Some action schemas use `entity` instead of `target_entity`
          // (e.g. set_ai_state, apply_modifier).  See action-schema.js.
          if (action.entity) {
            this._addRef(action.entity, path, 'action',
              `trigger.action entity`);
          }
          if (action.type === 'add_objective' && action.id) {
            if (!this._objectiveIds.has(action.id)) {
              this._objectiveIds.set(action.id, path);
            }
          }
        }
      }
    }
  }

  _scanComms(worldState, path) {
    if (!Array.isArray(worldState.comms)) return;
    for (const comms of worldState.comms) {
      if (comms.entity) {
        this._addRef(comms.entity, path, 'comms', `comms.entity`);
      }
      if (comms.from) {
        this._addRef(comms.from, path, 'comms', `comms.from`);
      }
      if (Array.isArray(comms.response)) {
        for (const resp of comms.response) {
          const actions = Array.isArray(resp.action) ? resp.action : [];
          for (const action of actions) {
            if (action.target_entity) {
              this._addRef(action.target_entity, path, 'action',
                `comms.response.action target_entity`);
            }
            if (action.entity) {
              this._addRef(action.entity, path, 'action',
                `comms.response.action entity`);
            }
            if (action.type === 'add_objective' && action.id) {
              if (!this._objectiveIds.has(action.id)) {
                this._objectiveIds.set(action.id, path);
              }
            }
          }
        }
      }
    }
  }

  _addRef(targetName, layerPath, type, context) {
    if (!this._references.has(targetName)) {
      this._references.set(targetName, []);
    }
    this._references.get(targetName).push({ layerPath, type, context });
  }

  findReferences(targetName) {
    return this._references.get(targetName) || [];
  }

  /**
   * Iterate every recorded reference as
   *   { targetName, layerPath, type, context }
   * objects.  Used by `world-references.js` to surface
   * references whose `targetName` isn't a known entity.
   */
  *allReferences() {
    for (const [targetName, refs] of this._references) {
      for (const ref of refs) {
        yield { targetName, ...ref };
      }
    }
  }

  /**
   * True if `name` was recorded as a `[[entity]] name = "..."` during
   * the last `indexLayers` call.
   */
  hasEntity(name) {
    return this._entityNames.has(name);
  }

  getAllEntityNames() {
    return Array.from(this._entityNames.entries()).map(([name, layerPath]) => ({ name, layerPath }));
  }

  getAllAnchorNames() {
    return Array.from(this._anchorNames.entries()).map(([name, layerPath]) => ({ name, layerPath }));
  }

  getAllObjectiveIds() {
    return Array.from(this._objectiveIds.entries()).map(([id, layerPath]) => ({ id, layerPath }));
  }
}
