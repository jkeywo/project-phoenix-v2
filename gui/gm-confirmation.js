/** Private GM confirmation policy. This module never creates a game command. */
import { createFocusTrap } from './focus-trap.js';
import { createSemanticActionRegistry } from './semantic-action-registry.js';
import { createClientSemanticActionRegistry } from './client-semantic-actions.js';
import {
  GM_CONFIRMATION_MODES,
  loadOperatorProfile,
  prepareOperatorProfileImport,
  saveOperatorProfile,
  serializeOperatorProfile,
} from './operator-profile.js';

const category = (id, defaultMode) => Object.freeze({
  id, defaultMode, labelId: `settings.gm.confirmation.${id}`,
});

/** Defaults ratified in PRD #1273; categories are independent of control labels. */
export const GM_CONFIRMATION_CATEGORIES = Object.freeze([
  category('session.pause', 'immediate'),
  category('session.force-start', 'confirm'),
  category('event.fire', 'confirm'),
  category('event.skip', 'confirm'),
  category('event.pause', 'immediate'),
  category('station.takeover', 'immediate'),
  category('station.takeover-human', 'confirm'),
  category('station.release', 'immediate'),
  category('station.command', 'immediate'),
  category('contact.override', 'immediate'),
  category('world.spawn', 'immediate'),
  category('world.despawn', 'confirm-preview'),
  category('effect.damage', 'confirm'),
  category('effect.lethal', 'confirm-preview'),
  category('effect.heal', 'immediate'),
  category('system.disable', 'confirm'),
  category('system.restore', 'immediate'),
  category('objective.activate', 'immediate'),
  category('objective.complete', 'confirm'),
  category('objective.fail', 'confirm'),
  category('npc.directive', 'immediate'),
  category('comms.send', 'immediate'),
]);
const categories = new Map(GM_CONFIRMATION_CATEGORIES.map((entry) => [entry.id, entry]));

/** Every typed GM action declares the categories its actual intent can select. */
export const GM_ACTION_CONFIRMATION_METADATA = Object.freeze(Object.fromEntries(Object.entries({
  SetSessionPaused: ['session.pause'],
  SetStationPuppet: ['station.takeover', 'station.takeover-human', 'station.release'],
  IssueStationCommand: ['station.command'],
  FireGmEvent: ['event.fire'],
  ApplyDirectEffect: ['effect.damage', 'effect.lethal', 'effect.heal'],
  SpawnPaletteEntity: ['world.spawn'],
  SetEventPaused: ['event.pause'],
  ArmGmEventSkip: ['event.skip'],
  DespawnEntity: ['world.despawn'],
  ObjectiveAction: ['objective.activate', 'objective.complete', 'objective.fail'],
  SetContactOverride: ['contact.override'],
  SetSystemDisabled: ['system.disable', 'system.restore'],
  TransmitComms: ['comms.send'],
}).map(([action, ids]) => [action, Object.freeze(ids.map(gmConfirmationMetadata))])));

/** Registration must name a real category, including its accepted default. */
export function gmConfirmationMetadata(id) {
  const value = categories.get(id);
  if (!value) throw new TypeError(`Unregistered GM confirmation category: ${id}`);
  return { confirmationCategory: id, confirmationDefault: value.defaultMode };
}

/** One page's private choices; import/export uses the existing portable schema. */
export function createGmConfirmationProfile({ storage, registry = createSemanticActionRegistry() } = {}) {
  // Retain player/editor bindings when this same portable profile is opened on
  // a GM page. Only the active registry dispatches; this catalogue validates.
  const catalogue = createClientSemanticActionRegistry();
  for (const definition of registry.list()) {
    if (!catalogue.action(definition.id)) catalogue.register(definition);
  }
  const loaded = loadOperatorProfile(storage, { registry: catalogue });
  let profile = loaded.profile;
  registry.replaceProfile({ bindings: profile.bindings, tuning: profile.gamepad.tuning });
  function snapshot() {
    return { ...profile, bindings: { ...profile.bindings, ...registry.bindingProfile() },
      gamepad: { ...profile.gamepad, tuning: { ...profile.gamepad.tuning, ...registry.tuningProfile() } } };
  }
  function mode(id) {
    const metadata = gmConfirmationMetadata(id);
    return profile.gmConfirmations[id] || metadata.confirmationDefault;
  }
  function setMode(id, value) {
    gmConfirmationMetadata(id);
    if (!GM_CONFIRMATION_MODES.includes(value)) return { status: 'rejected' };
    const next = { ...snapshot(), gmConfirmations: { ...profile.gmConfirmations, [id]: value } };
    const result = saveOperatorProfile(storage, next);
    if (result.status === 'saved') profile = next;
    return result;
  }
  function importProfile(text) {
    const prepared = prepareOperatorProfileImport(text, { registry: catalogue });
    if (prepared.status === 'rejected') return prepared;
    const result = saveOperatorProfile(storage, prepared.profile);
    if (result.status !== 'saved') return result;
    profile = prepared.profile;
    registry.replaceProfile({ bindings: profile.bindings, tuning: profile.gamepad.tuning });
    return prepared;
  }
  return { mode, setMode, importProfile,
    exportProfile: () => serializeOperatorProfile(snapshot()),
    initialStatus: loaded.status,
  };
}

/**
 * Ask before invoking an ordinary panel's submit lifecycle. Nothing is Pending
 * before acceptance. The callback retains the original target/amount; changing
 * the projection can change the preview, never the operator's captured intent.
 */
export function createGmConfirmationController({
  doc = globalThis.document, t = (id) => id, profile,
} = {}) {
  let intent = null;
  const overlay = doc.createElement('div');
  overlay.id = 'gm-action-confirmation';
  overlay.className = 'gm-action-confirmation';
  overlay.hidden = true;
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-labelledby', 'gm-action-confirmation-title');
  const card = doc.createElement('div');
  const title = doc.createElement('h2');
  title.id = 'gm-action-confirmation-title';
  title.textContent = t('settings.gm.confirmation.title');
  const description = doc.createElement('p');
  description.dataset.confirmationDescription = '';
  const preview = doc.createElement('p');
  preview.dataset.confirmationPreview = '';
  preview.setAttribute('aria-live', 'polite');
  const cancel = doc.createElement('button');
  cancel.type = 'button';
  cancel.dataset.confirmationCancel = '';
  cancel.textContent = t('settings.gm.confirmation.cancel');
  const accept = doc.createElement('button');
  accept.type = 'button';
  accept.dataset.confirmationAccept = '';
  accept.textContent = t('settings.gm.confirmation.accept');
  card.append(title, description, preview, cancel, accept);
  overlay.append(card);
  doc.body.append(overlay);
  const trap = createFocusTrap(overlay, { doc, onEscape: close });
  function close() {
    const cancelled = intent;
    intent = null;
    trap.release();
    overlay.hidden = true;
    cancelled?.onCancel?.();
  }
  function refresh() {
    if (!intent) return;
    // A changing prediction is advisory. Canonical admission still resolves at
    // the apply tick, including a target that disappeared while this was open.
    preview.hidden = intent.mode !== 'confirm-preview';
    preview.textContent = preview.hidden ? '' : String(intent.preview?.() || '');
  }
  function request(value) {
    gmConfirmationMetadata(value?.category);
    if (typeof value.accept !== 'function') throw new TypeError('GM confirmation requires an action');
    // Releasing a held control completes its input lifecycle. It must reach
    // ordinary admission even while another action owns the modal. Retire any
    // unsent value for this same control so acceptance cannot revive it later.
    if (value.controlRelease === true && value.key) {
      if (intent?.key === value.key) close();
      return value.accept() !== false;
    }
    if (intent) {
      // Continuous console updates retain only the latest intent for this one
      // target/control. In particular blur/key-up can replace thrust with zero
      // before acceptance. Nothing queues behind the modal.
      if (!value.key || value.key !== intent.key) return false;
      const previous = intent;
      intent = { ...value, mode: previous.mode };
      previous.onCancel?.();
      description.textContent = value.description;
      refresh();
      return true;
    }
    const mode = profile.mode(value.category);
    if (mode === 'immediate') return value.accept() !== false;
    intent = { ...value, mode };
    overlay.dataset.category = value.category;
    overlay.dataset.mode = mode;
    description.textContent = value.description;
    refresh();
    overlay.hidden = false;
    trap.activate();
    cancel.focus();
    return true;
  }
  cancel.addEventListener('click', close);
  accept.addEventListener('click', () => {
    const chosen = intent;
    if (!chosen) return;
    intent = null;
    close();
    chosen.accept();
  });
  overlay.addEventListener('click', (event) => { if (event.target === overlay) close(); });
  return { request, refresh, cancel: close, isOpen: () => !!intent,
    destroy: () => { close(); overlay.remove(); },
  };
}
