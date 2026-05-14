import { LayerManager, renderLayersPanel, inferLayerKind } from './layers.js';
import { CanvasManager } from './canvas.js';
import { PropertiesPanel } from './sidebar.js';
import { EntityEditor } from './entity-editor.js';
import { stringifyToml, getEntityPath } from './toml-utils.js';
import { preloadEntityCache, loadEntityConfig } from './entity-cache.js';

const layerManager = new LayerManager();
let canvasManager;
let propertiesPanel;
let entityEditor;
let fileInput;

async function init() {
  canvasManager = new CanvasManager(
    layerManager,
    onSpawnSelect,
    onSpawnUpdate,
    onSpawnCreate,
    onSpawnDrag
  );
  canvasManager.init();

  propertiesPanel = new PropertiesPanel(canvasManager, layerManager);

  entityEditor = new EntityEditor(
    canvasManager,
    layerManager,
    onEntitySaved
  );
  entityEditor.init();

  await preloadEntityCache();
  entityEditor.loadEntitiesPalette();

  setupToolbar();
  setupLayersPanel();
  renderAll();
}

function onSpawnSelect(spawn, layer) {
  propertiesPanel.render(spawn, layer);
}

function onSpawnUpdate(spawn, layer) {
  renderAll();
}

function onSpawnDrag(spawn, layer) {
  propertiesPanel.updatePositionFields(layerManager);
  updateUnsavedIndicator();
}

function onSpawnCreate(spawn, layer) {
  loadEntityConfig(getEntityPath(spawn));
  renderAll();
}

function onEntitySaved(entity) {
  entityEditor.loadEntitiesPalette();
}

function renderAll() {
  renderLayersPanel(layerManager, () => {
    canvasManager.renderAll();
    updateUnsavedIndicator();
  });
  canvasManager.renderAll();
  updateUnsavedIndicator();
}

function updateUnsavedIndicator() {
  const indicator = document.getElementById('unsavedIndicator');
  if (layerManager.hasUnsavedChanges()) {
    indicator.textContent = '● Unsaved changes';
  } else {
    indicator.textContent = '';
  }
}

function setupToolbar() {
  const openFileBtn = document.getElementById('openFileBtn');
  const openFileMenu = document.getElementById('openFileMenu');

  openFileBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    openFileMenu.classList.toggle('show');
  });

  document.addEventListener('click', () => {
    openFileMenu.classList.remove('show');
  });

  fileInput = document.createElement('input');
  fileInput.type = 'file';
  fileInput.accept = '.toml';
  fileInput.multiple = true;
  fileInput.style.display = 'none';
  document.body.appendChild(fileInput);

  fileInput.addEventListener('change', async (e) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;

    for (const file of files) {
      try {
        const text = await file.text();
        const toml = await window.tomlParse(text);
        const kind = inferLayerKind(toml);

        const layer = {
          fileHandle: file,
          filename: file.name,
          toml,
          kind,
          visible: true,
          active: false,
          konvaLayer: null,
          originalText: text,
          isDirty: false
        };

        layerManager.layers.push(layer);
        layerManager.activeLayer = layer;
      } catch (err) {
        console.error('Failed to parse file:', file.name, err);
      }
    }
    renderAll();
    fileInput.value = '';
  });

  document.getElementById('openMapScenario').addEventListener('click', (e) => {
    e.preventDefault();
    fileInput.click();
  });

  document.getElementById('saveAllBtn').addEventListener('click', async () => {
    for (const layer of layerManager.getLayers()) {
      await saveLayer(layer);
    }
  });

  document.getElementById('saveLayerBtn').addEventListener('click', async () => {
    const activeLayer = layerManager.getActiveLayer();
    if (activeLayer) {
      await saveLayer(activeLayer);
    }
  });

  document.getElementById('newEntityBtn').addEventListener('click', () => {
    entityEditor.openModal();
  });
}

async function saveLayer(layer) {
  try {
    const raw = stringifyToml(layer.toml);

    if (layer.fileHandle.createWritable) {
      const writable = await layer.fileHandle.createWritable();
      await writable.write(raw);
      await writable.close();
    } else {
      const blob = new Blob([raw], { type: 'application/toml' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = layer.filename;
      a.click();
      URL.revokeObjectURL(url);
      console.log(`Downloaded: ${layer.filename} (save-as required)`);
    }

    layer.isDirty = false;
    console.log(`Saved: ${layer.filename}`);
  } catch (err) {
    console.error(`Failed to save ${layer.filename}:`, err);
  }
}

function setupLayersPanel() {
  document.getElementById('addLayerBtn').addEventListener('click', () => {
    fileInput.click();
  });
}

window.addEventListener('DOMContentLoaded', init);