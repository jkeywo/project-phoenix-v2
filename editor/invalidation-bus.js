export class InvalidationBus {
  constructor() {
    this._entitySavedListeners = [];
    this._worldSavedListeners = [];
    this._factionSavedListeners = [];
    this._modelSavedListeners = [];
  }

  fireEntitySaved(entityPath) {
    for (const cb of this._entitySavedListeners) {
      cb(entityPath);
    }
  }

  fireWorldSaved(worldPath) {
    for (const cb of this._worldSavedListeners) {
      cb(worldPath);
    }
  }

  fireFactionSaved(factionPath) {
    for (const cb of this._factionSavedListeners) {
      cb(factionPath);
    }
  }

  /**
   * A rig sidecar (`assets/models/*.toml`) was written to disk (issue #758).
   *
   * Unlike the other channels this one also carries the TOML text that was
   * just written. The cross-file `RigIndex` entity saves validate against is
   * synchronous, so it has to be re-seeded from the in-hand text at fire time
   * — re-reading the file asynchronously would leave a window in which a
   * marker the author just added is still reported as missing and the entity
   * save is refused.
   *
   * @param {string} sidecarPath
   * @param {string} [tomlText]
   */
  fireModelSaved(sidecarPath, tomlText) {
    for (const cb of this._modelSavedListeners) {
      cb(sidecarPath, tomlText);
    }
  }

  onEntitySaved(callback) {
    this._entitySavedListeners.push(callback);
    return {
      unsubscribe: () => {
        this._entitySavedListeners = this._entitySavedListeners.filter(
          (cb) => cb !== callback
        );
      },
    };
  }

  onWorldSaved(callback) {
    this._worldSavedListeners.push(callback);
    return {
      unsubscribe: () => {
        this._worldSavedListeners = this._worldSavedListeners.filter(
          (cb) => cb !== callback
        );
      },
    };
  }

  onFactionSaved(callback) {
    this._factionSavedListeners.push(callback);
    return {
      unsubscribe: () => {
        this._factionSavedListeners = this._factionSavedListeners.filter(
          (cb) => cb !== callback
        );
      },
    };
  }

  /** Subscribe to rig-sidecar writes. Callback is `(sidecarPath, tomlText)`. */
  onModelSaved(callback) {
    this._modelSavedListeners.push(callback);
    return {
      unsubscribe: () => {
        this._modelSavedListeners = this._modelSavedListeners.filter(
          (cb) => cb !== callback
        );
      },
    };
  }
}
