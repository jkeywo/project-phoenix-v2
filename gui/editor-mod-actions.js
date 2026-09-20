/** Portable semantic-action definitions retained by Workshop and the shared
 * private operator profile. This module owns no shell, DOM or filesystem. */
export const MOD_ACTION_CONTEXT = 'editor.mod';
export const MOD_IMPORT_ACTION_ID = 'editor.mod.import';
export const MOD_VALIDATE_ACTION_ID = 'editor.mod.validate';
export const MOD_EXPORT_ACTION_ID = 'editor.mod.export';

function keyboard(code) {
  return Object.freeze({ type: 'keyboard', code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false });
}

function action(id, labelId, accessibilityLabelId, code) {
  return Object.freeze({ id, contexts: Object.freeze([MOD_ACTION_CONTEXT]), labelId,
    accessibilityLabelId, feedback: 'local', bindings: Object.freeze([keyboard(code), null]) });
}

export const MOD_IMPORT_ACTION = action(MOD_IMPORT_ACTION_ID,
  'semantic_action.editor.mod.import.label', 'semantic_action.editor.mod.import.accessibility', 'KeyI');
export const MOD_VALIDATE_ACTION = action(MOD_VALIDATE_ACTION_ID,
  'semantic_action.editor.mod.validate.label', 'semantic_action.editor.mod.validate.accessibility', 'KeyV');
export const MOD_EXPORT_ACTION = action(MOD_EXPORT_ACTION_ID,
  'semantic_action.editor.mod.export.label', 'semantic_action.editor.mod.export.accessibility', 'KeyE');
export const MOD_ACTIONS = Object.freeze([MOD_IMPORT_ACTION, MOD_VALIDATE_ACTION, MOD_EXPORT_ACTION]);
