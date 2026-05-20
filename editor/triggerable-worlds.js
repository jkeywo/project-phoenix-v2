/**
 * triggerable-worlds.js — Pure data model for worlds referenced by
 * load_world trigger actions.
 *
 * Scans open world layers for [[trigger]] blocks whose actions contain
 * type = "load_world" and tracks session-only load/unload toggles.
 */

export class TriggerableWorlds {
  constructor() {
    this._paths = [];
    this._loaded = {};
  }

  /**
   * Scan an array of { worldState: object, path: string } (open layers)
   * and extract unique paths referenced by load_world trigger actions.
   *
   * @param {Array<{ worldState: object, path: string }>} layers
   * @returns {string[]} — deduped paths
   */
  scanLayers(layers) {
    const seen = new Set();

    for (const layer of layers) {
      const triggers = layer.worldState && layer.worldState.trigger;
      if (!Array.isArray(triggers)) continue;

      for (const trigger of triggers) {
        if (!trigger || !Array.isArray(trigger.action)) continue;

        for (const action of trigger.action) {
          if (action && action.type === 'load_world' && typeof action.path === 'string') {
            seen.add(action.path);
          }
        }
      }
    }

    this._paths = [...seen];
    return this._paths;
  }

  /**
   * Get current set of triggerable paths (from last scan).
   * @returns {string[]}
   */
  getPaths() {
    return this._paths;
  }

  /**
   * Session-only toggle: returns true if currently toggled ON.
   * @param {string} path
   * @returns {boolean}
   */
  isLoaded(path) {
    return !!this._loaded[path];
  }

  /**
   * Toggle a path on/off (session only, not persisted to TOML).
   * @param {string} path
   * @returns {boolean} — new toggle state
   */
  togglePath(path) {
    this._loaded[path] = !this._loaded[path];
    return this._loaded[path];
  }

  /**
   * Reset all toggles (e.g. when layers change).
   */
  reset() {
    this._loaded = {};
  }
}
