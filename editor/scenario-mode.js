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
 * Every cross-mode collaborator (modeShell, saveFlow, registerRestore,
 * invalidationBus) is passed in explicitly, and the undo controller is
 * constructed here from the injected ModeShell and threaded into the leaf
 * views (CanvasManager, PropertiesPanel) as a normal dependency.
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
import { restoreWorldLayer, createUndoController } from './undo-controller.js';
import { CrossReferenceIndex } from './cross-references.js';
import { renderWorldContentPanel as defaultRenderWorldContent } from './world-content-view.js';
import { renderTriggerableWorldsPanel as defaultRenderTriggerableWorlds } from './triggerable-worlds-panel.js';
import { mountNewWorldButton as defaultMountNewWorldButton } from './new-world-dialog.js';
import { mountOpenWorldButton as defaultMountOpenWorldButton } from './open-world-dialog.js';
import {
  readFile as defaultReadFile,
  writeFile as defaultWriteFile,
  listDirectory as defaultListDirectory,
  onRootChanged as defaultOnRootChanged,
} from './project-root.js';

/**
 * @param {object} opts
 * @param {HTMLElement} [opts.host]
 *   The `#world-mode-root` element that contains the Scenario Mode pane.
 *   DOM IDs that live INSIDE this root (e.g. `#newEntityBtn` in the
 *   entities-palette section) are looked up via `host.querySelector` so
 *   the mode is self-contained.
 *
 *   IDs that live OUTSIDE the root — the shared top-level toolbar buttons
 *   (`#saveAllBtn`, `#saveLayerBtn`) and the toolbar status text
 *   (`#unsavedIndicator`) — remain `document.getElementById` lookups
 *   because they are shared across all modes, not owned by the Scenario
 *   pane. This mirrors how `mountEntityMode` / `mountDefinitionsMode`
 *   treat their hosts: scoped where it makes sense, global where the DOM
 *   is genuinely global.
 *
 *   `host` is optional for backwards-compatibility with older callers and
 *   tests; when omitted the host-scoped lookups also fall back to
 *   `document.getElementById`.
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
  host,
  modeShell,
  saveFlow,
  registerRestore,
  invalidationBus,
  io,
  deps,
} = {}) {
  // Look up an ID under `host` when provided, otherwise fall back to
  // a global lookup. Used for elements that live INSIDE #world-mode-root.
  const findInHost = (id) =>
    (host && typeof host.querySelector === 'function'
      ? host.querySelector(`#${id}`)
      : null) || document.getElementById(id);

  // Surface save outcomes to the shared #v2-status element so a blocked save
  // (issue #757) is visible to the author instead of only console.error.
  const setStatus = (msg) => {
    const el = typeof document !== 'undefined' ? document.getElementById('v2-status') : null;
    if (el) el.textContent = msg;
  };
  const ioDeps = {
    readFile:       io?.readFile       || defaultReadFile,
    writeFile:      io?.writeFile      || defaultWriteFile,
    listDirectory:  io?.listDirectory  || defaultListDirectory,
    tomlParse:      io?.tomlParse      || (typeof window !== 'undefined' ? window.tomlParse : null),
    onRootChanged:  io?.onRootChanged  || defaultOnRootChanged,
  };

  const CanvasManager     = deps?.CanvasManager     || DefaultCanvasManager;
  const PropertiesPanel   = deps?.PropertiesPanel   || DefaultPropertiesPanel;
  const EntityEditor      = deps?.EntityEditor      || DefaultEntityEditor;
  const preloadEntityCache       = deps?.preloadEntityCache       || defaultPreload;
  const renderWorldContentPanel  = deps?.renderWorldContentPanel  || defaultRenderWorldContent;
  const renderTriggerableWorlds  = deps?.renderTriggerableWorldsPanel || defaultRenderTriggerableWorlds;
  const mountNewWorldButton      = deps?.mountNewWorldButton      || defaultMountNewWorldButton;
  const mountOpenWorldButton     = deps?.mountOpenWorldButton     || defaultMountOpenWorldButton;

  const layerManager = new LayerManager();
  const crossRefIndex = new CrossReferenceIndex();
  const undoController = createUndoController({ modeShell });
  const canvasManager = new CanvasManager(
    layerManager,
    onSpawnSelect,
    onSpawnUpdate,
    onSpawnCreate,
    onSpawnDrag,
    undoController,
  );
  canvasManager.init();

  const propertiesPanel = new PropertiesPanel(canvasManager, layerManager, undoController);

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

    mountOpenWorldButton({
      layerManager,
      listDirectory: ioDeps.listDirectory,
      readFile: ioDeps.readFile,
      tomlParse: ioDeps.tomlParse,
      onOpened: renderAll,
    });

    // Re-populate the entity palette whenever the project root changes (e.g.
    // user picks a root after the mode has already mounted, which is the
    // common first-run case — preloadEntityCache runs before a root exists
    // and silently returns nothing).
    ioDeps.onRootChanged(async () => {
      try {
        await preloadEntityCache();
      } catch (err) {
        console.warn('[scenario-mode] preloadEntityCache on root change failed:', err?.message || err);
      }
      entityEditor.loadEntitiesPalette();
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
    // The indicator is shared across all modes. OR-in modeShell's cross-mode
    // dirty state so a dirty Models sidecar (or any other mode) lights it too.
    const dirty = !!layerManager.hasUnsavedChanges?.() || !!modeShell?.hasAnyDirty?.();
    indicator.textContent = dirty ? '● Unsaved changes' : '';
  }

  // ── Toolbar ────────────────────────────────────────────────────────────

  function setupToolbar() {
    // `saveAllBtn` / `saveLayerBtn` / `unsavedIndicator` live in the
    // shared top-level toolbar (outside #world-mode-root) and are reused
    // across all modes — keep them `document`-scoped.
    // `newEntityBtn` lives inside #world-mode-root → scope under `host`.
    const saveAllBtn   = document.getElementById('saveAllBtn');
    const saveLayerBtn = document.getElementById('saveLayerBtn');
    const newEntityBtn = findInHost('newEntityBtn');

    if (saveAllBtn) {
      saveAllBtn.addEventListener('click', async () => {
        if (!saveFlow) {
          console.warn('[scenario-mode] SaveFlow unavailable.');
          return;
        }
        const results = await saveFlow.saveAll();
        const blocked = [];
        for (const r of (results || [])) {
          if (!r.ok) {
            console.error(`Save failed for ${r.path}:`, r.errors);
            blocked.push(r);
          } else {
            const layer = layerManager.getLayers().find((l) => l.filename === r.path);
            if (layer) layer.isDirty = false;
          }
        }
        if (blocked.length > 0) {
          const first = blocked[0];
          setStatus(
            `Save blocked: ${first.path} has ${first.errors.length} error(s) — ${first.errors[0]}`,
          );
        } else {
          setStatus('Saved');
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
          if (!result.ok) {
            console.error('Entity save failed:', result.errors);
            setStatus(`Save blocked: ${result.errors.length} error(s) — ${result.errors[0]}`);
          } else {
            setStatus('Saved');
          }
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
          setStatus(
            `Save blocked: ${activeLayer.filename} has ${result.errors.length} error(s) — ${result.errors[0]}`,
          );
        } else {
          activeLayer.isDirty = false;
          setStatus('Saved');
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
