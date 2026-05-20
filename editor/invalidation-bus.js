export class InvalidationBus {
  constructor() {
    this._entitySavedListeners = [];
    this._worldSavedListeners = [];
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
}
