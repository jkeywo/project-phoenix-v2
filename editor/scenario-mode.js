/**
 * scenario-mode.js
 *
 * Mount the Scenario (World) editor — V1 canvas + layers + properties +
 * world content panel + triggerable worlds + new-world dialog + save flow
 * wiring + Ctrl+Z restore.
 *
 * Extracted from the legacy `editor/app.js` so the editor has a single
 * entry-point script (app-v2.js) that boots Scenario / Entity / Definitions
 * uniformly via `mount*Mode(...)` calls — mirroring the shape of
 * `mountEntityMode` and `mountDefinitionsMode`.
 *
 * The old `window.__editorV2` cross-file handoff is no longer used by this
 * module: every cross-mode collaborator (modeShell, saveFlow,
 * registerRestore, invalidationBus) is passed in explicitly.
 *
 * The V1 legacy file-picker fallback (the path that produced "unsavable"
 * layers and emitted the "Saving through the new SaveFlow will fail"
 * warning) has been deleted — opening world TOMLs now goes through the FSA
 * project root only (triggerable-worlds panel, new-world dialog, etc.).
 */

import { LayerManager, renderLayersPanel } from './layers.js';
import { CanvasManager as DefaultCanvasManager } from './canvas.js';
import { PropertiesPanel as DefaultPropertiesPanel } from './sidebar.js';
import { EntityEditor as DefaultEntityEditor } from './entity-editor.js';
import { getEntityPath } from './toml-utils.js';
import {
  preloadEntityCache as defaultPreload,
  loadEntityConfig,
  invalidateEntity,
} from './entity-cache.js';
import { restoreWorldLayer } from './undo-controller.js';
import { CrossReferenceIndex } from './cross-references.js';
import { renderWorldContentPanel as defaultRenderWorldContent } from './world-content-view.js';
import { renderTriggerableWorldsPanel as defaultRenderTriggerableWorlds } from './triggerable-worlds-panel.js';
import { mountNewWorldButton as defaultMountNewWorldButton } from './new-world-dialog.js';
import {
  readFile as defaultReadFile,
  writeFile as defaultWriteFile,
  listDirectory as defaultListDirectory,
} from './project-root.js';

/**
 * @param {object} opts
 * @param {import('./mode-shell.js').ModeShell} opts.modeShell
 * @param {import('./save-flow.js').SaveFlow} opts.saveFlow
 * @param {(mode:string, fn:(modeShell, path, direction)=>void)=>void} opts.registerRestore
 * @param {object} [opts.invalidationBus]
 * @param {object} [opts.io]   I/O overrides for tests:
 *                             { readFile, writeFile, listDirectory, tomlParse }
 * @param {object} [opts.deps] Class/factory overrides for tests:
 *                             { CanvasManager, PropertiesPanel, EntityEditor,
 *                               preloadEntityCache, renderWorldContentPanel,
 *                               renderTriggerableWorldsPanel,
 *                               mountNewWorldButton }
 * @returns {{
 *   layerManager: LayerManager,
 *   canvasManager: object,
 *   propertiesPanel: object,
 *   entityEditor: object,
 *   renderAll: () => void,
 *   ready: Promise<void>,
 * }}
 */
export function mountScenarioMode({
  modeShell,
  saveFlow,
  registerRestore,
  invalidationBus,
  io,
  deps,
} = {}) {
  const ioDeps = {
    readFile:      io?.readFile      || defaultReadFile,
    writeFile:     io?.writeFile     || defaultWriteFile,
    listDirectory: io?.listDirectory || defaultListDirectory,
    tomlParse:     io?.tomlParse     || (typeof window !== 'undefined' ? window.tomlParse : null),
  };

  const CanvasManager     = deps?.CanvasManager     || DefaultCanvasManager;
  const PropertiesPanel   = deps?.PropertiesPanel   || DefaultPropertiesPanel;
  const EntityEditor      = deps?.EntityEditor      || DefaultEntityEditor;
  const preloadEntityCache       = deps?.preloadEntityCache       || defaultPreload;
  const renderWorldContentPanel  = deps?.renderWorldContentPanel  || defaultRenderWorldContent;
  const renderTriggerableWorlds  = deps?.renderTriggerableWorldsPanel || defaultRenderTriggerableWorlds;
  const mountNewWorldButton      = deps?.mountNewWorldButton      || defaultMountNewWorldButton;

  const layerManager = new LayerManager();
  const crossRefIndex = new CrossReferenceIndex();
  const canvasManager = new CanvasManager(
    layerManager,
    onSpawnSelect,
    onSpawnUpdate,
    onSpawnCreate,
    onSpawnDrag,
  );
  canvasManager.init();

  const propertiesPanel = new PropertiesPanel(canvasManager, layerManager);

  const entityEditor = new EntityEditor(canvasManager, layerManager, onEntitySaved);
  entityEditor.init();

  // Bootstrap (entity cache preload + initial render) returns a Promise
  // so tests can await it deterministically.
  const ready = (async () => {
    try {
      await preloadEntityCache();
    } catch (err) {
      console.warn('[scenario-mode] preloadEntityCache failed:', err?.message || err);
    }
    entityEditor.loadEntitiesPalette();

    // Cross-mode coupling: when Entity Mode saves an entity TOML the World
    // canvas keeps a stale `entity-cache` row. Drop + refetch then re-render.
    if (invalidationBus && typeof invalidationBus.onEntitySaved === 'function') {
      invalidationBus.onEntitySaved(async (savedPath) => {
        try {
          invalidateEntity(savedPath);
          await loadEntityConfig(savedPath);
          canvasManager.renderAll();
        } catch (err) {
          console.warn('[scenario-mode] entity-saved refresh failed:', err?.message || err);
        }
      });
    }

    setupToolbar();
    setupLayersPanel();
    registerWorldUndoRestore();

    mountNewWorldButton({
      layerManager,
      writeFile: ioDeps.writeFile,
      tomlParse: ioDeps.tomlParse,
      onCreated: renderAll,
      getExistingPaths: () => layerManager.getLayers().map((l) => l.filename),
    });

    // Session-only triggerable layers must skip the FSA-backed save flow.
    if (saveFlow && typeof saveFlow.setSessionOnlyChecker === 'function') {
      saveFlow.setSessionOnlyChecker((mode, path) => {
        if (mode !== 'World') return false;
        const layer = layerManager.getLayers().find((l) => l.filename === path);
        return !!(layer && layer._sessionOnly);
      });
    }

    renderAll();
  })();

  // ── Spawn callbacks ────────────────────────────────────────────────────

  function onSpawnSelect(spawn, layer) {
    propertiesPanel.render(spawn, layer);
  }

  function onSpawnSelectFromTree(spawn, layer) {
    layerManager.setActiveLayer(layer);
    canvasManager.selectedSpawn = spawn ? { spawn, layer } : null;
    propertiesPanel.render(spawn, layer);
  }

  function onSpawnUpdate(_spawn, _layer) {
    renderAll();
  }

  function onSpawnDrag(_spawn, _layer) {
    propertiesPanel.updatePositionFields(layerManager);
    updateUnsavedIndicator();
  }

  function onSpawnCreate(spawn, _layer) {
    loadEntityConfig(getEntityPath(spawn));
    renderAll();
  }

  function onEntitySaved(_entity) {
    entityEditor.loadEntitiesPalette();
  }

  // ── Undo restore ───────────────────────────────────────────────────────

  function registerWorldUndoRestore() {
    if (typeof registerRestore !== 'function') return;

    registerRestore('World', (ms, path, direction) => {
      const layer = layerManager.getLayers().find((l) => l.filename === path);
      if (!layer) return;

      const current = structuredClone(layer.toml);
      const snapshot = direction === 'undo'
        ? ms.swapUndoActive('World', path, current)
        : ms.swapRedoActive('World', path, current);
      if (!snapshot) return;

      restoreWorldLayer(layerManager, path, snapshot);
      renderAll();

      const sel = canvasManager.selectedSpawn;
      if (sel && sel.layer === layer) {
        canvasManager.deselectSpawn();
        propertiesPanel.render(null, null);
      }
    });
  }

  // ── Render ─────────────────────────────────────────────────────────────

  function renderAll() {
    crossRefIndex.indexLayers(canvasManager.buildV2Layers());

    renderLayersPanel(layerManager, renderAll, onSpawnSelectFromTree);
    canvasManager.renderAll();

    const activeLayer = layerManager.getActiveLayer?.();
    renderWorldContentPanel({
      worldState: activeLayer?.toml ?? null,
      crossRefIndex,
      activeLayerPath: activeLayer?.filename ?? null,
      onSelectEntity: (name) => canvasManager.selectByEntityName(name),
      onSelectTrigger: (triggerIndex) => {
        const layer = layerManager.getActiveLayer?.();
        if (!layer) return;
        propertiesPanel.render({ type: 'trigger', triggerIndex, layer });
      },
      onSelectComms: (commsIndex) => {
        const layer = layerManager.getActiveLayer?.();
        if (!layer) return;
        propertiesPanel.render({ type: 'comms', commsIndex, layer });
      },
    });

    updateUnsavedIndicator();

    renderTriggerableWorlds({
      layerManager,
      onLayersChanged: renderAll,
      readFile: ioDeps.readFile,
      listDirectory: ioDeps.listDirectory,
      tomlParse: ioDeps.tomlParse,
    })?.catch?.((err) => {
      console.warn('[scenario-mode] triggerable-worlds panel render failed:', err?.message || err);
    });

    // Mirror the active layer into ModeShell + SaveFlow so the toolbar
    // Save buttons (and Ctrl+Z) act on the right file.
    const active = layerManager.getActiveLayer?.();
    if (modeShell && active && active.filename) {
      modeShell.setActiveFile('World', active.filename);
      modeShell.setActiveLayer('World', active.filename);

      const openFilenames = layerManager.getLayers().map((l) => l.filename).filter(Boolean);
      modeShell.setOpenFiles('World', openFilenames);

      if (saveFlow) {
        for (const layer of layerManager.getLayers()) {
          if (!layer.filename) continue;
          saveFlow.setContent('World', layer.filename, layer.toml);
        }
      }
    }
  }

  function updateUnsavedIndicator() {
    const indicator = document.getElementById('unsavedIndicator');
    if (!indicator) return;
    indicator.textContent = layerManager.hasUnsavedChanges?.() ? '● Unsaved changes' : '';
  }

  // ── Toolbar ────────────────────────────────────────────────────────────

  function setupToolbar() {
    const saveAllBtn   = document.getElementById('saveAllBtn');
    const saveLayerBtn = document.getElementById('saveLayerBtn');
    const newEntityBtn = document.getElementById('newEntityBtn');

    if (saveAllBtn) {
      saveAllBtn.addEventListener('click', async () => {
        if (!saveFlow) {
          console.warn('[scenario-mode] SaveFlow unavailable.');
          return;
        }
        const results = await saveFlow.saveAll();
        for (const r of (results || [])) {
          if (!r.ok) {
            console.error(`Save failed for ${r.path}:`, r.errors);
          } else {
            const layer = layerManager.getLayers().find((l) => l.filename === r.path);
            if (layer) layer.isDirty = false;
          }
        }
        renderAll();
      });
    }

    if (saveLayerBtn) {
      saveLayerBtn.addEventListener('click', async () => {
        if (!saveFlow) {
          console.warn('[scenario-mode] SaveFlow unavailable.');
          return;
        }
        // Entity Mode owns its own save pipeline via mountEntityMode.
        const currentMode = modeShell && typeof modeShell.getCurrentMode === 'function'
          ? modeShell.getCurrentMode()
          : 'World';
        if (currentMode === 'Entity') {
          const result = await saveFlow.saveActive();
          if (!result.ok) console.error('Entity save failed:', result.errors);
          return;
        }

        const activeLayer = layerManager.getActiveLayer?.();
        if (!activeLayer) {
          console.warn('[scenario-mode] No active layer.');
          return;
        }
        saveFlow.setContent('World', activeLayer.filename, activeLayer.toml);
        modeShell.setActiveFile('World', activeLayer.filename);
        const result = await saveFlow.saveActive();
        if (!result.ok) {
          console.error(`Save failed for ${activeLayer.filename}:`, result.errors);
        } else {
          activeLayer.isDirty = false;
          renderAll();
        }
      });
    }

    if (newEntityBtn) {
      newEntityBtn.addEventListener('click', () => {
        entityEditor.openModal();
      });
    }
  }

  function setupLayersPanel() {
    // The legacy "+ Add Layer" file-picker fallback has been removed.
    // Opening world TOMLs now goes through the FSA project root via the
    // triggerable-worlds panel and the + New World dialog.
  }

  return {
    layerManager,
    canvasManager,
    propertiesPanel,
    entityEditor,
    renderAll,
    ready,
  };
}
