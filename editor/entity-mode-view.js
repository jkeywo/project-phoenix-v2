/**
 * entity-mode-view.js
 *
 * Top-level coordinator for the Entity Mode three-pane shell.
 *
 *   Left   — entity file list (entity-file-list-view)
 *   Centre — component card stack + add menu (entity-component-stack-view)
 *   Right  — Konva preview + overlay (entity-preview-view)
 *
 * Owns a single EntityModeShell instance and wires:
 *   - file-list selection → readFile → shell.openFile → re-render
 *   - card edits → snapshot-before-mutate → shell.setSection →
 *     saveFlow.setContent('Entity', path, parsed) → markDirty
 *   - registerRestore('Entity', …) for Ctrl+Z dispatch
 *   - entityCache.onInvalidate → refresh file list
 *
 * Imports are deferred so unit-test seams stay clean: discovery / fs
 * helpers are passed in as deps.
 */

import { EntityModeShell } from './entity-mode.js';
import { discoverFactionsAndComplexity } from './faction-complexity-discovery.js';
import { renderEntityFileListView } from './entity-file-list-view.js';
import { renderEntityComponentStackView } from './entity-component-stack-view.js';
import { renderEntityPreviewView } from './entity-preview-view.js';
import { entityCache, onInvalidate, preloadEntityCache } from './entity-cache.js';
import { readFile, listDirectory, getProjectRoot } from './project-root.js';

/**
 * Mount the Entity Mode UI into `host`.
 *
 * @param {object} opts
 * @param {HTMLElement} opts.host
 * @param {import('./mode-shell.js').ModeShell} opts.modeShell
 * @param {import('./save-flow.js').SaveFlow} opts.saveFlow
 * @param {(mode:string, fn:(modeShell, path, direction)=>void)=>void} opts.registerRestore
 * @param {object} [opts.invalidationBus]
 *   Optional InvalidationBus. When supplied, Entity Mode subscribes to
 *   `onFactionSaved` so the faction dropdown picks up Definitions Mode
 *   edits (Slice 6 cross-mode coupling).
 * @param {object} [opts.io]  Override I/O for tests: { readFile, listDirectory,
 *                            preload, onCacheInvalidate, getProjectRoot, discover }.
 * @returns {{ shell: EntityModeShell, render: () => void }}
 */
export function mountEntityMode({ host, modeShell, saveFlow, registerRestore, invalidationBus, io }) {
  if (!host) return null;

  const shell = new EntityModeShell();

  const ioDeps = {
    readFile: io?.readFile || readFile,
    listDirectory: io?.listDirectory || listDirectory,
    preload: io?.preload || preloadEntityCache,
    onCacheInvalidate: io?.onCacheInvalidate || onInvalidate,
    getProjectRoot: io?.getProjectRoot || getProjectRoot,
    discover: io?.discover || discoverFactionsAndComplexity,
    Konva: io?.Konva,
  };

  // Three-pane skeleton.
  host.innerHTML = '';
  const wrap = document.createElement('div');
  wrap.className = 'entity-three-pane';
  host.appendChild(wrap);

  const leftPane = document.createElement('div');
  leftPane.className = 'entity-pane entity-pane-left';
  wrap.appendChild(leftPane);

  const centerPane = document.createElement('div');
  centerPane.className = 'entity-pane entity-pane-center';
  wrap.appendChild(centerPane);

  const rightPane = document.createElement('div');
  rightPane.className = 'entity-pane entity-pane-right';
  wrap.appendChild(rightPane);

  // ── State + render helpers ────────────────────────────────────────────
  let rawTextCache = new Map(); // path → raw TOML (so we can re-open after edits)

  function renderLeft() {
    const paths = shell.getFileList();
    renderEntityFileListView(leftPane, {
      paths,
      activePath: shell.getActiveFile(),
      modeShell,
      onSelect: (path) => loadEntity(path),
    });
  }

  function renderCenter() {
    const active = shell.getActiveFile();
    if (!active) {
      centerPane.innerHTML = '<p class="placeholder">Select an entity file from the left.</p>';
      return;
    }
    renderEntityComponentStackView(centerPane, {
      cards: shell.getComponentCards(),
      deps: makeCardDeps(),
      onAddChoice: (choice) => handleAddChoice(choice),
    });
  }

  function renderRight() {
    const preview = shell.getPreviewPane();
    renderEntityPreviewView(rightPane, preview, { Konva: ioDeps.Konva });
  }

  function renderAll() {
    renderLeft();
    renderCenter();
    renderRight();
  }

  function makeCardDeps() {
    return {
      getFactionOptions: () => shell.getFactionDropdownOptions(),
      getComplexityPaths: () => shell.getComplexityPaths(),
      onEdit: (section, newData) => handleCardEdit(section, newData),
      onDelete: (section) => handleCardDelete(section),
    };
  }

  // ── Edit pipeline ─────────────────────────────────────────────────────

  function handleCardEdit(section, newData) {
    const path = shell.getActiveFile();
    if (!path) return;
    snapshotBeforeMutate(path);
    shell.setSection(section, newData);
    const parsed = shell.getParsedEntity();
    saveFlow.setContent('Entity', path, parsed);
    modeShell.markDirty('Entity', path, true);
    renderCenter();
    renderRight();
    renderLeft(); // dirty-dot indicator
  }

  function handleCardDelete(section) {
    const path = shell.getActiveFile();
    if (!path) return;
    snapshotBeforeMutate(path);
    shell.setSection(section, undefined);
    const parsed = shell.getParsedEntity();
    saveFlow.setContent('Entity', path, parsed);
    modeShell.markDirty('Entity', path, true);
    renderAll();
  }

  function handleAddChoice(choice) {
    const path = shell.getActiveFile();
    if (!path) return;
    snapshotBeforeMutate(path);
    if (choice.kind === 'combo') {
      shell.addCombo(choice.name);
    } else if (choice.kind === 'raw') {
      shell.addComponent(choice.sectionKey);
    }
    const parsed = shell.getParsedEntity();
    saveFlow.setContent('Entity', path, parsed);
    modeShell.markDirty('Entity', path, true);
    renderAll();
  }

  function snapshotBeforeMutate(path) {
    const parsed = shell.getParsedEntity();
    if (!parsed) return;
    modeShell.pushUndoEntry('Entity', path, structuredClone(parsed));
  }

  // ── File load ─────────────────────────────────────────────────────────

  async function loadEntity(path) {
    let text = rawTextCache.get(path);
    if (text === undefined) {
      try {
        text = await ioDeps.readFile(path);
        rawTextCache.set(path, text);
      } catch (err) {
        console.warn(`[entity-mode-view] failed to read ${path}: ${err?.message || err}`);
        return;
      }
    }
    const result = shell.openFile(path, text);
    if (!result.ok) {
      console.warn(`[entity-mode-view] openFile failed for ${path}:`, result.errors);
    }
    modeShell.setActiveFile('Entity', path);
    saveFlow.setContent('Entity', path, shell.getParsedEntity());
    renderAll();
  }

  async function refreshFileList() {
    let entries = [];
    try {
      entries = await ioDeps.listDirectory('assets/entities');
    } catch {
      entries = [];
    }
    const paths = (entries || [])
      .filter((e) => e && e.kind === 'file' && typeof e.name === 'string' && e.name.endsWith('.toml'))
      .map((e) => `assets/entities/${e.name}`)
      .sort();
    shell.setFileList(paths);
    modeShell.setOpenFiles('Entity', paths);
    renderLeft();
  }

  // ── Undo restore registration ─────────────────────────────────────────

  if (typeof registerRestore === 'function') {
    registerRestore('Entity', (ms, path, direction) => {
      const current = structuredClone(shell.getParsedEntity());
      const snap = direction === 'undo'
        ? ms.swapUndoActive('Entity', path, current)
        : ms.swapRedoActive('Entity', path, current);
      if (!snap) return;
      shell.restoreParsed(snap);
      saveFlow.setContent('Entity', path, snap);
      renderCenter();
      renderRight();
    });
  }

  // ── Cache invalidation ────────────────────────────────────────────────
  let invalidationSub = null;
  if (typeof ioDeps.onCacheInvalidate === 'function') {
    invalidationSub = ioDeps.onCacheInvalidate((changedPath) => {
      // Drop our local rawText cache for the changed file so the next
      // openFile re-reads from disk.
      if (changedPath) rawTextCache.delete(changedPath);
      else rawTextCache.clear();
      refreshFileList();
    });
  }

  // ── Faction invalidation (Slice 6 cross-mode coupling) ────────────────
  // When Definitions Mode saves a faction file, re-run discovery so the
  // faction dropdown in component cards reflects the renamed/removed
  // faction immediately.
  async function refreshFactionMap() {
    try {
      const { factionMap } = await ioDeps.discover({
        listDirectory: ioDeps.listDirectory,
        readFile: ioDeps.readFile,
      });
      shell.setFactionMap(factionMap);
      renderCenter();
    } catch (err) {
      console.warn('[entity-mode-view] faction refresh failed:', err?.message || err);
    }
  }

  let factionSavedSub = null;
  if (invalidationBus && typeof invalidationBus.onFactionSaved === 'function') {
    factionSavedSub = invalidationBus.onFactionSaved(() => {
      refreshFactionMap();
    });
  }

  // ── Bootstrap ─────────────────────────────────────────────────────────
  (async () => {
    // Initial discovery (factions + complexity).
    try {
      const { factionMap, complexityPaths } = await ioDeps.discover({
        listDirectory: ioDeps.listDirectory,
        readFile: ioDeps.readFile,
      });
      shell.setFactionMap(factionMap);
      shell.setComplexityPaths(complexityPaths);
    } catch (err) {
      console.warn('[entity-mode-view] discovery failed:', err?.message || err);
    }

    // If no project root is selected yet, show CTA in the left pane.
    try {
      const root = await ioDeps.getProjectRoot();
      if (!root) {
        leftPane.innerHTML = '<p class="placeholder">Pick a project root to load entity files.</p>';
        return;
      }
    } catch {
      // best-effort; carry on
    }

    // Preload entity cache so future Save → invalidate keeps tags etc. fresh.
    try {
      await ioDeps.preload();
    } catch {
      // Already warned upstream.
    }

    await refreshFileList();
  })();

  return {
    shell,
    render: renderAll,
    _internal: {
      loadEntity,
      refreshFileList,
      handleCardEdit,
      handleAddChoice,
      refreshFactionMap,
    },
  };
}
