/**
 * Semantic actions for the existing browser MOD editor (issue #1321).
 *
 * This is presentation/input metadata only.  The adapter opens the existing
 * file chooser; `mod-mode-view.js` still owns archive parsing and validation.
 */

import { createSemanticActionRegistry } from '../gui/semantic-action-registry.js';

export const MOD_ACTION_CONTEXT = 'editor.mod';
export const MOD_IMPORT_ACTION_ID = 'editor.mod.import';

/**
 * T2 deliberately adds only the import tracer to the old editor.  The object
 * is exported so the M6 boundary is machine-testable rather than an aspiration
 * hidden in prose.
 */
export const MOD_T2_SCOPE = Object.freeze({
  import: true,
  inspectors: false,
  projectTooling: false,
});

export const MOD_IMPORT_ACTION = Object.freeze({
  id: MOD_IMPORT_ACTION_ID,
  contexts: Object.freeze([MOD_ACTION_CONTEXT]),
  labelId: 'semantic_action.editor.mod.import.label',
  accessibilityLabelId: 'semantic_action.editor.mod.import.accessibility',
  feedback: 'local',
  bindings: Object.freeze([
    Object.freeze({
      type: 'keyboard',
      code: 'KeyI',
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    }),
    null,
  ]),
});

export function registerModActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('MOD action registration requires a semantic action registry');
  }
  const openImport = typeof options.openImport === 'function' ? options.openImport : null;
  registry.register(MOD_IMPORT_ACTION, (activation) => {
    if (!openImport) return false;
    return openImport(activation) !== false;
  });
  return registry;
}

export function createModActionRegistry(options = {}) {
  return registerModActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

/** Install the product keyboard adapter without making the registry global. */
export function installModActionKeyboard({ target, modeShell, modActions } = {}) {
  if (!target || typeof target.addEventListener !== 'function') {
    throw new TypeError('MOD keyboard installation requires an event target');
  }
  if (!modeShell || typeof modeShell.getCurrentMode !== 'function') {
    throw new TypeError('MOD keyboard installation requires the editor mode shell');
  }
  if (!modActions || typeof modActions.dispatchKeyboardEvent !== 'function') {
    throw new TypeError('MOD keyboard installation requires the mounted MOD actions');
  }
  const onKeydown = (event) => {
    if (modeShell.getCurrentMode() !== 'MOD') return;
    modActions.dispatchKeyboardEvent(event);
  };
  target.addEventListener('keydown', onKeydown);
  return () => target.removeEventListener('keydown', onKeydown);
}
