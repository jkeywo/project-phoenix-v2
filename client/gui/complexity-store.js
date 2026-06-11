/**
 * gui/complexity-store.js — Pure JS port of src/client_complexity.rs
 * (ComplexityChoice + ComplexityStore) with localStorage persistence.
 * Issue #460.
 *
 * Storage is injected (`attachStorage`) / feature-detected so the module is
 * testable in Node with a fake storage object exposing getItem/setItem.
 *
 * Wire format of the stored value (key "complexity-presets"):
 *   { "Helm": "Std", "Tactical": "Low", ... }
 *
 * Exposed on `window` as `window.complexityStore` (singleton). The singleton
 * self-attaches `window.localStorage` and loads stored presets on module load.
 */

export const COMPLEXITY_STORAGE_KEY = 'complexity-presets';

/**
 * Lifecycle state of the complexity preset choice for a single console.
 * Mirrors `ComplexityChoice` in src/client_complexity.rs.
 */
export class ComplexityChoice {
  /**
   * @param {string[]} availablePresets Preset names in display order.
   * @param {string|null} [stored] Optional stored preset from a previous session.
   */
  constructor(availablePresets, stored = null) {
    this.availablePresets = availablePresets;
    this.chosen = stored != null ? stored : null;
    this.popupShown = this.chosen !== null;
  }

  /**
   * The preset that should be active: the chosen one, or the sole available
   * preset, or "Low" (the default selection on the first-use pop-up).
   */
  effectivePreset() {
    if (this.chosen !== null) return this.chosen;
    if (this.availablePresets.length === 1) return this.availablePresets[0];
    return 'Low';
  }

  /** True when the console has more than one preset to choose from. */
  showDropdown() {
    return this.availablePresets.length > 1;
  }

  /** True when a first-use (or re-prompt) pop-up should be displayed. */
  showPopup() {
    return this.showDropdown() && !this.popupShown && this.chosen === null;
  }

  /** True when the stored preset name is not in the available list. */
  isStale() {
    if (this.chosen === null) return false;
    return !this.availablePresets.includes(this.chosen);
  }

  /**
   * Select a preset by name. Returns true on success, false when the name
   * is not in the available list (Rust returns Result).
   */
  select(name) {
    if (!this.availablePresets.includes(name)) return false;
    this.chosen = name;
    this.popupShown = true;
    return true;
  }
}

/**
 * Default available presets for a console: Tactical and Power get
 * ["Low", "Std"]; every other console gets ["Std"] only.
 */
export function defaultPresetsFor(console) {
  return (console === 'Tactical' || console === 'Power')
    ? ['Low', 'Std']
    : ['Std'];
}

/** Build a SetComplexity ClientMessage `{ type, data }`. */
export function setComplexityMessage(console, presetName) {
  return { type: 'SetComplexity', data: { console, preset_name: presetName } };
}

/**
 * Per-console complexity preset selections, keyed by console name.
 * Mirrors `ComplexityStore` in src/client_complexity.rs, plus localStorage
 * persistence (previously done inline in client.html / the Bevy bridge).
 */
export class ComplexityStore {
  /**
   * @param {{getItem:Function, setItem:Function}|null} [storage]
   *        Optional Web-Storage-like object for persistence.
   */
  constructor(storage = null) {
    /** Map of console name → ComplexityChoice. */
    this.choices = new Map();
    this._storage = storage;
  }

  /** Attach (or replace) the persistence backend. */
  attachStorage(storage) {
    this._storage = storage;
  }

  /**
   * Get the choice state for a console, creating a default one if absent.
   * Mirrors `ComplexityStore::for_console`.
   */
  forConsole(console) {
    let choice = this.choices.get(console);
    if (!choice) {
      choice = new ComplexityChoice(defaultPresetsFor(console), null);
      this.choices.set(console, choice);
    }
    return choice;
  }

  /**
   * Apply stored complexity presets (an object map console → preset name).
   * Only valid preset names are applied; stale names are discarded so the
   * UI shows the first-use pop-up again. Mirrors `apply_stored`.
   */
  applyStored(stored) {
    for (const [console, presetName] of Object.entries(stored || {})) {
      const choice = this.forConsole(console);
      if (choice.availablePresets.includes(presetName)) {
        choice.select(presetName);
      }
      // Stale: leave unchosen so the pop-up re-triggers.
    }
  }

  /**
   * Select a preset for a console (creating the default choice if needed)
   * and persist on success. Returns true when the selection was applied.
   */
  select(console, presetName) {
    const ok = this.forConsole(console).select(presetName);
    if (ok) this.persist();
    return ok;
  }

  /**
   * Apply a single inbound ServerMessage. Handles ComplexityChanged:
   * updates the store (mirroring the deleted Bevy `apply_inbound_messages`
   * sync) and persists to storage. Like the Bevy drain, only a choice this
   * client has already materialised (via `forConsole`) is updated —
   * broadcasts about other players' consoles must not create a local
   * choice, which would persist their preset as this device's preference
   * and suppress the first-use popup.
   */
  apply(msg) {
    if (!msg || msg.type !== 'ComplexityChanged') return;
    const d = msg.data || {};
    if (!d.console || !d.preset_name) return;
    if (!this.choices.has(d.console)) return;
    this.select(d.console, d.preset_name);
  }

  /** Plain-object snapshot of the chosen presets, suitable for storage. */
  toStoredObject() {
    const out = {};
    for (const [console, choice] of this.choices) {
      if (choice.chosen !== null) out[console] = choice.chosen;
    }
    return out;
  }

  /**
   * Load stored presets from the attached storage (no-op when none).
   * Malformed JSON is ignored.
   */
  loadFromStorage() {
    if (!this._storage) return;
    let stored = null;
    try {
      stored = JSON.parse(this._storage.getItem(COMPLEXITY_STORAGE_KEY));
    } catch (_) { /* ignore malformed storage */ }
    if (stored && typeof stored === 'object') this.applyStored(stored);
  }

  /**
   * Persist the chosen presets to the attached storage (no-op when none).
   * Merges over any presets already stored so entries for consoles this
   * store has not yet materialised are preserved.
   */
  persist() {
    if (!this._storage) return;
    let existing = {};
    try {
      existing = JSON.parse(this._storage.getItem(COMPLEXITY_STORAGE_KEY)) || {};
    } catch (_) { /* start fresh on malformed storage */ }
    const merged = Object.assign({}, existing, this.toStoredObject());
    try {
      this._storage.setItem(COMPLEXITY_STORAGE_KEY, JSON.stringify(merged));
    } catch (_) { /* storage full / unavailable — selection still applies in memory */ }
  }
}

/** Singleton used by client.html. */
export const complexityStore = new ComplexityStore();

if (typeof window !== 'undefined') {
  // Feature-detect browser storage; tests construct their own store with a fake.
  try {
    if (window.localStorage) {
      complexityStore.attachStorage(window.localStorage);
      complexityStore.loadFromStorage();
    }
  } catch (_) { /* storage blocked (private mode) — run memory-only */ }
  window.complexityStore = complexityStore;
}
