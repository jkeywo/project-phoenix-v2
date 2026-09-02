/**
 * Portable semantic-action definitions for the bounded T2 MOD editor.
 *
 * Definitions live with the product-wide client catalogue so the one private
 * operator profile can retain editor bindings even while a phone or native
 * pane is the surface currently saving it.  The editor-only adapters remain in
 * `editor/mod-actions.js`; this module has no editor DOM or filesystem access.
 */

export const MOD_ACTION_CONTEXT = 'editor.mod';
export const MOD_IMPORT_ACTION_ID = 'editor.mod.import';
export const MOD_VALIDATE_ACTION_ID = 'editor.mod.validate';
export const MOD_EXPORT_ACTION_ID = 'editor.mod.export';

function keyboard(code) {
  return Object.freeze({
    type: 'keyboard',
    code,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
  });
}

function action(id, labelId, accessibilityLabelId, code) {
  return Object.freeze({
    id,
    contexts: Object.freeze([MOD_ACTION_CONTEXT]),
    labelId,
    accessibilityLabelId,
    feedback: 'local',
    bindings: Object.freeze([keyboard(code), null]),
  });
}

export const MOD_IMPORT_ACTION = action(
  MOD_IMPORT_ACTION_ID,
  'semantic_action.editor.mod.import.label',
  'semantic_action.editor.mod.import.accessibility',
  'KeyI',
);

export const MOD_VALIDATE_ACTION = action(
  MOD_VALIDATE_ACTION_ID,
  'semantic_action.editor.mod.validate.label',
  'semantic_action.editor.mod.validate.accessibility',
  'KeyV',
);

export const MOD_EXPORT_ACTION = action(
  MOD_EXPORT_ACTION_ID,
  'semantic_action.editor.mod.export.label',
  'semantic_action.editor.mod.export.accessibility',
  'KeyE',
);

export const MOD_ACTIONS = Object.freeze([
  MOD_IMPORT_ACTION,
  MOD_VALIDATE_ACTION,
  MOD_EXPORT_ACTION,
]);
