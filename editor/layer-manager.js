/**
 * layer-manager.js — Pure data model for multi-layer world loading.
 *
 * When a root world TOML is opened, every path listed in its `extra_worlds`
 * array is auto-loaded as a child layer. The model tracks:
 *   - The full layer tree (root + children)
 *   - Which layer is currently "active" (defaults to root)
 *   - Per-layer dirty state (in-memory state diverged from the on-disk snapshot)
 *
 * All mutations (addSpawn, addAnchor, markDirty) target the active layer and
 * return a new LayerManager instance — the class is effectively immutable.
 */

/**
 * Internal representation of one layer.
 *
 * @typedef {Object} Layer
 * @property {string}  path        - File path used as the stable ID.
 * @property {Object}  worldState  - Parsed TOML object (mutable in-memory copy).
 * @property {boolean} dirty       - True when worldState diverges from disk snapshot.
 * @property {boolean} active      - True for the currently-active layer.
 * @property {string[]} children   - Paths of direct child layers (extra_worlds).
 */

export class LayerManager {
  /**
   * @param {Layer[]} _layers - Internal flat list, root is always index 0.
   */
  constructor(_layers = []) {
    this._layers = _layers;
  }

  // ── Factory ──────────────────────────────────────────────────────────────────

  /**
   * Open a root world and its extra_worlds children.
   *
   * @param {string} rootPath                   - File path of the root world.
   * @param {Object} rootContent                - Parsed TOML object for the root.
   * @param {Record<string, Object>} extraWorldContents
   *   Map of { [childPath]: parsedTomlObject } for every entry in
   *   rootContent.extra_worlds. Paths that are missing from this map are
   *   skipped gracefully.
   * @returns {LayerManager}
   */
  static openRoot(rootPath, rootContent, extraWorldContents = {}) {
    const childPaths = Array.isArray(rootContent.extra_worlds)
      ? rootContent.extra_worlds.filter(p => typeof p === 'string')
      : [];

    const rootLayer = {
      path: rootPath,
      worldState: deepClone(rootContent),
      dirty: false,
      active: true,
      children: childPaths,
    };

    const childLayers = childPaths
      .filter(p => extraWorldContents[p] !== undefined)
      .map(p => ({
        path: p,
        worldState: deepClone(extraWorldContents[p]),
        dirty: false,
        active: false,
        children: [],
      }));

    return new LayerManager([rootLayer, ...childLayers]);
  }

  // ── Queries ───────────────────────────────────────────────────────────────────

  /**
   * Returns a hierarchical tree rooted at the root layer.
   * Each node: `{ path, dirty, active, children: [...] }`.
   *
   * @returns {Array<{ path: string, dirty: boolean, active: boolean, children: Array }>}
   */
  getLayerTree() {
    if (this._layers.length === 0) return [];
    const root = this._layers[0];
    return [this._buildNode(root)];
  }

  _buildNode(layer) {
    const children = layer.children
      .map(childPath => this._layers.find(l => l.path === childPath))
      .filter(Boolean)
      .map(child => this._buildNode(child));

    return {
      path: layer.path,
      dirty: layer.dirty,
      active: layer.active,
      children,
    };
  }

  /**
   * Returns the active layer, or null if none exists.
   * @returns {Layer|null}
   */
  getActiveLayer() {
    return this._layers.find(l => l.active) ?? null;
  }

  /**
   * Returns the current in-memory world state for the given path.
   * @param {string} path
   * @returns {Object|null}
   */
  getWorldState(path) {
    const layer = this._layers.find(l => l.path === path);
    return layer ? layer.worldState : null;
  }

  // ── Mutations (return new LayerManager) ──────────────────────────────────────

  /**
   * Set the active layer by path. Deactivates the previous active layer.
   * No-op if path is not found.
   *
   * @param {string} path
   * @returns {LayerManager}
   */
  setActiveLayer(path) {
    if (!this._layers.find(l => l.path === path)) return this;
    const layers = this._layers.map(l => ({ ...l, active: l.path === path }));
    return new LayerManager(layers);
  }

  /**
   * Mark a layer as dirty (in-memory state has diverged from disk).
   *
   * @param {string} path
   * @returns {LayerManager}
   */
  markDirty(path) {
    const layers = this._layers.map(l =>
      l.path === path ? { ...l, dirty: true } : l
    );
    return new LayerManager(layers);
  }

  /**
   * Add a spawn entity to the active layer's `entity` array and mark it dirty.
   *
   * @param {Object} spawn - Entity object to append.
   * @returns {LayerManager}
   */
  addSpawn(spawn) {
    return this._updateActive(layer => {
      const entities = Array.isArray(layer.worldState.entity)
        ? [...layer.worldState.entity, spawn]
        : [spawn];
      return {
        ...layer,
        dirty: true,
        worldState: { ...layer.worldState, entity: entities },
      };
    });
  }

  /**
   * Add or update a named anchor in the active layer's `anchors` map and mark dirty.
   *
   * @param {string} name        - Anchor key.
   * @param {[number, number, number]} position - [x, y, z] array.
   * @returns {LayerManager}
   */
  addAnchor(name, position) {
    return this._updateActive(layer => ({
      ...layer,
      dirty: true,
      worldState: {
        ...layer.worldState,
        anchors: {
          ...(layer.worldState.anchors ?? {}),
          [name]: position,
        },
      },
    }));
  }

  // ── Private helpers ───────────────────────────────────────────────────────────

  /**
   * Apply `fn` to the active layer and return a new LayerManager.
   * If no active layer exists, returns `this` unchanged.
   *
   * @param {(layer: Layer) => Layer} fn
   * @returns {LayerManager}
   */
  _updateActive(fn) {
    const active = this.getActiveLayer();
    if (!active) return this;
    const layers = this._layers.map(l => (l.active ? fn(l) : l));
    return new LayerManager(layers);
  }
}

// ── Utility ───────────────────────────────────────────────────────────────────

/**
 * Shallow-safe deep clone via JSON round-trip.
 * Suitable for plain TOML-parsed objects (no functions, no circular refs).
 *
 * @param {Object} obj
 * @returns {Object}
 */
function deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}
