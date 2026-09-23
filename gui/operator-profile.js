/**
 * One portable, private operator profile (issue #1279).
 *
 * The profile is presentation/input data only.  It contains no transport
 * identity, player name, Station ownership, session state, save catalogue or
 * deterministic simulation field, and this module exposes no wire-message
 * builder.  Import is prepared and fully validated before a caller persists or
 * applies it, so a refusal cannot leave half a binding profile live.
 */

import {
  ACCESSIBILITY_PROFILE_KEY,
  emptyAccessibilityProfile,
  normalizeAccessibilityProfile,
} from './accessibility-profile.js';
import { normalizePrivateAudio, legacyPrivateMaster, LEGACY_PRIVATE_MASTER_KEY } from './private-audio-preferences.js';
import { defaultWorkshopLayout, normalizeWorkshopLayout } from './workshop-layout-model.js';
import { defaultWorkshopTestLayout, normalizeWorkshopTestLayout } from './workshop-test-layout-model.js';
import { defaultLiveLayout, normalizeLiveLayout, LIVE_LAYOUT_VERSION } from './live-layout-model.js';

export const OPERATOR_PROFILE_KIND = 'project-phoenix/operator-profile';
export const OPERATOR_PROFILE_VERSION = 1;
export const OPERATOR_PROFILE_KEY = 'phoenix-operator-profile-v1';
export const OPERATOR_PROFILE_FILENAME = 'phoenix-operator-profile.json';

export const FEEDBACK_PREFERENCE_DEFAULTS = Object.freeze({
  vibration: true,
  semanticCues: true,
});

export const GM_CONFIRMATION_MODES = Object.freeze([
  'immediate',
  'confirm',
  'confirm-preview',
]);

const GM_CONFIRMATION_MODE_SET = new Set(GM_CONFIRMATION_MODES);
const PROFILE_ID = /^[a-z0-9][a-z0-9._-]{0,127}$/;
const MAX_PROFILE_BYTES = 1024 * 1024;
const MAX_PROFILE_ENTRIES = 512;
const MAX_GAMEPAD_SLOT = 15;
const CURRENT_FIELDS = new Set([
  'kind', 'version', 'accessibility', 'bindings', 'gamepad', 'feedback',
  'gmConfirmations', 'audio', 'authoringLayout', 'testLayout', 'liveLayout', 'previousLiveLayout', 'gmDensity',
]);

function ownRecord(value) {
  return value != null && typeof value === 'object' && !Array.isArray(value);
}

function own(value, key) {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function define(out, key, value) {
  Object.defineProperty(out, key, {
    value,
    enumerable: true,
    configurable: true,
    writable: true,
  });
}

function record() {
  return Object.create(null);
}

function diagnostic(code, values = {}) {
  return Object.freeze({ code, ...values });
}

function copyBinding(binding) {
  if (binding == null) return null;
  if (!ownRecord(binding)) return null;
  if (binding.type === 'gamepad') {
    const out = {
      type: 'gamepad',
      input: binding.input,
      control: binding.control,
    };
    if (binding.direction != null) out.direction = binding.direction;
    if (binding.threshold != null) out.threshold = binding.threshold;
    return out;
  }
  return {
    type: 'keyboard',
    code: binding.code,
    ctrlKey: binding.ctrlKey === true,
    shiftKey: binding.shiftKey === true,
    altKey: binding.altKey === true,
    metaKey: binding.metaKey === true,
  };
}

function copyBindings(source) {
  const out = record();
  if (!ownRecord(source)) return out;
  for (const id of Object.keys(source).slice(0, MAX_PROFILE_ENTRIES)) {
    if (!PROFILE_ID.test(id) || !Array.isArray(source[id])) continue;
    define(out, id, Array.from({ length: 2 }, (_, slot) => copyBinding(source[id][slot])));
  }
  return out;
}

function copyTuning(source) {
  const out = record();
  if (!ownRecord(source)) return out;
  for (const id of Object.keys(source).slice(0, MAX_PROFILE_ENTRIES)) {
    if (!PROFILE_ID.test(id) || !ownRecord(source[id])) continue;
    define(out, id, {
      deadzone: Number(source[id].deadzone),
      inverted: source[id].inverted === true,
    });
  }
  return out;
}

function normalizePreferredSlot(value, diagnostics) {
  if (value == null) return null;
  if (typeof value === 'number' && Number.isInteger(value)
      && value >= 0 && value <= MAX_GAMEPAD_SLOT) return value;
  diagnostics.push(diagnostic('gamepad-preference-normalized'));
  return null;
}

/** A device description, never a session slot or transport identity. */
export function normalizePreferredDevice(value) {
  if (!ownRecord(value) || typeof value.id !== 'string' || !value.id.trim()
      || value.id.length > 512 || value.mapping !== 'standard') return null;
  return { id: value.id, mapping: 'standard' };
}

function normalizeFeedback(value, diagnostics) {
  if (!ownRecord(value)) {
    if (value != null) diagnostics.push(diagnostic('feedback-normalized'));
    return { ...FEEDBACK_PREFERENCE_DEFAULTS };
  }
  const out = { ...FEEDBACK_PREFERENCE_DEFAULTS };
  for (const key of Object.keys(FEEDBACK_PREFERENCE_DEFAULTS)) {
    if (!own(value, key)) continue;
    if (typeof value[key] === 'boolean') out[key] = value[key];
    else diagnostics.push(diagnostic('feedback-normalized'));
  }
  return out;
}

function normalizeConfirmations(value, diagnostics) {
  const out = record();
  if (!ownRecord(value)) {
    if (value != null) diagnostics.push(diagnostic('gm-confirmations-normalized'));
    return out;
  }
  const keys = Object.keys(value);
  if (keys.length > MAX_PROFILE_ENTRIES) {
    diagnostics.push(diagnostic('gm-confirmations-truncated'));
  }
  for (const id of keys.slice(0, MAX_PROFILE_ENTRIES)) {
    if (!PROFILE_ID.test(id) || !GM_CONFIRMATION_MODE_SET.has(value[id])) {
      diagnostics.push(diagnostic('gm-confirmation-ignored'));
      continue;
    }
    define(out, id, value[id]);
  }
  return out;
}

function registryDefaults(registry) {
  return {
    bindings: registry && typeof registry.bindingProfile === 'function'
      ? registry.bindingProfile() : {},
    tuning: registry && typeof registry.tuningProfile === 'function'
      ? registry.tuningProfile() : {},
  };
}

/** A fresh profile uses every currently authored action default. */
export function createDefaultOperatorProfile(registry = null) {
  const defaults = registryDefaults(registry);
  return {
    kind: OPERATOR_PROFILE_KIND,
    version: OPERATOR_PROFILE_VERSION,
    accessibility: emptyAccessibilityProfile(),
    bindings: copyBindings(defaults.bindings),
    gamepad: {
      preferredSlot: null,
      preferredDevice: null,
      hideTouchControls: true,
      tuning: copyTuning(defaults.tuning),
    },
    feedback: { ...FEEDBACK_PREFERENCE_DEFAULTS },
    audio: normalizePrivateAudio(),
    gmConfirmations: record(),
    authoringLayout: defaultWorkshopLayout(),
    testLayout: defaultWorkshopTestLayout(),
    liveLayout: defaultLiveLayout(),
    previousLiveLayout: null, gmDensity: 'compact',
  };
}

/**
 * Snapshot only the approved private fields from live client state.  This
 * whitelist is also the export privacy boundary.
 */
export function createOperatorProfileSnapshot({
  accessibility,
  bindings,
  preferredGamepadSlot,
  preferredGamepadDevice,
  hideTouchControls = true,
  tuning,
  feedback,
  audio,
  gmConfirmations,
  authoringLayout,
  testLayout,
  liveLayout,
  previousLiveLayout, gmDensity,
} = {}) {
  const diagnostics = [];
  return {
    kind: OPERATOR_PROFILE_KIND,
    version: OPERATOR_PROFILE_VERSION,
    accessibility: normalizeAccessibilityProfile(accessibility),
    bindings: copyBindings(bindings),
    gamepad: {
      preferredSlot: normalizePreferredSlot(preferredGamepadSlot, diagnostics),
      preferredDevice: normalizePreferredDevice(preferredGamepadDevice),
      hideTouchControls: hideTouchControls !== false,
      tuning: copyTuning(tuning),
    },
    feedback: normalizeFeedback(feedback, diagnostics),
    audio: normalizePrivateAudio(audio),
    gmConfirmations: normalizeConfirmations(gmConfirmations, diagnostics),
    authoringLayout: normalizeWorkshopLayout(authoringLayout),
    testLayout: normalizeWorkshopTestLayout(testLayout),
    liveLayout: normalizeLiveLayout(liveLayout),
    previousLiveLayout: previousLiveLayout ? normalizeLiveLayout({ ...previousLiveLayout, version: LIVE_LAYOUT_VERSION }) : null,
    gmDensity: gmDensity === 'touch' ? 'touch' : 'compact',
  };
}

function rejected(code, registry) {
  return {
    status: 'rejected',
    code,
    diagnostics: [diagnostic(code)],
    profile: createDefaultOperatorProfile(registry),
  };
}

/**
 * Parse and validate JSON without mutating storage, Accessibility or bindings.
 * The direct Accessibility-only v1 shape is the one supported migration.
 */
export function prepareOperatorProfileImport(text, { registry } = {}) {
  if (typeof text !== 'string' || new TextEncoder().encode(text).length > MAX_PROFILE_BYTES) {
    return rejected('profile-too-large', registry);
  }
  let raw;
  try {
    raw = JSON.parse(text);
  } catch (_) {
    return rejected('profile-json', registry);
  }
  if (!ownRecord(raw)) return rejected('profile-shape', registry);

  const legacy = !own(raw, 'version') && !own(raw, 'kind')
    && (own(raw, 'presentation') || own(raw, 'assistance'));
  if (!legacy) {
    if (raw.kind !== OPERATOR_PROFILE_KIND) return rejected('profile-kind', registry);
    if (raw.version !== OPERATOR_PROFILE_VERSION) {
      return rejected('profile-version', registry);
    }
  }

  if (!registry || typeof registry.validateProfile !== 'function') {
    return rejected('profile-registry-unavailable', registry);
  }

  const diagnostics = [];
  if (!legacy) {
    const ignored = Object.keys(raw).filter((key) => !CURRENT_FIELDS.has(key));
    if (ignored.length > 0) {
      diagnostics.push(diagnostic('private-or-unsupported-fields-ignored', {
        count: ignored.length,
      }));
    }
  }

  const defaults = createDefaultOperatorProfile(registry);
  const rawAccessibility = legacy ? raw : raw.accessibility;
  const accessibility = normalizeAccessibilityProfile(rawAccessibility);
  try {
    if (JSON.stringify(accessibility) !== JSON.stringify(rawAccessibility)) {
      diagnostics.push(diagnostic('accessibility-normalized'));
    }
  } catch (_) {
    diagnostics.push(diagnostic('accessibility-normalized'));
  }
  const rawBindings = legacy ? defaults.bindings : (raw.bindings ?? defaults.bindings);
  const rawGamepad = !legacy && ownRecord(raw.gamepad) ? raw.gamepad : {};
  const rawTuning = legacy ? defaults.gamepad.tuning
    : (rawGamepad.tuning ?? defaults.gamepad.tuning);
  const controls = registry.validateProfile({
    bindings: rawBindings,
    tuning: rawTuning,
  });
  if (controls.status !== 'valid') {
    return {
      status: 'rejected',
      code: controls.code || 'profile-controls',
      diagnostics: [diagnostic(controls.code || 'profile-controls')],
      profile: defaults,
    };
  }
  if (controls.ignoredBindings.length || controls.ignoredTuning.length) {
    diagnostics.push(diagnostic('future-actions-ignored', {
      count: controls.ignoredBindings.length + controls.ignoredTuning.length,
    }));
  }
  if (controls.reconciledDefaults.length > 0) {
    diagnostics.push(diagnostic('new-action-defaults-cleared', {
      count: controls.reconciledDefaults.length,
    }));
  }

  const profile = {
    kind: OPERATOR_PROFILE_KIND,
    version: OPERATOR_PROFILE_VERSION,
    accessibility,
    bindings: copyBindings(controls.profile.bindings),
    gamepad: {
      preferredSlot: normalizePreferredSlot(rawGamepad.preferredSlot, diagnostics),
      preferredDevice: normalizePreferredDevice(rawGamepad.preferredDevice),
      hideTouchControls: rawGamepad.hideTouchControls !== false,
      tuning: copyTuning(controls.profile.tuning),
    },
    feedback: normalizeFeedback(legacy ? null : raw.feedback, diagnostics),
    audio: normalizePrivateAudio(legacy ? null : raw.audio),
    gmConfirmations: normalizeConfirmations(
      legacy ? null : raw.gmConfirmations,
      diagnostics,
    ),
    authoringLayout: normalizeWorkshopLayout(legacy ? null : raw.authoringLayout),
    testLayout: normalizeWorkshopTestLayout(legacy ? null : raw.testLayout),
    liveLayout: normalizeLiveLayout(legacy ? null : raw.liveLayout),
    previousLiveLayout: (raw.previousLiveLayout || (raw.liveLayout?.version < LIVE_LAYOUT_VERSION && raw.liveLayout))
      ? normalizeLiveLayout({ ...(raw.previousLiveLayout || raw.liveLayout), version: LIVE_LAYOUT_VERSION }) : null,
    gmDensity: raw.gmDensity === 'touch' ? 'touch' : 'compact',
  };
  return {
    status: legacy ? 'migrated' : 'imported',
    code: legacy ? 'accessibility-v1-migrated' : 'profile-imported',
    diagnostics,
    profile,
  };
}

/** Apply a previously prepared profile to the registry in one commit. */
export function applyOperatorProfile(profile, registry) {
  if (!registry || typeof registry.replaceProfile !== 'function') {
    return { status: 'rejected', code: 'profile-registry-unavailable' };
  }
  const result = registry.replaceProfile({
    bindings: profile && profile.bindings,
    tuning: profile && profile.gamepad && profile.gamepad.tuning,
  });
  return result.status === 'applied'
    ? { status: 'applied', profile }
    : { status: 'rejected', code: result.code || 'profile-controls' };
}

/** Stable, human-inspectable JSON containing only approved fields. */
export function serializeOperatorProfile(profile) {
  const safe = createOperatorProfileSnapshot({
    accessibility: profile && profile.accessibility,
    bindings: profile && profile.bindings,
    preferredGamepadSlot: profile && profile.gamepad && profile.gamepad.preferredSlot,
    preferredGamepadDevice: profile?.gamepad?.preferredDevice,
    hideTouchControls: profile?.gamepad?.hideTouchControls,
    tuning: profile && profile.gamepad && profile.gamepad.tuning,
    feedback: profile && profile.feedback,
    audio: profile && profile.audio,
    gmConfirmations: profile && profile.gmConfirmations,
    authoringLayout: profile && profile.authoringLayout,
    testLayout: profile && profile.testLayout,
    liveLayout: profile && profile.liveLayout,
    previousLiveLayout: profile?.previousLiveLayout, gmDensity: profile?.gmDensity,
  });
  return JSON.stringify(safe, null, 2) + '\n';
}

/** Persist current-version JSON and report storage failure explicitly. */
export function saveOperatorProfile(storage, profile, key = OPERATOR_PROFILE_KEY) {
  try {
    if (!storage || typeof storage.setItem !== 'function') {
      return { status: 'rejected', code: 'profile-storage-write' };
    }
    storage.setItem(key, serializeOperatorProfile(profile));
    return { status: 'saved' };
  } catch (_) {
    return { status: 'rejected', code: 'profile-storage-write' };
  }
}

/**
 * Load current storage, or migrate the old Accessibility-only key when and
 * only when the current key is absent.  A corrupt current record deliberately
 * does not fall back: that would resurrect stale choices behind the user's
 * explicit current profile.
 */
export function loadOperatorProfile(storage, { registry } = {}) {
  const fallback = createDefaultOperatorProfile(registry);
  let current;
  try {
    current = storage && typeof storage.getItem === 'function'
      ? storage.getItem(OPERATOR_PROFILE_KEY) : null;
  } catch (_) {
    return {
      status: 'rejected', code: 'profile-storage-read',
      diagnostics: [diagnostic('profile-storage-read')], profile: fallback,
    };
  }
  const migrateMaster = (result, persist = false) => {
    if (result.status === 'rejected') return result;
    const level = legacyPrivateMaster(storage);
    if (level === null && !persist) return result;
    if (level !== null) result.profile.audio.mix.master.level = level;
    const saved = saveOperatorProfile(storage, result.profile);
    if (saved.status === 'saved') {
      if (level !== null) {
        try { storage.removeItem?.(LEGACY_PRIVATE_MASTER_KEY); } catch (_) { /* replacement already persisted */ }
      }
    } else result.diagnostics.push(diagnostic(saved.code));
    return result;
  };
  if (current != null) {
    const result = prepareOperatorProfileImport(current, { registry });
    let hasAudio = true;
    let resetLayout = false;
    try {
      const raw = JSON.parse(current);
      hasAudio = own(raw, 'audio');
      resetLayout = Number.isInteger(raw.liveLayout?.version) && raw.liveLayout.version < LIVE_LAYOUT_VERSION;
    } catch (_) { /* rejected above */ }
    if (resetLayout) return migrateMaster(result, true);
    return hasAudio ? result : migrateMaster(result);
  }

  let legacy;
  try {
    legacy = storage && typeof storage.getItem === 'function'
      ? storage.getItem(ACCESSIBILITY_PROFILE_KEY) : null;
  } catch (_) {
    return {
      status: 'rejected', code: 'profile-storage-read',
      diagnostics: [diagnostic('profile-storage-read')], profile: fallback,
    };
  }
  if (legacy == null) {
    return migrateMaster({ status: 'default', code: 'profile-default', diagnostics: [], profile: fallback });
  }
  const migrated = prepareOperatorProfileImport(legacy, { registry });
  if (migrated.status === 'migrated') {
    return migrateMaster(migrated, true);
  }
  return {
    ...migrated,
    code: 'legacy-profile-corrupt',
    diagnostics: [diagnostic('legacy-profile-corrupt')],
    profile: fallback,
  };
}

if (typeof window !== 'undefined') {
  window.OperatorProfile = Object.freeze({
    applyOperatorProfile,
    createDefaultOperatorProfile,
    createOperatorProfileSnapshot,
    loadOperatorProfile,
    prepareOperatorProfileImport,
    saveOperatorProfile,
    serializeOperatorProfile,
    key: OPERATOR_PROFILE_KEY,
    filename: OPERATOR_PROFILE_FILENAME,
  });
}
