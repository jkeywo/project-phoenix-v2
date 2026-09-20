/**
 * Semantic actions for Workshop's browser/native archive workflow.
 *
 * This is presentation/input metadata only. The adapters enter the existing
 * import, validation and export paths; `gui/workshop-authoring.js` owns their
 * controls while the archive modules remain DOM-free.
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
