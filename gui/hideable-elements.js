/**
 * gui/hideable-elements.js - Applies complexity preset hiding to console DOM.
 *
 * Console HTML pages mark hideable nodes with `data-hideable="<element_name>"`;
 * console-core calls `applyHiddenElements` after each state render when the
 * state carries a `complexityPreset` field.
 */

export const HIDEABLE_CLASS = 'cpx-hidden';

export const HIDDEN_ELEMENTS = Object.freeze({
  Tactical: Object.freeze({
    Std: Object.freeze([]),
    Low: Object.freeze(['phaser_mode_selector', 'torpedo_tube_selector', 'target_lock_button']),
  }),
  Power: Object.freeze({
    Std: Object.freeze([]),
    Low: Object.freeze(['power_overflow_controls']),
  }),
  Sensors: Object.freeze({ Std: Object.freeze([]) }),
  Shields: Object.freeze({ Std: Object.freeze([]) }),
  Navigation: Object.freeze({ Std: Object.freeze([]) }),
});

export function hideableElementNames(consoleName, presetName) {
  const presets = HIDDEN_ELEMENTS[consoleName];
  if (!presets) return [];
  const list = presets[presetName];
  return list ? Array.from(list) : [];
}

export function plannedChanges(currentHidden, consoleName, presetName, registeredNames) {
  const toHide = hideableElementNames(consoleName, presetName);
  const hidden = currentHidden instanceof Set ? currentHidden : new Set(currentHidden || []);
  const registered = registeredNames instanceof Set ? registeredNames : new Set(registeredNames || []);
  const toShow = [...hidden].filter((n) => !toHide.includes(n));
  const unknown = toHide.filter((n) => !registered.has(n));
  return { toHide, toShow, unknown };
}

export function applyChanges(hiddenSet, changes) {
  for (const name of changes.toHide) hiddenSet.add(name);
  for (const name of changes.toShow) hiddenSet.delete(name);
  return hiddenSet;
}

export function applyHiddenElements(rootEl, consoleName, presetName) {
  if (!rootEl || typeof rootEl.querySelectorAll !== 'function') {
    return { toHide: [], toShow: [], unknown: [] };
  }

  const els = Array.from(rootEl.querySelectorAll('[data-hideable]'));
  const registered = new Set();
  const currentHidden = new Set();

  for (const el of els) {
    const name = el.getAttribute('data-hideable');
    if (!name) continue;
    registered.add(name);
    if (el.classList.contains(HIDEABLE_CLASS)) currentHidden.add(name);
  }

  const changes = plannedChanges(currentHidden, consoleName, presetName, registered);

  for (const name of changes.unknown) {
    console.warn(
      '[' + consoleName + '] hideable element "' + name + '" is in the complexity '
      + 'preset table but no element carries data-hideable="' + name + '"',
    );
  }

  for (const el of els) {
    const name = el.getAttribute('data-hideable');
    if (changes.toHide.includes(name)) el.classList.add(HIDEABLE_CLASS);
    else if (changes.toShow.includes(name)) el.classList.remove(HIDEABLE_CLASS);
  }

  return changes;
}

if (typeof window !== 'undefined') {
  window.hideableElementNames = hideableElementNames;
  window.applyHiddenElements = applyHiddenElements;
}
