import { isSupported, pickProjectRoot, getProjectRoot, readFile, writeFile, listDirectory, onRootChanged } from './project-root.js';
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
  },
  writeFile,
  invalidationBus,
  (content) => confirmSaveIfCommented(content),
  readFile,
);

// Per-mode restore callbacks. Each mode registers a `(path, snapshot)`
// handler that knows how to apply a snapshot back to that mode's state.
const restoreCallbacks = {};

function registerRestore(mode, fn) {
  restoreCallbacks[mode] = fn;
}

// Internal binding for cross-module helpers that need access to the
// ModeShell without taking it as a parameter (notably `snapshotForUndo`
// in `undo-controller.js`, which is invoked from ~20 leaf editor modules
// — refactoring those to dependency-inject the ModeShell is out of scope
// for the V1/V2 shell collapse). This is NOT the V1↔V2 boot handoff
// it used to be; mode mounting now takes deps explicitly.
if (typeof window !== 'undefined') {
  window.__editorV2 = { modeShell };
}

async function init() {
  if (!isSupported()) {
    showBanner();
    return;
  }

  setupModeSwitcher();
  setupPickRoot();
  setupChangeRoot();
  setupGlobalUndoShortcuts();
  resetCommentWarningOnRootChange({ onRootChanged });

  // Mount each mode's UI directly — no V1/V2 dual-shell handoff.
  mountScenarioMode({
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

  // Make the persisted root handle available for FSA reads/writes.
  await getProjectRoot();
}

function showBanner() {
  $('browser-not-supported').classList.remove('hidden');
}

const MODE_PANE_IDS = {
  World: 'world-mode-root',
  Entity: 'entity-mode-root',
  Definitions: 'definitions-mode-root',
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
