export function inferLayerKind(toml) {
  if (toml.spawn && Array.isArray(toml.spawn)) {
    return 'scenario';
  }
  if (toml.anchors || (toml.entity && Array.isArray(toml.entity))) {
    return 'map';
  }
  return 'unknown';
}

import { getSpawns, getSpawnName, getEntityPath } from './toml-utils.js';

export class LayerManager {
  constructor() {
    this.layers = [];
    this.activeLayer = null;
  }

  async addLayer(fileHandle) {
    try {
      const file = await fileHandle.getFile();
      const text = await file.text();
      const toml = await window.tomlParse(text);
      const kind = inferLayerKind(toml);

      const layer = {
        fileHandle,
        filename: file.name,
        toml,
        kind,
        visible: true,
        active: false,
        konvaLayer: null,
        originalText: text,
        isDirty: false
      };

      this.layers.push(layer);
      this.activeLayer = layer;
      return layer;
    } catch (err) {
      console.error('Failed to add layer:', err);
      throw err;
    }
  }

  /**
   * Add a layer whose TOML has already been parsed in-memory (e.g. a
   * session-only "triggerable world" loaded via the side panel, with no
   * dedicated `FileSystemFileHandle`). The layer is appended and becomes
   * the active layer, matching `addLayer` behaviour.
   *
   * @param {string} filename — root-relative path used as the layer's id.
   * @param {object} parsedToml — already-parsed TOML object.
   * @param {object} [opts]
   * @param {boolean} [opts.sessionOnly=false] — when true, marks the
   *   layer with `_sessionOnly: true` so `SaveFlow.getDirtyFiles()` skips
   *   it (the session loader has no write surface for these).
   * @returns {object} the new layer
   */
  addInMemoryLayer(filename, parsedToml, opts = {}) {
    const sessionOnly = !!opts.sessionOnly;
    const layer = {
      fileHandle: null,
      filename,
      toml: parsedToml,
      kind: inferLayerKind(parsedToml),
      visible: true,
      active: false,
      konvaLayer: null,
      originalText: null,
      isDirty: false,
      _sessionOnly: sessionOnly,
    };
    this.layers.push(layer);
    this.activeLayer = layer;
    return layer;
  }

  removeLayer(layer) {
    const idx = this.layers.indexOf(layer);
    if (idx !== -1) {
      this.layers.splice(idx, 1);
      if (this.activeLayer === layer) {
        this.activeLayer = this.layers[0] || null;
      }
    }
  }

  setActiveLayer(layer) {
    this.activeLayer = layer;
  }

  getActiveLayer() {
    return this.activeLayer;
  }

  getLayers() {
    return this.layers;
  }

  hasUnsavedChanges() {
    return this.layers.some(l => l.isDirty);
  }

  markLayerDirty(layer, dirty = true) {
    layer.isDirty = dirty;
    if (dirty) {
      layer.originalText = null;
    } else {
      layer.originalText = null;
    }
  }
}

export function renderLayersPanel(layerManager, onUpdate, onSpawnSelect) {
  const container = document.getElementById('layersList');
  container.innerHTML = '';

  for (const layer of layerManager.getLayers()) {
    if (layer._expanded === undefined) layer._expanded = true;

    const el = document.createElement('div');
    el.className = 'layer-item' + (layer === layerManager.getActiveLayer() ? ' active' : '');

    const header = document.createElement('div');
    header.className = 'layer-header';
    header.innerHTML = `
      <span class="layer-expand">${layer._expanded ? '▾' : '▸'}</span>
      <span class="visibility-toggle">${layer.visible ? '👁' : '🚫'}</span>
      <span class="layer-name">${layer.filename}</span>
      ${layer.isDirty ? '<span class="unsaved-mark">*</span>' : ''}
      <span class="delete-layer">✕</span>
    `;

    header.querySelector('.layer-expand').addEventListener('click', (e) => {
      e.stopPropagation();
      layer._expanded = !layer._expanded;
      onUpdate();
    });

    header.querySelector('.visibility-toggle').addEventListener('click', (e) => {
      e.stopPropagation();
      layer.visible = !layer.visible;
      onUpdate();
    });

    header.querySelector('.delete-layer').addEventListener('click', (e) => {
      e.stopPropagation();
      layerManager.removeLayer(layer);
      onUpdate();
    });

    header.addEventListener('click', () => {
      layerManager.setActiveLayer(layer);
      onUpdate();
    });

    el.appendChild(header);

    if (layer._expanded) {
      const spawns = getSpawns(layer);
      if (spawns.length > 0) {
        const spawnList = document.createElement('div');
        spawnList.className = 'layer-spawn-list';
        for (const spawn of spawns) {
          const spawnEl = document.createElement('div');
          spawnEl.className = 'layer-spawn-item';
          const name = getSpawnName(spawn);
          const path = getEntityPath(spawn);
          const short = path ? path.split('/').pop().replace('.toml', '') : '';
          spawnEl.innerHTML = `<span class="spawn-bullet">◦</span><span class="spawn-name">${name}</span>${short ? `<span class="spawn-entity-label">${short}</span>` : ''}`;
          spawnEl.addEventListener('click', (e) => {
            e.stopPropagation();
            if (onSpawnSelect) onSpawnSelect(spawn, layer);
            onUpdate();
          });
          spawnList.appendChild(spawnEl);
        }
        el.appendChild(spawnList);
      } else {
        const empty = document.createElement('div');
        empty.className = 'layer-spawn-empty';
        empty.textContent = 'no entities';
        el.appendChild(empty);
      }
    }

    container.appendChild(el);
  }
}

export function getEntityKindFromTags(tags = []) {
  if (tags.includes('player')) return 'player';
  if (tags.includes('station')) return 'station';
  if (tags.includes('asteroid')) return 'asteroid';
  if (tags.includes('region')) return 'region';
  if (tags.includes('enemy') || tags.includes('pirate')) return 'enemy';
  return 'unknown';
}

export function getColorForEntity(tags = []) {
  const kind = getEntityKindFromTags(tags);
  switch (kind) {
    case 'player': return '#2196f3';
    case 'station': return '#ffc107';
    case 'asteroid': return '#9e9e9e';
    case 'region': return '#4caf50';
    case 'enemy': return '#f44336';
    default: return '#9c27b0';
  }
}