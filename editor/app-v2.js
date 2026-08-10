import { isSupported, pickProjectRoot, getProjectRoot, readFile, writeFile, listDirectory, readBinaryFile, onRootChanged } from './project-root.js';
import { ModeShell } from './mode-shell.js';
import { stringifyWorldToml } from './world-toml.js';
import { stringifyEntityToml } from './entity-toml.js';
import { stringifyFactionToml } from './faction-editor.js';
import { stringifyComplexityToml } from './complexity-editor.js';
import { InvalidationBus } from './invalidation-bus.js';
import { SaveFlow } from './save-flow.js';
import { confirmSaveIfCommented, resetCommentWarningOnRootChange } from './save-confirm.js';
import { mountScenarioMode } from './scenario-mode.js';
import { mountEntityMode } from './entity-mode-view.js';
import { mountDefinitionsMode } from './definitions-mode-view.js';
import { mountModelsMode } from './models-mode-view.js';
import { RigIndex } from './marker-validate.js';
import { parseRigToml, wireRigIndexToSaves } from './models-rig.js';
import { resolveEntityConfigFromText } from './entity-cache.js';

const $ = (id) => document.getElementById(id);

/**
 * Definitions Mode wraps content as { kind: 'faction'|'complexity', data }.
 * This stringifier routes to the per-kind serializer.
 */
function stringifyDefinitionsPayload(payload) {
  if (!payload || typeof payload !== 'object') {
    throw new Error('Definitions payload must be { kind, data }');
  }
  const { kind, data } = payload;
  if (kind === 'faction') return stringifyFactionToml(data);
  if (kind === 'complexity') return stringifyComplexityToml(data);
  throw new Error(`Unknown definitions kind: ${kind}`);
}

const modeShell = new ModeShell();
const invalidationBus = new InvalidationBus();
const saveFlow = new SaveFlow(
  modeShell,
  {
    world: stringifyWorldToml,
    entity: stringifyEntityToml,
    definitions: stringifyDefinitionsPayload,
    // Models Mode caches a ready-made TOML string (see models-mode-view.js);
    // this passthrough lets a global Save All write it without crashing or
    // mis-routing to the entity stringifier. Models' own Save buttons write
    // directly via writeFile and do NOT depend on this path.
    models: (tomlString) => tomlString,
  },
  writeFile,
  invalidationBus,
  (content) => confirmSaveIfCommented(content),
  readFile,
);

// Cross-file model-marker index (issue #758). Entity saves resolve every
// authored `marker` / `markers` reference against the rig sidecar the entity's
// `[mesh]` selects; a reference that names no marker in that sidecar is an
// error and blocks the write, so an invalid attachment never reaches disk.
// Seeded per project root — `validateFile` is synchronous, so the sidecars
// have to be in memory before it runs — and re-seeded on every rig write (see
// `onModelSaved` below), so a marker an author adds in Models Mode is visible
// to the very next entity save without reloading the editor.
const rigIndex = new RigIndex();
saveFlow.setRigIndex(rigIndex);

// Composed-entity save gate (issue #910). A hull that authors only `tags` +
// `includes` inherits its `[behaviour]`/`[mesh]` from a fragment; without this
// the interactive save validates the authored doc and skips those checks. The
// resolver re-composes the LIVE authored text against its on-disk fragment
// closure so the save is validated as the RESOLVED document, while the authored
// document (with `includes` intact) is still what gets written.
saveFlow.setEntityResolver(resolveEntityConfigFromText);

// Keep the index in step with Models-Mode writes. Both write paths — Models'
// own Save button and the SaveFlow 'Models' branch — fire `fireModelSaved`
// with the exact text they wrote, so no re-read (and no async window) is
// involved.
wireRigIndexToSaves(rigIndex, invalidationBus);

async function loadRigIndex() {
  // Evict first: sidecars from a previous root share relative paths with the
  // new one, so a survivor would judge this root's entities against old markers.
  rigIndex.clear();
  let entries = [];
  try {
    entries = await listDirectory('assets/models');
  } catch {
    return; // No root yet / no models dir — marker checks stay skipped.
  }
  for (const entry of entries || []) {
    if (!entry || entry.kind !== 'file' || typeof entry.name !== 'string') continue;
    if (!entry.name.endsWith('.toml')) continue;
    const path = `assets/models/${entry.name}`;
    try {
      rigIndex.set(path, parseRigToml(await readFile(path)));
    } catch {
      // Unparseable sidecar: leave it unindexed so entity marker checks are
      // skipped rather than reporting every marker as missing. Models Mode
      // surfaces the sidecar's own problem.
    }
  }
}

// Per-mode restore callbacks. Each mode registers a `(path, snapshot)`
// handler that knows how to apply a snapshot back to that mode's state.
const restoreCallbacks = {};

function registerRestore(mode, fn) {
  restoreCallbacks[mode] = fn;
}

async function init() {
  if (!isSupported()) {
    showBanner();
    return;
  }

  setupModeSwitcher();
  setupUnsavedGuard();
  setupPickRoot();
  setupChangeRoot();
  setupGlobalUndoShortcuts();
  resetCommentWarningOnRootChange({ onRootChanged });

  // Mount each mode's UI directly — no V1/V2 dual-shell handoff.
  const worldHost = document.getElementById('world-mode-root');
  mountScenarioMode({
    host: worldHost,
    modeShell,
    saveFlow,
    registerRestore,
    invalidationBus,
    io: { readFile, writeFile, listDirectory },
  });

  const entityHost = document.getElementById('entity-mode-root');
  if (entityHost) {
    mountEntityMode({
      host: entityHost,
      modeShell,
      saveFlow,
      registerRestore,
      invalidationBus,
    });
  }

  const definitionsHost = document.getElementById('definitions-mode-root');
  if (definitionsHost) {
    mountDefinitionsMode({
      host: definitionsHost,
      modeShell,
      saveFlow,
      registerRestore,
      invalidationBus,
      io: { readFile, listDirectory },
    });
  }

  const modelsHost = document.getElementById('models-mode-root');
  if (modelsHost) {
    mountModelsMode({
      host: modelsHost,
      modeShell,
      saveFlow,
      invalidationBus,
      io: { readFile, writeFile, listDirectory, readBinaryFile, onRootChanged },
    });
  }

  // Make the persisted root handle available for FSA reads/writes.
  await getProjectRoot();

  // Sidecars must be in memory before any entity save validates markers.
  await loadRigIndex();
  if (typeof onRootChanged === 'function') onRootChanged(() => { loadRigIndex(); });
}

function showBanner() {
  $('browser-not-supported').classList.remove('hidden');
}

const MODE_PANE_IDS = {
  World: 'world-mode-root',
  Entity: 'entity-mode-root',
  Definitions: 'definitions-mode-root',
  Models: 'models-mode-root',
};

function setupModeSwitcher() {
  document.querySelectorAll('.v2-mode-tab').forEach((tab) => {
    tab.addEventListener('click', () => {
      const mode = tab.dataset.mode;
      if (!modeShell.switchMode(mode)) return;

      document.querySelectorAll('.v2-mode-tab').forEach((t) => t.classList.remove('active'));
      tab.classList.add('active');

      for (const [m, id] of Object.entries(MODE_PANE_IDS)) {
        const pane = document.getElementById(id);
        if (!pane) continue;
        pane.classList.toggle('hidden', m !== mode);
      }
    });
  });
}

// Trigger the browser's native "unsaved changes" prompt on tab close /
// reload whenever ANY mode (World/Entity/Definitions/Models) has a dirty
// file tracked in modeShell. Per the platform contract, calling
// preventDefault and setting returnValue is what shows the prompt.
function setupUnsavedGuard() {
  window.addEventListener('beforeunload', (e) => {
    if (!modeShell.hasAnyDirty()) return;
    e.preventDefault();
    e.returnValue = '';
    return '';
  });
}

function setupPickRoot() {
  $('pickRootBtn').addEventListener('click', async () => {
    try {
      await pickProjectRoot();
      $('root-picker').classList.add('hidden');
    } catch (err) {
      $('v2-status').textContent = `Error picking root: ${err.message}`;
    }
  });
}

function setupChangeRoot() {
  $('v2-change-root-btn').addEventListener('click', async () => {
    try {
      await pickProjectRoot();
      $('v2-status').textContent = 'Root changed';
    } catch (err) {
      $('v2-status').textContent = `Error changing root: ${err.message}`;
    }
  });
}

function setupGlobalUndoShortcuts() {
  document.addEventListener('keydown', (e) => {
    // Don't hijack native textarea/input undo.
    const tag = e.target?.tagName;
    if (tag === 'TEXTAREA' || tag === 'INPUT') return;
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.key !== 'z' && e.key !== 'Z') return;

    const mode = modeShell.getCurrentMode();
    const path = modeShell.getActiveFile(mode);
    if (!path) return;

    const direction = e.shiftKey ? 'redo' : 'undo';

    // Early-return without preventDefault when nothing to do, so the browser
    // can still surface its own no-op.
    if (direction === 'undo' && !modeShell.canUndoActive(mode, path)) return;
    if (direction === 'redo' && !modeShell.canRedoActive(mode, path)) return;

    const restore = restoreCallbacks[mode];
    if (!restore) return;

    e.preventDefault();
    restore(modeShell, path, direction);
  });
}

document.addEventListener('DOMContentLoaded', init);
