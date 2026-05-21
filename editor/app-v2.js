import { isSupported, pickProjectRoot, getProjectRoot, readFile, writeFile, onRootChanged } from './project-root.js';
import { ModeShell } from './mode-shell.js';
import { stringifyWorldToml } from './world-toml.js';
import { stringifyEntityToml } from './entity-toml.js';
import { stringifyFactionToml } from './faction-editor.js';
import { stringifyComplexityToml } from './complexity-editor.js';
import { InvalidationBus } from './invalidation-bus.js';
import { SaveFlow } from './save-flow.js';
import { confirmSaveIfCommented, resetCommentWarningOnRootChange } from './save-confirm.js';

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
);

let currentFilePath = null;

// Per-mode restore callbacks. Cross-file decoupling: each mode (Scenario in
// Slice 1; Entity/Definitions in later slices) registers a `(path, snapshot)`
// handler that knows how to apply a snapshot back to that mode's V1 state.
const restoreCallbacks = {};

function registerRestore(mode, fn) {
  restoreCallbacks[mode] = fn;
}

// Expose to V1 (app.js) and to dev consoles. This is the cross-file
// integration point until Slice 2+ moves everything into V2 properly.
if (typeof window !== 'undefined') {
  window.__editorV2 = {
    modeShell,
    invalidationBus,
    saveFlow,
    registerRestore,
  };
}

async function init() {
  if (!isSupported()) {
    showBanner();
    return;
  }

  setupModeSwitcher();
  setupPickRoot();
  setupChangeRoot();
  setupOpenFile();
  setupSaveFile();
  setupGlobalUndoShortcuts();
  resetCommentWarningOnRootChange({ onRootChanged });

  // V1 map editor (canvas + layers) is the default view.
  // V2 text editor stays hidden; it's shown only when the user triggers it
  // from the V1 toolbar. The root handle (if persisted) is available for
  // V2's File System Access API read/write when needed.
  await getProjectRoot();
}

function showBanner() {
  $('browser-not-supported').classList.remove('hidden');
}

const MODE_PANE_IDS = {
  Scenario: 'scenario-mode-root',
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

function setupOpenFile() {
  const fileInput = document.createElement('input');
  fileInput.type = 'file';
  fileInput.accept = '.toml';
  fileInput.multiple = false;
  fileInput.style.display = 'none';
  document.body.appendChild(fileInput);

  fileInput.addEventListener('change', async (e) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const content = await file.text();
      currentFilePath = file.name;
      $('v2-file-content').value = content;
      $('v2-status').textContent = `Loaded: ${file.name}`;
    } catch (err) {
      $('v2-status').textContent = `Error: ${err.message}`;
    }
    fileInput.value = '';
  });

  $('v2-open-btn').addEventListener('click', () => {
    fileInput.click();
  });
}

function setupSaveFile() {
  $('v2-save-btn').addEventListener('click', async () => {
    if (!currentFilePath) {
      $('v2-status').textContent = 'No file open';
      return;
    }

    try {
      const content = $('v2-file-content').value;
      await writeFile(currentFilePath, content);
      $('v2-status').textContent = `Saved: ${currentFilePath}`;
    } catch (err) {
      $('v2-status').textContent = `Error: ${err.message}`;
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
