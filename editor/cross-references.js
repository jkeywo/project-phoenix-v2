/**
 * cross-references.js — Name index over the open world layers.
 *
 * It used to record every *use* of an entity name too, but every use site it
 * knew — `[[trigger]]`, `[[trigger.action]]`, `[[comms]]`, `[[comms.response]]`
 * — was deleted with the declarative scenario front-end (issue #985). A world's
 * scenario logic now lives in its `[script]` Rhai body, where names appear
 * inside script source this TOML walk cannot read, so the reference half of the
 * index (and the `add_objective` id collection built on it) had no source left
 * and was removed rather than kept as a permanently empty map.
 *
 * What survives is the DECLARATION half: the names a world still authors in
 * TOML — `[[entity]] name = "..."` and the `[anchors]` keys.
 */
export class CrossReferenceIndex {
  constructor() {
    this._entityNames = new Map();
    this._anchorNames = new Map();
  }

  indexLayers(layers) {
    this._entityNames.clear();
    this._anchorNames.clear();

    for (const layer of layers) {
      const { path, worldState } = layer;
      if (!worldState || typeof worldState !== 'object') continue;

      this._collectEntityNames(worldState, path);
      this._collectAnchorNames(worldState, path);
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
}
