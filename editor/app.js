import { LayerManager, renderLayersPanel, inferLayerKind } from './layers.js';
import { CanvasManager } from './canvas.js';
import { PropertiesPanel } from './sidebar.js';
import { EntityEditor } from './entity-editor.js';
import { stringifyToml, getEntityPath } from './toml-utils.js';
import { preloadEntityCache, loadEntityConfig } from './entity-cache.js';
import { restoreScenarioLayer } from './undo-controller.js';

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
  registerScenarioUndoRestore();
  renderAll();
}

function registerScenarioUndoRestore() {
  const v2 = window.__editorV2;
  if (!v2 || typeof v2.registerRestore !== 'function') return;

  v2.registerRestore('Scenario', (modeShell, path, direction) => {
    const layer = layerManager.getLayers().find((l) => l.filename === path);
    if (!layer) return;

    // Snapshot CURRENT (post-mutation) state so the opposite stack can
    // restore it later. structuredClone keeps the undo entries independent
    // of subsequent in-place edits.
    const current = structuredClone(layer.toml);

    const snapshot = direction === 'undo'
      ? modeShell.swapUndoActive('Scenario', path, current)
      : modeShell.swapRedoActive('Scenario', path, current);
    if (!snapshot) return;

    restoreScenarioLayer(layerManager, path, snapshot);

    // Re-render canvas + layers panel and refresh the properties sidebar
    // if a spawn is still selected (and still exists in the restored TOML).
    renderAll();

    const sel = canvasManager.selectedSpawn;
    if (sel && sel.layer === layer) {
      // The spawn object may have been replaced by structuredClone; clear
      // selection to avoid pointing at a stale reference.
      canvasManager.deselectSpawn();
      propertiesPanel.render(null, null);
    }
  });
}

function onSpawnSelect(spawn, layer) {
  propertiesPanel.render(spawn, layer);
}

function onSpawnSelectFromTree(spawn, layer) {
  layerManager.setActiveLayer(layer);
  canvasManager.selectedSpawn = spawn ? { spawn, layer } : null;
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
  renderLayersPanel(layerManager, renderAll, onSpawnSelectFromTree);
  canvasManager.renderAll();
  updateUnsavedIndicator();

  // Mirror the V1 active layer into ModeShell so V2 features (save,
  // undo shortcuts, invalidation) target the right file.
  const active = layerManager.getActiveLayer();
  const editorV2 = window.__editorV2;
  if (editorV2 && editorV2.modeShell && active && active.filename) {
    editorV2.modeShell.setActiveFile('Scenario', active.filename);
    editorV2.modeShell.setActiveLayer('Scenario', active.filename);

    const openFilenames = layerManager.getLayers()
      .map((l) => l.filename)
      .filter(Boolean);
    editorV2.modeShell.setOpenFiles('Scenario', openFilenames);

    // Mirror the saveFlow content cache so V2's saveActive has a parsed
    // payload to serialize when the toolbar Save buttons run.
    if (editorV2.saveFlow) {
      for (const layer of layerManager.getLayers()) {
        if (!layer.filename) continue;
        editorV2.saveFlow.setContent('Scenario', layer.filename, layer.toml);
      }
    }
  }
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

        console.warn(
          `[editor] Layer "${file.name}" was opened via file picker, so its root-relative path is unknown. ` +
          `Saving through the new SaveFlow will fail. Use "Pick Project Root" and reopen the file from there.`
        );
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
    const v2 = window.__editorV2;
    if (!v2 || !v2.saveFlow) {
      console.warn('[editor] SaveFlow not yet initialised; cannot save.');
      return;
    }
    const results = await v2.saveFlow.saveAll(null);
    for (const r of results) {
      if (!r.ok) {
        console.error(`Save failed for ${r.path}:`, r.errors);
      } else {
        const layer = layerManager.getLayers().find((l) => l.filename === r.path);
        if (layer) layer.isDirty = false;
      }
    }
    renderAll();
  });

  document.getElementById('saveLayerBtn').addEventListener('click', async () => {
    const v2 = window.__editorV2;
    const activeLayer = layerManager.getActiveLayer();
    if (!v2 || !v2.saveFlow || !activeLayer) {
      console.warn('[editor] No active layer or SaveFlow unavailable.');
      return;
    }
    // renderAll() already mirrored the parsed payload into saveFlow's cache,
    // but call again here in case the user edited and clicked save before a
    // render tick fired.
    v2.saveFlow.setContent('Scenario', activeLayer.filename, activeLayer.toml);
    v2.modeShell.setActiveFile('Scenario', activeLayer.filename);
    const result = await v2.saveFlow.saveActive(null);
    if (!result.ok) {
      console.error(`Save failed for ${activeLayer.filename}:`, result.errors);
    } else {
      activeLayer.isDirty = false;
      renderAll();
    }
  });

  document.getElementById('newEntityBtn').addEventListener('click', () => {
    entityEditor.openModal();
  });
}

function setupLayersPanel() {
  document.getElementById('addLayerBtn').addEventListener('click', () => {
    fileInput.click();
  });
}

window.addEventListener('DOMContentLoaded', init);