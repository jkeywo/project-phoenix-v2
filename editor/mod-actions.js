/**
 * Semantic actions for the existing browser MOD editor (issue #1321).
 *
 * This is presentation/input metadata only. The adapters enter the existing
 * import, validation and export paths; `mod-mode-view.js` still owns their DOM,
 * archive parsing, validation and download effects.
 */

import { createClientSemanticActionRegistry } from '../gui/client-semantic-actions.js';
import {
  MOD_ACTION_CONTEXT,
  MOD_ACTIONS,
  MOD_EXPORT_ACTION,
  MOD_EXPORT_ACTION_ID,
  MOD_IMPORT_ACTION,
  MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION,
  MOD_VALIDATE_ACTION_ID,
} from '../gui/editor-mod-actions.js';

export {
  MOD_ACTION_CONTEXT,
  MOD_ACTIONS,
  MOD_EXPORT_ACTION,
  MOD_EXPORT_ACTION_ID,
  MOD_IMPORT_ACTION,
  MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION,
  MOD_VALIDATE_ACTION_ID,
};

/**
 * T2 deliberately keeps this one-pack edit/validation/export tracer bounded.
 * The object is exported so the M6 boundary is machine-testable rather than an
 * aspiration hidden in prose.
 */
export const MOD_T2_SCOPE = Object.freeze({
  import: true,
  memberSourceEdit: true,
  validate: true,
  export: true,
  inspectors: false,
  projectTooling: false,
  modelTooling: false,
  workshopRedesign: false,
});

export function registerModActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('MOD action registration requires a semantic action registry');
  }
  const openImport = typeof options.openImport === 'function' ? options.openImport : null;
  const validatePack = typeof options.validatePack === 'function' ? options.validatePack : null;
  const exportPack = typeof options.exportPack === 'function' ? options.exportPack : null;
  const adapters = {
    [MOD_IMPORT_ACTION_ID]: (activation) => (
      openImport ? openImport(activation) !== false : false
    ),
    [MOD_VALIDATE_ACTION_ID]: (activation) => (
      validatePack ? validatePack(activation) !== false : false
    ),
    [MOD_EXPORT_ACTION_ID]: (activation) => (
      exportPack ? exportPack(activation) !== false : false
    ),
  };
  for (const action of MOD_ACTIONS) registry.register(action, adapters[action.id]);
  return registry;
}

export function createModActionRegistry(options = {}) {
  const adapters = {
    [MOD_IMPORT_ACTION_ID]: (activation) => (
      typeof options.openImport === 'function' ? options.openImport(activation) !== false : false
    ),
    [MOD_VALIDATE_ACTION_ID]: (activation) => (
      typeof options.validatePack === 'function' ? options.validatePack(activation) !== false : false
    ),
    [MOD_EXPORT_ACTION_ID]: (activation) => (
      typeof options.exportPack === 'function' ? options.exportPack(activation) !== false : false
    ),
  };
  return createClientSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
    adapters,
  });
}

function isEditableEventTarget(target) {
  const tagName = String(target?.tagName || '').toLowerCase();
  return target?.isContentEditable === true
    || tagName === 'input'
    || tagName === 'textarea'
    || tagName === 'select';
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
    // Plain-letter defaults must remain typeable in metadata and member source
    // editors. Controls remain keyboard-operable whenever focus is outside an
    // editable field; binding captures own and stop their key events themselves.
    if (isEditableEventTarget(event?.target)) return;
    modActions.dispatchKeyboardEvent(event);
  };
  target.addEventListener('keydown', onKeydown);
  return () => target.removeEventListener('keydown', onKeydown);
}
