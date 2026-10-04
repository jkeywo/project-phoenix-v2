/**
 * Own one Station client's portable operator profile and its live application.
 * Browser clients and native panes use this same lifecycle. The schema and
 * capability projection remain in operator-profile and operator-surface-adapter.
 * Collaborators are read live because audio and Console mounts arrive at boot
 * before the gamepad runtime, and native storage can arrive after all of them.
 */
import {
  FEEDBACK_PREFERENCE_DEFAULTS,
  loadOperatorProfile,
  prepareOperatorProfileImport,
  saveOperatorProfile,
  serializeOperatorProfile,
} from './operator-profile.js';
import { applyOperatorProfileToSurface } from './operator-surface-adapter.js';

export function createClientOperatorProfile({
  registry,
  getStorage,
  surfaceRoot = globalThis,
  accessibility = {},
  getConsoles = () => null,
  getAudio = () => null,
  refreshGamepadPresentation = () => {},
} = {}) {
  let profile = null;
  let capabilities = null;
  let feedback = { ...FEEDBACK_PREFERENCE_DEFAULTS };
  let gamepad = null;

  // Readers may inspect this record, but only this lifecycle changes it.
  function state() { return { profile, capabilities, feedback }; }

  function snapshot() {
    if (!profile) return null;
    // Keep the complete canonical record. Rebuilding a second field whitelist
    // here used to discard inactive Test layouts, Live backups and GM density.
    // The schema's serializer remains the export/persistence privacy filter.
    profile = {
      ...profile,
      accessibility: capabilities?.accessibility
        ? (accessibility.read?.() ?? profile.accessibility) : profile.accessibility,
      bindings: registry.bindingProfile(),
      gamepad: {
        ...profile.gamepad,
        preferredDevice: gamepad?.state().preferredDevice ?? profile.gamepad.preferredDevice,
        tuning: registry.tuningProfile(),
      },
    };
    return profile;
  }

  function persist() {
    const current = snapshot();
    return current ? saveOperatorProfile(getStorage(), current)
      : { status: 'rejected', code: 'profile-storage-write' };
  }

  function restoreGamepad() {
    if (profile) gamepad?.restoreDevice(profile.gamepad.preferredDevice);
  }

  function apply(prepared) {
    if (!prepared?.profile || !registry) {
      return { status: 'rejected', code: 'profile-registry-unavailable' };
    }
    const applied = applyOperatorProfileToSurface(prepared.profile, registry, { root: surfaceRoot });
    if (applied.status !== 'applied') return applied;
    profile = applied.portableProfile;
    capabilities = applied.capabilities;
    feedback = applied.active.feedback;
    getAudio()?.reload();
    if (applied.active.accessibility) accessibility.write?.(applied.active.accessibility);
    restoreGamepad();
    getConsoles()?.refreshBindings();
    gamepad?.neutralize();
    accessibility.apply?.();
    try { accessibility.assistanceChanged?.(); } catch (_) { /* best-effort eligibility */ }
    return prepared;
  }

  function load() {
    // Rejected storage still supplies authored defaults. Do not resurrect a
    // legacy record, or persist defaults over a native profile still loading.
    return apply(loadOperatorProfile(getStorage(), { registry }));
  }

  function importJson(text) {
    const prepared = prepareOperatorProfileImport(text, { registry });
    if (prepared.status === 'rejected') return prepared;
    // Import alone is persist-first: storage refusal changes no live owner.
    const saved = saveOperatorProfile(getStorage(), prepared.profile);
    return saved.status === 'saved' ? apply(prepared) : saved;
  }

  function exportJson() {
    const current = snapshot();
    return current ? serializeOperatorProfile(current) : null;
  }

  function changeBindings(change) {
    const result = change();
    if (result.status === 'applied') {
      getConsoles()?.refreshBindings();
      gamepad?.neutralize();
      // Ordinary settings remain live even when private storage is unavailable.
      persist();
    }
    return result;
  }

  function setTuning(actionId, tuning) {
    const result = registry.setTuning(actionId, tuning);
    if (result.status === 'applied') {
      gamepad?.neutralize();
      persist();
    }
    return result;
  }

  function selectGamepad(index) {
    if (!gamepad) return { status: 'unavailable' };
    const result = gamepad.select(index);
    if (profile && (result.status === 'selected' || result.status === 'none')) {
      profile = { ...profile, gamepad: { ...profile.gamepad,
        preferredSlot: result.status === 'selected' ? result.index : null,
        preferredDevice: gamepad.state().preferredDevice,
      } };
      persist();
    }
    return result;
  }

  function setHideTouchControls(hide) {
    if (!profile) return { status: 'unavailable' };
    profile = { ...profile, gamepad: { ...profile.gamepad, hideTouchControls: hide } };
    const result = persist();
    refreshGamepadPresentation();
    return result;
  }

  function saveAudio(audio) {
    if (!profile) return { status: 'unavailable' };
    profile = { ...profile, audio };
    // Audio already applies live and reports the returned persistence status.
    return persist();
  }

  function attachGamepad(runtime) {
    gamepad = runtime;
    restoreGamepad();
  }

  return {
    state, load, importJson, exportJson, persist, attachGamepad,
    setBinding: (...args) => changeBindings(() => registry.setBinding(...args)),
    resetAction: (id) => changeBindings(() => registry.resetAction(id)),
    resetAllBindings: () => changeBindings(() => registry.resetAllBindings()),
    setTuning, selectGamepad, setHideTouchControls, saveAudio,
  };
}

if (typeof window !== 'undefined') window.createClientOperatorProfile = createClientOperatorProfile;