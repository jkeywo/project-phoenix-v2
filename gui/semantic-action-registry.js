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

export const SEMANTIC_BINDING_SLOT_COUNT = 2;

const MODIFIER_KEYS = ['ctrlKey', 'shiftKey', 'altKey', 'metaKey'];
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
  return Object.freeze({
    type: 'keyboard',
    code,
    ctrlKey: !!value.ctrlKey,
    shiftKey: !!value.shiftKey,
    altKey: !!value.altKey,
    metaKey: !!value.metaKey,
  });
}

/** Normalize and pad an action's binding list to exactly two slots. */
export function normalizeBindingSlots(values) {
  if (values != null && !Array.isArray(values)) {
    throw new TypeError('semantic action bindings must be an array');
  }
  const slots = values || [];
  if (slots.length > SEMANTIC_BINDING_SLOT_COUNT) {
    throw new RangeError('semantic action has exactly two binding slots');
  }
  return Object.freeze(Array.from(
    { length: SEMANTIC_BINDING_SLOT_COUNT },
    (_, index) => normalizeKeyboardBinding(slots[index]),
  ));
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
  return left.code === right.code
    && MODIFIER_KEYS.every((key) => left[key] === right[key]);
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
  return binding.code === event.code
    && MODIFIER_KEYS.every((key) => binding[key] === !!event[key]);
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
  return Object.freeze({
    id,
    contexts: Object.freeze(contexts),
    labelId,
    accessibilityLabelId,
    bindings: normalizeBindingSlots(definition.bindings),
  });
}

function copyBinding(binding) {
  return binding ? { ...binding } : null;
}

function copySlots(slots) {
  return slots.map(copyBinding);
}

/** Create an isolated registry. No mutable singleton is shared across frames. */
export function createSemanticActionRegistry() {
  const definitions = new Map();
  const adapters = new Map();
  const authoredDefaults = new Map();
  const bindings = new Map();

  function assertActionAndSlot(id, slot) {
    if (!definitions.has(id)) throw new Error('unknown semantic action: ' + id);
    if (!Number.isInteger(slot) || slot < 0 || slot >= SEMANTIC_BINDING_SLOT_COUNT) {
      throw new RangeError('semantic action binding slot must be zero or one');
    }
  }

  function contextsOverlap(left, right) {
    return left.contexts.some((context) => right.contexts.includes(context));
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
        if (keyboardBindingsEqual(binding, otherSlots[otherSlot])) {
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
      bindings.set(actionId, normalizeBindingSlots(slots));
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
    authoredDefaults.set(normalized.id, normalizeBindingSlots(normalized.bindings));
    bindings.set(normalized.id, normalizeBindingSlots(normalized.bindings));
    for (let slot = 0; slot < SEMANTIC_BINDING_SLOT_COUNT; slot++) {
      const binding = normalized.bindings[slot];
      if (isReservedKeyboardBinding(binding)) {
        definitions.delete(normalized.id);
        authoredDefaults.delete(normalized.id);
        bindings.delete(normalized.id);
        throw new Error('semantic action authored default is reserved: ' + normalized.id);
      }
      if (conflictsFor(normalized.id, slot, binding).length > 0) {
        definitions.delete(normalized.id);
        authoredDefaults.delete(normalized.id);
        bindings.delete(normalized.id);
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
    };
  }

  function list(context) {
    return [...definitions.keys()]
      .map(action)
      .filter((entry) => !context || entry.contexts.includes(context));
  }

  function setBinding(id, slot, value, options = {}) {
    assertActionAndSlot(id, slot);
    const binding = normalizeKeyboardBinding(value);
    if (isReservedKeyboardBinding(binding)) {
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
    for (const [actionId, slots] of next) bindings.set(actionId, normalizeBindingSlots(slots));
    return { status: 'applied', actionId: id, action: action(id), cleared };
  }

  /** Restore the complete conflict-free authored profile atomically. */
  function resetAllBindings() {
    for (const [id, slots] of authoredDefaults) {
      bindings.set(id, normalizeBindingSlots(copySlots(slots)));
    }
    return { status: 'applied', profile: bindingProfile() };
  }

  /** Plain serialisable binding map for the explicit parent→iframe seam. */
  function bindingProfile() {
    const profile = {};
    for (const id of definitions.keys()) profile[id] = copySlots(bindings.get(id));
    return profile;
  }

  /** Apply known entries from a parent-owned in-memory binding profile. */
  function updateBindings(profile) {
    if (!profile || typeof profile !== 'object') return bindingProfile();
    for (const id of definitions.keys()) {
      if (Object.prototype.hasOwnProperty.call(profile, id)) {
        bindings.set(id, normalizeBindingSlots(profile[id]));
      }
    }
    return bindingProfile();
  }

  function activate(id, options = {}) {
    const definition = definitions.get(id);
    const context = options.context;
    if (!definition || !definition.contexts.includes(context)) {
      return { claimed: false, actionId: id || null, handled: false };
    }
    const adapter = adapters.get(id);
    const handled = adapter ? adapter({
      actionId: id,
      context,
      source: options.source || 'control',
      event: options.event || null,
    }) !== false : false;
    return { claimed: true, actionId: id, handled };
  }

  function dispatchKeyboardEvent(event, context) {
    if (!event || (event.type && event.type !== 'keydown') || event.repeat) {
      return { claimed: false, actionId: null, handled: false };
    }
    if (isSemanticInputTarget(event.target)) {
      return { claimed: false, actionId: null, handled: false };
    }
    for (const entry of list(context)) {
      if (!entry.bindings.some((binding) => keyboardBindingMatches(binding, event))) continue;
      // The binding is now claimed. Nothing else causes preventDefault — an
      // unbound key remains ordinary browser/page input.
      if (event.cancelable !== false && typeof event.preventDefault === 'function') {
        event.preventDefault();
      }
      return activate(entry.id, { context, source: 'keyboard', event });
    }
    return { claimed: false, actionId: null, handled: false };
  }

  return {
    register,
    action,
    list,
    setBinding,
    resetAction,
    resetAllBindings,
    bindingProfile,
    updateBindings,
    activate,
    dispatchKeyboardEvent,
  };
}

if (typeof window !== 'undefined') {
  window.createSemanticActionRegistry = createSemanticActionRegistry;
}
