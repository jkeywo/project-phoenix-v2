export function inferLayerKind(toml) {
  if (toml.spawn && Array.isArray(toml.spawn)) {
    return 'scenario';
  }
  if (toml.anchors || (toml.entity && Array.isArray(toml.entity))) {
    return 'map';
  }
  return 'unknown';
}

import { getSpawns } from './toml-utils.js';

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

export function renderLayersPanel(layerManager, onUpdate) {
  const container = document.getElementById('layersList');
  container.innerHTML = '';

  for (const layer of layerManager.getLayers()) {
    const el = document.createElement('div');
    el.className = 'layer-item' + (layer === layerManager.getActiveLayer() ? ' active' : '');
    el.dataset.filename = layer.filename;

    el.innerHTML = `
      <span class="visibility-toggle">${layer.visible ? '👁' : '🚫'}</span>
      <span class="layer-name">${layer.filename}</span>
      ${layer.isDirty ? '<span class="unsaved-mark">*</span>' : ''}
      <span class="delete-layer" data-action="delete">✕</span>
    `;

    el.querySelector('.visibility-toggle').addEventListener('click', (e) => {
      e.stopPropagation();
      layer.visible = !layer.visible;
      onUpdate();
    });

    el.addEventListener('click', () => {
      layerManager.setActiveLayer(layer);
      onUpdate();
    });

    el.querySelector('.delete-layer').addEventListener('click', (e) => {
      e.stopPropagation();
      layerManager.removeLayer(layer);
      onUpdate();
    });

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