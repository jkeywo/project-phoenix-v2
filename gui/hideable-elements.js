/**
 * gui/hideable-elements.js — Pure JS port of the hideable-element registry
 * (HideableElementRegistry / hideable_element_names / planned_changes) that
 * lived in src/client/elements.rs until issue #461.
 *
 * Complexity presets can hide named UI elements per console. Console HTML
 * pages mark hideable nodes with `data-hideable="<element_name>"`; when a
 * state push carries a `complexityPreset` field, gui/console-core.js calls
 * `applyHiddenElements` after render to toggle the `.cpx-hidden` class
 * (defined in gui/console.css as `display: none !important`).
 *
 * The preset → hidden_elements table below mirrors the `hidden_elements`
 * arrays in assets/complexity/*.toml — that TOML stays the source of truth
 * (the server's config loader still parses it); keep this table in sync
 * when editing those files.
 *
 * Exposed on `window` as `window.applyHiddenElements` and
 * `window.hideableElementNames` for non-module callers.
 */

/** CSS class applied to hidden elements. */
export const HIDEABLE_CLASS = 'cpx-hidden';

/**
 * Per-console, per-preset hidden element names.
 * Mirrors assets/complexity/{tactical,power,sensors,shields,navigation}.toml.
 */
export const HIDDEN_ELEMENTS = Object.freeze({
  Tactical: Object.freeze({
    Std: Object.freeze([]),
    Low: Object.freeze(['phaser_mode_selector', 'torpedo_tube_selector', 'target_lock_button']),
  }),
  Power: Object.freeze({
    Std: Object.freeze([]),
    Low: Object.freeze(['power_overflow_controls']),
  }),
  Sensors:    Object.freeze({ Std: Object.freeze([]) }),
  Shields:    Object.freeze({ Std: Object.freeze([]) }),
  Navigation: Object.freeze({ Std: Object.freeze([]) }),
});

/**
 * Returns the element names the given preset wants to hide.
 * Mirrors `hideable_element_names` in the deleted Rust code: empty array for
 * consoles without complexity config and for unknown preset names.
 *
 * @param {string} consoleName PascalCase Console enum variant (e.g. 'Tactical').
 * @param {string} presetName  Preset name (e.g. 'Low', 'Std').
 * @returns {string[]} Fresh array of element names to hide.
 */
export function hideableElementNames(consoleName, presetName) {
  const presets = HIDDEN_ELEMENTS[consoleName];
  if (!presets) return [];
  const list = presets[presetName];
  return list ? Array.from(list) : [];
}

/**
 * Compute what should change when switching to the given preset.
 * Mirrors `HideableElementRegistry::planned_changes`:
 *  - toHide:  every name the preset hides (regardless of current state)
 *  - toShow:  currently-hidden names not in the new hide list
 *  - unknown: names in the hide list with no registered element
 *
 * @param {Iterable<string>} currentHidden   Names currently hidden.
 * @param {string}           consoleName     PascalCase console name.
 * @param {string}           presetName      Preset name.
 * @param {Iterable<string>} registeredNames Names of registered elements.
 * @returns {{ toHide: string[], toShow: string[], unknown: string[] }}
 */
export function plannedChanges(currentHidden, consoleName, presetName, registeredNames) {
  const toHide = hideableElementNames(consoleName, presetName);
  const hidden = currentHidden instanceof Set ? currentHidden : new Set(currentHidden || []);
  const registered = registeredNames instanceof Set ? registeredNames : new Set(registeredNames || []);
  const toShow = [...hidden].filter((n) => !toHide.includes(n));
  const unknown = toHide.filter((n) => !registered.has(n));
  return { toHide, toShow, unknown };
}

/**
 * Apply a set of planned changes to a hidden-name Set (in place).
 * Mirrors `HideableElementRegistry::apply_changes`.
 *
 * @param {Set<string>} hiddenSet Mutated in place.
 * @param {{ toHide: string[], toShow: string[] }} changes
 * @returns {Set<string>} The same set, for chaining.
 */
export function applyChanges(hiddenSet, changes) {
  for (const name of changes.toHide) hiddenSet.add(name);
  for (const name of changes.toShow) hiddenSet.delete(name);
  return hiddenSet;
}

/**
 * DOM helper: toggle `.cpx-hidden` on every `[data-hideable]` element under
 * `rootEl` to match the given console/preset. Registration is implicit — the
 * present `data-hideable` attributes are the registry. Unknown names (in the
 * preset table but with no matching element) produce a console.warn, the
 * same diagnostic the deleted Bevy `sync_complexity_hiding` system logged.
 *
 * Idempotent: re-applying the same preset is a no-op.
 *
 * @param {{ querySelectorAll: function }} rootEl Document or element root.
 * @param {string} consoleName PascalCase console name.
 * @param {string} presetName  Preset name.
 * @returns {{ toHide: string[], toShow: string[], unknown: string[] }} The applied changes.
 */
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
      + 'preset table but no element carries data-hideable="' + name + '"; '
      + 'check spelling or add the attribute');
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
