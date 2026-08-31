/**
 * gui/semantic-action-registry.js — context-scoped semantic input actions.
 *
 * This layer sits above gui/action-map.js. It identifies an operator intent
 * and chooses its local adapter; it never constructs a ClientMessage and owns
 * no Station, session or command authority. The adapter still emits the
 * existing console action envelope, which follows the normal admission path.
 *
 * The module is deliberately DOM-free. KeyboardEvent-shaped objects are plain
 * data here, which keeps registration, binding normalization and dispatch
 * fully testable in Node.
 */

import {
  gamepadBindingDisplay,
  gamepadBindingsEqual,
  normalizeGamepadBinding,
} from './gamepad-input.js';

export const SEMANTIC_BINDING_SLOT_COUNT = 2;
/** Adapter result for a real local hold-source transition that emitted no
 * authoritative command. The source remains tracked for its eventual release,
 * while the provisional feedback press is cancelled instead of timing out. */
export const SEMANTIC_HANDLED_WITHOUT_FEEDBACK = Symbol('semantic-handled-without-feedback');
export const CONTINUOUS_DEADZONE_MAX = 0.95;

const MODIFIER_KEYS = ['ctrlKey', 'shiftKey', 'altKey', 'metaKey'];
const MODIFIER_CODE_FAMILIES = Object.freeze({
  ControlLeft: 'Control', ControlRight: 'Control',
  ShiftLeft: 'Shift', ShiftRight: 'Shift',
  AltLeft: 'Alt', AltRight: 'Alt',
  MetaLeft: 'Meta', MetaRight: 'Meta',
});
const MODIFIER_CODE_FLAGS = Object.freeze({
  Control: 'ctrlKey', Shift: 'shiftKey', Alt: 'altKey', Meta: 'metaKey',
});
const EDITABLE_TAGS = new Set(['INPUT', 'TEXTAREA', 'SELECT']);
const RESERVED_UNMODIFIED_CODES = new Set([
  'Escape', 'Tab',
  'F1', 'F3', 'F5', 'F6', 'F7', 'F10', 'F11', 'F12',
]);
const RESERVED_DEDICATED_BROWSER_CODES = new Set([
  'PrintScreen',
  'BrowserBack', 'BrowserForward', 'BrowserRefresh', 'BrowserHome',
  'BrowserSearch', 'BrowserFavorites', 'BrowserStop',
]);
const RESERVED_CTRL_CODES = new Set([
  // Tabs, windows, reload, location, find, print, save, open and zoom.
  'KeyR', 'KeyW', 'KeyT', 'KeyN', 'KeyL', 'KeyF', 'KeyP', 'KeyS', 'KeyO',
  'Equal', 'Minus', 'Digit0',
  // Direct tab selection.
  'Digit1', 'Digit2', 'Digit3', 'Digit4', 'Digit5',
  'Digit6', 'Digit7', 'Digit8', 'Digit9',
  // Bookmarks, history, downloads, source and browser search.
  'KeyD', 'KeyH', 'KeyJ', 'KeyU', 'KeyK', 'KeyE', 'KeyG',
]);
const RESERVED_CTRL_NAVIGATION_CODES = new Set(['F4', 'PageUp', 'PageDown']);
const RESERVED_CTRL_SHIFT_CODES = new Set([
  'KeyA',   // Search open tabs.
  'Delete', // Clear browsing data.
  'KeyB',   // Bookmark bar / manager.
  'KeyC',   // Browser inspector element picker.
  'KeyI',   // Browser developer tools.
  'KeyJ',   // Browser developer tools.
  'KeyM',   // Browser profile/window command.
  'KeyQ',   // Browser/window quit command.
]);
const RESERVED_ALT_CODES = new Set([
  // Browser menus, address/location, window switching and navigation.
  'KeyD', 'KeyE', 'KeyF', 'F4', 'Tab', 'Home', 'Space', 'Enter',
  'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown',
]);
const RESERVED_ALT_SHIFT_CODES = new Set(['KeyB', 'KeyI', 'KeyT']);
const RESERVED_STANDALONE_MODIFIER_CODES = new Set([
  // These leave the application for browser/OS menus or window management.
  // Standalone Control and Shift intentionally remain available.
  'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight',
]);

/** Normalize one keyboard binding, or the empty-slot sentinel `null`. */
export function normalizeKeyboardBinding(value) {
  if (value == null || value === '') return null;
  if (typeof value !== 'object') {
    throw new TypeError('semantic action binding must be an object or null');
  }
  if (value.type != null && value.type !== 'keyboard') {
    throw new TypeError('semantic action binding type must be keyboard');
  }
  const code = typeof value.code === 'string' ? value.code.trim() : '';
  if (!code) throw new TypeError('semantic action keyboard binding requires code');
  const ownFlag = MODIFIER_CODE_FLAGS[keyboardCodeFamily(code)] || null;
  return Object.freeze({
    type: 'keyboard',
    code,
    ctrlKey: ownFlag === 'ctrlKey' ? false : !!value.ctrlKey,
    shiftKey: ownFlag === 'shiftKey' ? false : !!value.shiftKey,
    altKey: ownFlag === 'altKey' ? false : !!value.altKey,
    metaKey: ownFlag === 'metaKey' ? false : !!value.metaKey,
  });
}

/** Normalize the keyboard/gamepad binding union, or the empty-slot sentinel. */
export function normalizeSemanticBinding(value, options = {}) {
  if (value == null || value === '') return null;
  if (options.continuous === true) {
    if (!value || value.type !== 'gamepad') {
      throw new TypeError('continuous semantic action binding must be a gamepad axis');
    }
    const binding = normalizeGamepadBinding(value, { continuous: true });
    if (!binding || binding.input !== 'axis') {
      throw new TypeError('continuous semantic action binding must be a gamepad axis');
    }
    return binding;
  }
  return value && value.type === 'gamepad'
    ? normalizeGamepadBinding(value)
    : normalizeKeyboardBinding(value);
}

/** Normalize and pad an action's binding list to exactly two slots. */
export function normalizeBindingSlots(values, options = {}) {
  if (values != null && !Array.isArray(values)) {
    throw new TypeError('semantic action bindings must be an array');
  }
  const slots = values || [];
  if (slots.length > SEMANTIC_BINDING_SLOT_COUNT) {
    throw new RangeError('semantic action has exactly two binding slots');
  }
  return Object.freeze(Array.from(
    { length: SEMANTIC_BINDING_SLOT_COUNT },
    (_, index) => normalizeSemanticBinding(slots[index], options),
  ));
}

/** Validate authored continuous output semantics independently of device tuning. */
export function normalizeContinuousDefinition(value) {
  if (value == null) return null;
  if (typeof value !== 'object') throw new TypeError('continuous action metadata must be an object');
  const min = Number(value.min);
  const max = Number(value.max);
  const neutral = Number(value.neutral);
  const cadenceMs = Number(value.cadenceMs);
  if (!Number.isFinite(min) || !Number.isFinite(max) || min >= max) {
    throw new RangeError('continuous action range must have finite min below max');
  }
  if (!Number.isFinite(neutral) || neutral <= min || neutral >= max) {
    throw new RangeError('continuous action neutral must be inside its range');
  }
  if (!Number.isFinite(cadenceMs) || cadenceMs <= 0) {
    throw new RangeError('continuous action cadence must be above zero');
  }
  return Object.freeze({ min, max, neutral, cadenceMs });
}

/** Normalize client-local axis tuning; this is serialisable but not persisted. */
export function normalizeContinuousTuning(value, fallback = {}) {
  const source = value && typeof value === 'object' ? value : fallback;
  const deadzone = Number(source.deadzone);
  if (!Number.isFinite(deadzone) || deadzone < 0 || deadzone > CONTINUOUS_DEADZONE_MAX) {
    throw new RangeError(`continuous action deadzone must be between 0 and ${CONTINUOUS_DEADZONE_MAX}`);
  }
  return Object.freeze({ deadzone, inverted: source.inverted === true });
}

/** Convert a KeyboardEvent-shaped object into a canonical binding. */
export function keyboardBindingFromEvent(event) {
  if (!event || typeof event.code !== 'string' || !event.code.trim()) return null;
  return normalizeKeyboardBinding({
    code: event.code,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    altKey: event.altKey,
    metaKey: event.metaKey,
  });
}

/** True when two canonical keyboard bindings identify the same chord. */
export function keyboardBindingsEqual(left, right) {
  if (!left || !right || left.type !== 'keyboard' || right.type !== 'keyboard') return false;
  return keyboardCodesEqual(left.code, right.code)
    && MODIFIER_KEYS.every((key) => left[key] === right[key]);
}

function keyboardCodeFamily(code) {
  return MODIFIER_CODE_FAMILIES[code] || code;
}

function keyboardCodesEqual(left, right) {
  return keyboardCodeFamily(left) === keyboardCodeFamily(right);
}

/** True when two canonical bindings identify the same logical input. */
export function semanticBindingsEqual(left, right) {
  return keyboardBindingsEqual(left, right) || gamepadBindingsEqual(left, right);
}

/**
 * Browser/OS chords that a Station must not capture from its surrounding UI.
 *
 * `KeyboardEvent.code` is used deliberately: it is the same physical-key
 * identity as ordinary semantic bindings. Shift is intentionally ignored by
 * the policy checks below, so adding it cannot turn Ctrl+R or Alt+F4 into a
 * capturable chord.
 */
export function isReservedKeyboardBinding(value) {
  const binding = normalizeKeyboardBinding(value);
  if (!binding) return false;
  const { code, ctrlKey, shiftKey, altKey, metaKey } = binding;
  if (RESERVED_UNMODIFIED_CODES.has(code)) return true;
  if (RESERVED_DEDICATED_BROWSER_CODES.has(code)) return true;
  if (RESERVED_STANDALONE_MODIFIER_CODES.has(code)) return true;
  // Meta is browser/OS chrome on macOS and the Windows/Super boundary on other
  // platforms. Treat every Meta chord as unavailable rather than maintaining
  // a necessarily incomplete platform-specific command list.
  if (metaKey) return true;
  if (ctrlKey && RESERVED_CTRL_CODES.has(code)) return true;
  if (ctrlKey && RESERVED_CTRL_NAVIGATION_CODES.has(code)) return true;
  if (ctrlKey && shiftKey && RESERVED_CTRL_SHIFT_CODES.has(code)) return true;
  if (ctrlKey && altKey && code === 'Delete') return true;
  if (altKey && RESERVED_ALT_CODES.has(code)) return true;
  if (altKey && shiftKey && RESERVED_ALT_SHIFT_CODES.has(code)) return true;
  return false;
}

/** True when an event target is editable or is actively capturing a remap. */
export function isSemanticInputTarget(target) {
  if (!target) return false;
  const tag = typeof target.tagName === 'string' ? target.tagName.toUpperCase() : '';
  if (EDITABLE_TAGS.has(tag) || target.isContentEditable === true) return true;
  if (typeof target.getAttribute === 'function'
      && target.getAttribute('data-semantic-binding-capture') != null) return true;
  if (typeof target.closest === 'function') {
    try {
      if (target.closest('[data-semantic-binding-capture]')) return true;
    } catch (_) { /* a minimal test double need not implement selector parsing */ }
  }
  return false;
}

/** Compare a canonical binding with a KeyboardEvent-shaped object. */
export function keyboardBindingMatches(binding, event) {
  if (!binding || binding.type !== 'keyboard' || !event) return false;
  if (!keyboardCodesEqual(binding.code, event.code)) return false;
  // A standalone modifier is the key being bound, not a modifier chord. Its
  // browser flag differs between keydown and keyup and between engines, so do
  // not require that one self-flag while retaining every other modifier.
  const ownFlag = MODIFIER_CODE_FLAGS[keyboardCodeFamily(binding.code)] || null;
  return MODIFIER_KEYS.every((key) => key === ownFlag || binding[key] === !!event[key]);
}

/**
 * Presentation tokens for a binding. The caller localises modifier names and
 * the separator; the physical KeyboardEvent.code remains the stable identity.
 */
export function keyboardBindingDisplay(binding) {
  const normalized = normalizeKeyboardBinding(binding);
  if (!normalized) return { modifiers: [], code: null };
  const modifiers = [];
  if (normalized.ctrlKey) modifiers.push('input.modifier.control');
  if (normalized.shiftKey) modifiers.push('input.modifier.shift');
  if (normalized.altKey) modifiers.push('input.modifier.alt');
  if (normalized.metaKey) modifiers.push('input.modifier.meta');
  let code = normalized.code;
  if (/^Key[A-Z]$/.test(code)) code = code.slice(3);
  else if (/^Digit[0-9]$/.test(code)) code = code.slice(5);
  return { modifiers, code };
}

/** Format a binding through the caller's string-table translator. */
export function formatKeyboardBinding(binding, translate) {
  const tr = typeof translate === 'function' ? translate : (id) => id;
  const display = keyboardBindingDisplay(binding);
  if (!display.code) return tr('input.binding.unassigned');
  return display.modifiers.map(tr)
    .concat(display.code)
    .join(tr('input.binding.separator'));
}

/** Format either member of the semantic binding union. */
export function formatSemanticBinding(binding, translate) {
  const tr = typeof translate === 'function' ? translate : (id) => id;
  if (!binding || binding.type !== 'gamepad') return formatKeyboardBinding(binding, tr);
  const display = gamepadBindingDisplay(binding);
  return display.labelId ? tr(display.labelId, display.values) : tr('input.binding.unassigned');
}

function normalizeDefinition(definition) {
  if (!definition || typeof definition !== 'object') {
    throw new TypeError('semantic action definition is required');
  }
  const id = typeof definition.id === 'string' ? definition.id.trim() : '';
  if (!id) throw new TypeError('semantic action id is required');
  const contexts = Array.isArray(definition.contexts)
    ? [...new Set(definition.contexts.map((value) => String(value).trim()).filter(Boolean))]
    : [];
  if (contexts.length === 0) throw new TypeError('semantic action context is required');
  const labelId = typeof definition.labelId === 'string' ? definition.labelId.trim() : '';
  const accessibilityLabelId = typeof definition.accessibilityLabelId === 'string'
    ? definition.accessibilityLabelId.trim()
    : '';
  if (!labelId || !accessibilityLabelId) {
    throw new TypeError('semantic action display and accessibility metadata are required');
  }
  const feedback = definition.feedback === 'local'
    ? 'local'
    : definition.authoritativeFeedback === true
      ? 'authoritative'
      : null;
  const continuous = normalizeContinuousDefinition(definition.continuous);
  const hold = definition.hold === true;
  if (continuous && hold) {
    throw new TypeError('continuous semantic action cannot also be a hold action');
  }
  const tuning = continuous
    ? normalizeContinuousTuning(definition.tuning)
    : null;
  return Object.freeze({
    id,
    contexts: Object.freeze(contexts),
    labelId,
    accessibilityLabelId,
    authoritativeFeedback: definition.authoritativeFeedback === true,
    feedback,
    hold,
    ...(continuous ? { continuous, tuning } : {}),
    bindings: normalizeBindingSlots(definition.bindings, { continuous: !!continuous }),
  });
}

function copyBinding(binding) {
  return binding ? { ...binding } : null;
}

function copySlots(slots) {
  return slots.map(copyBinding);
}

/** Create an isolated registry. No mutable singleton is shared across frames. */
export function createSemanticActionRegistry(options = {}) {
  const definitions = new Map();
  const adapters = new Map();
  const authoredDefaults = new Map();
  const bindings = new Map();
  const authoredTuning = new Map();
  const tuning = new Map();
  const keyboardHolds = new Map();
  const actionFeedback = options.actionFeedback || null;

  function assertActionAndSlot(id, slot) {
    if (!definitions.has(id)) throw new Error('unknown semantic action: ' + id);
    if (!Number.isInteger(slot) || slot < 0 || slot >= SEMANTIC_BINDING_SLOT_COUNT) {
      throw new RangeError('semantic action binding slot must be zero or one');
    }
  }

  function contextsOverlap(left, right) {
    return left.contexts.some((context) => right.contexts.includes(context));
  }

  function normalizeSlotsFor(id, slots) {
    const definition = definitions.get(id);
    return normalizeBindingSlots(slots, { continuous: !!(definition && definition.continuous) });
  }

  function conflictsFor(id, slot, binding, source = bindings) {
    if (!binding) return [];
    const target = definitions.get(id);
    const conflicts = [];
    for (const [otherId, otherDefinition] of definitions) {
      if (!contextsOverlap(target, otherDefinition)) continue;
      const otherSlots = source.get(otherId) || [];
      for (let otherSlot = 0; otherSlot < SEMANTIC_BINDING_SLOT_COUNT; otherSlot++) {
        if (otherId === id && otherSlot === slot) continue;
        if (semanticBindingsEqual(binding, otherSlots[otherSlot])) {
          conflicts.push({
            actionId: otherId,
            slot: otherSlot,
            labelId: otherDefinition.labelId,
            binding: copyBinding(otherSlots[otherSlot]),
          });
        }
      }
    }
    return conflicts;
  }

  function commitBinding(id, slot, binding, conflicts) {
    const nextByAction = new Map();
    const mutableSlots = (actionId) => {
      if (!nextByAction.has(actionId)) {
        nextByAction.set(actionId, copySlots(bindings.get(actionId)));
      }
      return nextByAction.get(actionId);
    };
    for (const conflict of conflicts) {
      mutableSlots(conflict.actionId)[conflict.slot] = null;
    }
    mutableSlots(id)[slot] = copyBinding(binding);
    for (const [actionId, slots] of nextByAction) {
      bindings.set(actionId, normalizeSlotsFor(actionId, slots));
    }
  }

  function register(definition, adapter) {
    const normalized = normalizeDefinition(definition);
    if (definitions.has(normalized.id)) {
      throw new Error('semantic action already registered: ' + normalized.id);
    }
    if (adapter != null && typeof adapter !== 'function') {
      throw new TypeError('semantic action adapter must be a function');
    }
    definitions.set(normalized.id, normalized);
    authoredDefaults.set(normalized.id, normalizeSlotsFor(normalized.id, normalized.bindings));
    bindings.set(normalized.id, normalizeSlotsFor(normalized.id, normalized.bindings));
    if (normalized.continuous) {
      authoredTuning.set(normalized.id, normalized.tuning);
      tuning.set(normalized.id, normalized.tuning);
    }
    for (let slot = 0; slot < SEMANTIC_BINDING_SLOT_COUNT; slot++) {
      const binding = normalized.bindings[slot];
      if (binding && binding.type === 'keyboard' && isReservedKeyboardBinding(binding)) {
        definitions.delete(normalized.id);
        authoredDefaults.delete(normalized.id);
        bindings.delete(normalized.id);
        authoredTuning.delete(normalized.id);
        tuning.delete(normalized.id);
        throw new Error('semantic action authored default is reserved: ' + normalized.id);
      }
      if (conflictsFor(normalized.id, slot, binding).length > 0) {
        definitions.delete(normalized.id);
        authoredDefaults.delete(normalized.id);
        bindings.delete(normalized.id);
        authoredTuning.delete(normalized.id);
        tuning.delete(normalized.id);
        throw new Error('semantic action authored default binding conflicts: ' + normalized.id);
      }
    }
    if (adapter) adapters.set(normalized.id, adapter);
    return action(normalized.id);
  }

  function action(id) {
    const definition = definitions.get(id);
    if (!definition) return null;
    return {
      ...definition,
      contexts: [...definition.contexts],
      bindings: copySlots(bindings.get(id)),
      ...(definition.continuous ? { tuning: { ...tuning.get(id) } } : {}),
    };
  }

  function list(context) {
    return [...definitions.keys()]
      .map(action)
      .filter((entry) => !context || entry.contexts.includes(context));
  }

  function setBinding(id, slot, value, options = {}) {
    assertActionAndSlot(id, slot);
    const binding = normalizeSemanticBinding(value, {
      continuous: !!definitions.get(id).continuous,
    });
    if (binding && binding.type === 'keyboard' && isReservedKeyboardBinding(binding)) {
      return { status: 'reserved', actionId: id, slot, binding: copyBinding(binding) };
    }
    const conflicts = conflictsFor(id, slot, binding);
    if (conflicts.length > 0 && options.replace !== true) {
      return {
        status: 'conflict', actionId: id, slot,
        binding: copyBinding(binding), conflicts,
      };
    }
    commitBinding(id, slot, binding, conflicts);
    return {
      status: 'applied', actionId: id, slot, action: action(id),
      cleared: conflicts,
    };
  }

  /** Restore both authored slots for one action, clearing overlapping remaps. */
  function resetAction(id) {
    if (!definitions.has(id)) throw new Error('unknown semantic action: ' + id);
    const defaults = copySlots(authoredDefaults.get(id));
    const next = new Map([...bindings].map(([actionId, slots]) => [actionId, copySlots(slots)]));
    next.set(id, copySlots(defaults));
    const cleared = [];
    for (let slot = 0; slot < SEMANTIC_BINDING_SLOT_COUNT; slot++) {
      const binding = defaults[slot];
      if (!binding) continue;
      for (const conflict of conflictsFor(id, slot, binding, next)) {
        // The target's two authored defaults were validated at registration.
        if (conflict.actionId === id) continue;
        next.get(conflict.actionId)[conflict.slot] = null;
        cleared.push(conflict);
      }
    }
    for (const [actionId, slots] of next) bindings.set(actionId, normalizeSlotsFor(actionId, slots));
    if (authoredTuning.has(id)) tuning.set(id, authoredTuning.get(id));
    return { status: 'applied', actionId: id, action: action(id), cleared };
  }

  /** Restore the complete conflict-free authored profile atomically. */
  function resetAllBindings() {
    for (const [id, slots] of authoredDefaults) {
      bindings.set(id, normalizeSlotsFor(id, copySlots(slots)));
    }
    for (const [id, value] of authoredTuning) tuning.set(id, value);
    return { status: 'applied', profile: bindingProfile() };
  }

  /** Plain serialisable binding map for the explicit parent→iframe seam. */
  function bindingProfile() {
    const profile = {};
    for (const id of definitions.keys()) profile[id] = copySlots(bindings.get(id));
    return profile;
  }

  /**
   * Validate an untrusted persisted/imported binding+tuning profile without
   * touching the live registry.  Supplied choices take precedence over
   * conflict-free authored defaults for missing actions, while unknown future
   * actions are ignored.  Every supplied known action must carry exactly two
   * slots; reserved chords and overlapping-context conflicts among those
   * supplied choices are refused with a machine-readable result.
   */
  function validateProfile(profile = {}) {
    const rawBindings = profile && profile.bindings;
    const rawTuning = profile && profile.tuning;
    if (rawBindings != null && (typeof rawBindings !== 'object' || Array.isArray(rawBindings))) {
      return { status: 'invalid', code: 'bindings-shape' };
    }
    if (rawTuning != null && (typeof rawTuning !== 'object' || Array.isArray(rawTuning))) {
      return { status: 'invalid', code: 'tuning-shape' };
    }

    const nextBindings = new Map(
      [...definitions.keys()].map((id) => [id, [null, null]]),
    );
    const nextTuning = new Map(
      [...authoredTuning].map(([id, value]) => [id, { ...value }]),
    );
    const suppliedBindings = new Set();
    const ignoredBindings = [];
    const ignoredTuning = [];
    const reconciledDefaults = [];

    for (const id of Object.keys(rawBindings || {})) {
      if (!definitions.has(id)) {
        ignoredBindings.push(id);
        continue;
      }
      const slots = rawBindings[id];
      if (!Array.isArray(slots) || slots.length !== SEMANTIC_BINDING_SLOT_COUNT) {
        return { status: 'invalid', code: 'binding-slot-count', actionId: id };
      }
      try {
        nextBindings.set(id, normalizeSlotsFor(id, slots));
        suppliedBindings.add(id);
      } catch (_) {
        return { status: 'invalid', code: 'binding-shape', actionId: id };
      }
    }

    // Validate the untrusted choices before introducing any authored defaults.
    // That keeps a valid older remap authoritative when a newer action happens
    // to gain the same default binding.
    for (const id of suppliedBindings) {
      const slots = nextBindings.get(id);
      for (let slot = 0; slot < SEMANTIC_BINDING_SLOT_COUNT; slot++) {
        const binding = slots[slot];
        if (binding && binding.type === 'keyboard' && isReservedKeyboardBinding(binding)) {
          return { status: 'invalid', code: 'binding-reserved', actionId: id, slot };
        }
        const conflicts = conflictsFor(id, slot, binding, nextBindings);
        if (conflicts.length > 0) {
          return {
            status: 'invalid', code: 'binding-conflict', actionId: id, slot,
            conflicts,
          };
        }
      }
    }

    // Reconcile actions absent from an older profile in registration order.
    // Each default slot is accepted only when it does not displace a supplied
    // choice; a colliding slot stays empty and is reported to the importer.
    for (const [id, defaults] of authoredDefaults) {
      if (suppliedBindings.has(id)) continue;
      const slots = nextBindings.get(id);
      for (let slot = 0; slot < SEMANTIC_BINDING_SLOT_COUNT; slot++) {
        const binding = defaults[slot];
        if (!binding) continue;
        const conflicts = conflictsFor(id, slot, binding, nextBindings);
        if (conflicts.length > 0) {
          reconciledDefaults.push({
            actionId: id,
            slot,
            binding: copyBinding(binding),
            conflicts,
          });
          continue;
        }
        slots[slot] = copyBinding(binding);
      }
    }

    for (const id of Object.keys(rawTuning || {})) {
      if (!authoredTuning.has(id)) {
        ignoredTuning.push(id);
        continue;
      }
      try {
        nextTuning.set(id, normalizeContinuousTuning(rawTuning[id]));
      } catch (_) {
        return { status: 'invalid', code: 'tuning-shape', actionId: id };
      }
    }

    const normalizedBindings = {};
    for (const [id, slots] of nextBindings) normalizedBindings[id] = copySlots(slots);
    const normalizedTuning = {};
    for (const [id, value] of nextTuning) normalizedTuning[id] = { ...value };
    return {
      status: 'valid',
      profile: { bindings: normalizedBindings, tuning: normalizedTuning },
      ignoredBindings,
      ignoredTuning,
      reconciledDefaults,
    };
  }

  /** Atomically replace live bindings+tuning after complete validation. */
  function replaceProfile(profile = {}) {
    const validated = validateProfile(profile);
    if (validated.status !== 'valid') return validated;
    for (const [id, slots] of Object.entries(validated.profile.bindings)) {
      bindings.set(id, normalizeSlotsFor(id, slots));
    }
    for (const [id, value] of Object.entries(validated.profile.tuning)) {
      tuning.set(id, normalizeContinuousTuning(value));
    }
    return { ...validated, status: 'applied' };
  }

  /** Apply known entries from a parent-owned in-memory binding profile. */
  function updateBindings(profile) {
    if (!profile || typeof profile !== 'object') return bindingProfile();
    for (const id of definitions.keys()) {
      if (Object.prototype.hasOwnProperty.call(profile, id)) {
        bindings.set(id, normalizeSlotsFor(id, profile[id]));
      }
    }
    return bindingProfile();
  }

  /** Plain serialisable continuous-axis tuning map for #1279 persistence. */
  function tuningProfile() {
    const profile = {};
    for (const [id, value] of tuning) profile[id] = { ...value };
    return profile;
  }

  function setTuning(id, value) {
    const definition = definitions.get(id);
    if (!definition) throw new Error('unknown semantic action: ' + id);
    if (!definition.continuous) throw new Error('semantic action is not continuous: ' + id);
    const next = normalizeContinuousTuning({ ...tuning.get(id), ...(value || {}) });
    tuning.set(id, next);
    return { status: 'applied', actionId: id, action: action(id) };
  }

  /** Apply known entries from a future persistence layer without storing here. */
  function updateTuning(profile) {
    if (!profile || typeof profile !== 'object') return tuningProfile();
    for (const id of tuning.keys()) {
      if (Object.prototype.hasOwnProperty.call(profile, id)) setTuning(id, profile[id]);
    }
    return tuningProfile();
  }

  function activate(id, options = {}) {
    const definition = definitions.get(id);
    const context = options.context;
    if (!definition || !definition.contexts.includes(context)) {
      return { claimed: false, actionId: id || null, handled: false };
    }
    let continuousValue = null;
    if (definition.continuous) {
      continuousValue = Number(options.value);
      if (!Number.isFinite(continuousValue)
          || continuousValue < definition.continuous.min
          || continuousValue > definition.continuous.max) {
        return { claimed: true, actionId: id, handled: false };
      }
    }
    const adapter = adapters.get(id);
    const feedback = definition.feedback && actionFeedback
      && typeof actionFeedback.press === 'function'
      ? actionFeedback.press(id)
      : null;
    // A local adapter may finish synchronously while its activation call is
    // still on the stack.  Keep that result until handled=true has advanced
    // the lifecycle to Pending; handled=false still cancels the provisional
    // press without ever presenting a terminal result.
    let activationOpen = true;
    let bufferedLocalFeedback = null;
    const bufferOrApplyLocalFeedback = (kind, outcome = null) => {
      if (activationOpen) {
        if (bufferedLocalFeedback) return false;
        bufferedLocalFeedback = { kind, outcome };
        return true;
      }
      return kind === 'settle'
        ? actionFeedback.settle(feedback.correlation, outcome)
        : actionFeedback.cancel(feedback.correlation);
    };
    let handled = false;
    let feedbackSuppressed = false;
    try {
      const adapterResult = adapter ? adapter({
        actionId: id,
        context,
        source: options.source || 'control',
        event: options.event || null,
        // Parameterised controls (for example one button per authored
        // Viewscreen camera or objective) keep one stable semantic identity.
        // The selected authored value is activation-local detail, never part
        // of the binding identity or persisted profile.
        detail: options.detail && typeof options.detail === 'object'
          ? { ...options.detail }
          : null,
        // A visible component may own local selection/cursor state needed to
        // resolve its semantic detail. Keep that activation-local surface out
        // of definitions and persisted bindings, but preserve it for the
        // adapter instead of making a composite document guess which of two
        // identical charts the operator used.
        surface: options.surface && typeof options.surface === 'object'
          ? options.surface
          : null,
        binding: options.binding && typeof options.binding === 'object'
          ? { ...options.binding }
          : null,
        pressed: options.pressed !== false,
        correlation: feedback ? feedback.correlation : null,
        inputMs: feedback ? feedback.inputMs : null,
        feedbackKind: definition.feedback,
        value: continuousValue,
        settleFeedback: feedback && definition.feedback === 'local'
          ? (outcome) => bufferOrApplyLocalFeedback('settle', outcome)
          : null,
        cancelFeedback: feedback && definition.feedback === 'local'
          ? () => bufferOrApplyLocalFeedback('cancel')
          : null,
      }) : false;
      feedbackSuppressed = adapterResult === SEMANTIC_HANDLED_WITHOUT_FEEDBACK;
      handled = adapterResult !== false;
    } catch (error) {
      activationOpen = false;
      if (feedback && typeof actionFeedback.cancel === 'function') {
        actionFeedback.cancel(feedback.correlation);
      }
      throw error;
    }
    activationOpen = false;
    if (feedback) {
      if (handled && !feedbackSuppressed && typeof actionFeedback.pending === 'function') {
        actionFeedback.pending(feedback.correlation);
        if (bufferedLocalFeedback?.kind === 'settle') {
          actionFeedback.settle(feedback.correlation, bufferedLocalFeedback.outcome);
        } else if (bufferedLocalFeedback?.kind === 'cancel') {
          actionFeedback.cancel(feedback.correlation);
        }
      } else if ((!handled || feedbackSuppressed)
          && typeof actionFeedback.cancel === 'function') {
        actionFeedback.cancel(feedback.correlation);
      }
    }
    return {
      claimed: true,
      actionId: id,
      handled,
      ...(feedback && handled && !feedbackSuppressed
        ? { correlation: feedback.correlation, inputMs: feedback.inputMs }
        : {}),
    };
  }

  function dispatchKeyboardEvent(event, context) {
    if (!event || (event.type && event.type !== 'keydown' && event.type !== 'keyup')) {
      return { claimed: false, actionId: null, handled: false };
    }
    if (event.type === 'keyup') {
      const heldKey = [...keyboardHolds.keys()]
        .find((code) => keyboardCodesEqual(code, event.code));
      if (!heldKey) return { claimed: false, actionId: null, handled: false };
      const held = keyboardHolds.get(heldKey);
      keyboardHolds.delete(heldKey);
      if (event.cancelable !== false && typeof event.preventDefault === 'function') {
        event.preventDefault();
      }
      return activate(held.actionId, {
        context: held.context,
        source: 'keyboard',
        event,
        binding: held.binding,
        pressed: false,
      });
    }
    if (event.repeat) return { claimed: false, actionId: null, handled: false };
    if (isSemanticInputTarget(event.target)) {
      return { claimed: false, actionId: null, handled: false };
    }
    for (const entry of list(context)) {
      const binding = entry.bindings.find((candidate) => keyboardBindingMatches(candidate, event));
      if (!binding) continue;
      // The binding is now claimed. Nothing else causes preventDefault — an
      // unbound key remains ordinary browser/page input.
      if (event.cancelable !== false && typeof event.preventDefault === 'function') {
        event.preventDefault();
      }
      const result = activate(entry.id, {
        context, source: 'keyboard', event, binding, pressed: true,
      });
      if (entry.hold && result.handled) {
        keyboardHolds.set(event.code, {
          actionId: entry.id,
          context,
          binding,
          eventCode: event.code,
        });
      }
      return result;
    }
    return { claimed: false, actionId: null, handled: false };
  }

  function releaseKeyboardHolds(context) {
    const releases = [];
    for (const [code, held] of keyboardHolds) {
      if (context && held.context !== context) continue;
      keyboardHolds.delete(code);
      releases.push(activate(held.actionId, {
        context: held.context,
        source: 'keyboard',
        event: { code: held.eventCode },
        binding: held.binding,
        pressed: false,
      }));
    }
    return releases;
  }

  return {
    register,
    action,
    list,
    setBinding,
    resetAction,
    resetAllBindings,
    bindingProfile,
    validateProfile,
    replaceProfile,
    updateBindings,
    tuningProfile,
    setTuning,
    updateTuning,
    activate,
    dispatchKeyboardEvent,
    releaseKeyboardHolds,
  };
}

if (typeof window !== 'undefined') {
  window.createSemanticActionRegistry = createSemanticActionRegistry;
}
