/**
 * gui/complexity-store.js - Pure JS port of src/client_complexity.rs
 * (ComplexityChoice + ComplexityStore) with localStorage persistence.
 *
 * Wire format of the stored value (key "complexity-presets"):
 *   { "Helm": "Std", "Tactical": "Low", ... }
 */

export const COMPLEXITY_STORAGE_KEY = 'complexity-presets';

export class ComplexityChoice {
  constructor(availablePresets, stored = null) {
    this.availablePresets = availablePresets;
    this.chosen = stored != null ? stored : null;
    this.popupShown = this.chosen !== null;
  }

  effectivePreset() {
    if (this.chosen !== null) return this.chosen;
    if (this.availablePresets.length === 1) return this.availablePresets[0];
    return 'Low';
  }

  showDropdown() {
    return this.availablePresets.length > 1;
  }

  showPopup() {
    return this.showDropdown() && !this.popupShown && this.chosen === null;
  }

  isStale() {
    if (this.chosen === null) return false;
    return !this.availablePresets.includes(this.chosen);
  }

  select(name) {
    if (!this.availablePresets.includes(name)) return false;
    this.chosen = name;
    this.popupShown = true;
    return true;
  }
}

export function defaultPresetsFor(consoleName) {
  return (consoleName === 'Tactical' || consoleName === 'Power')
    ? ['Low', 'Std']
    : ['Std'];
}

export function setComplexityMessage(consoleName, presetName) {
  return { type: 'SetComplexity', data: { console: consoleName, preset_name: presetName } };
}

export class ComplexityStore {
  constructor(storage = null) {
    this.choices = new Map();
    this._storage = storage;
  }

  attachStorage(storage) {
    this._storage = storage;
  }

  forConsole(consoleName) {
    let choice = this.choices.get(consoleName);
    if (!choice) {
      choice = new ComplexityChoice(defaultPresetsFor(consoleName), null);
      this.choices.set(consoleName, choice);
    }
    return choice;
  }

  applyStored(stored) {
    for (const [consoleName, presetName] of Object.entries(stored || {})) {
      const choice = this.forConsole(consoleName);
      if (choice.availablePresets.includes(presetName)) {
        choice.select(presetName);
      }
    }
  }

  select(consoleName, presetName) {
    const ok = this.forConsole(consoleName).select(presetName);
    if (ok) this.persist();
    return ok;
  }

  apply(msg) {
    if (!msg || msg.type !== 'ComplexityChanged') return;
    const d = msg.data || {};
    if (!d.console || !d.preset_name) return;
    if (!this.choices.has(d.console)) return;
    this.select(d.console, d.preset_name);
  }

  toStoredObject() {
    const out = {};
    for (const [consoleName, choice] of this.choices) {
      if (choice.chosen !== null) out[consoleName] = choice.chosen;
    }
    return out;
  }

  loadFromStorage() {
    if (!this._storage) return;
    let stored = null;
    try {
      stored = JSON.parse(this._storage.getItem(COMPLEXITY_STORAGE_KEY));
    } catch (_) {
      // Ignore malformed storage.
    }
    if (stored && typeof stored === 'object') this.applyStored(stored);
  }

  persist() {
    if (!this._storage) return;
    let existing = {};
    try {
      existing = JSON.parse(this._storage.getItem(COMPLEXITY_STORAGE_KEY)) || {};
    } catch (_) {
      // Start fresh on malformed storage.
    }
    const merged = Object.assign({}, existing, this.toStoredObject());
    try {
      this._storage.setItem(COMPLEXITY_STORAGE_KEY, JSON.stringify(merged));
    } catch (_) {
      // Storage may be full or unavailable; the in-memory selection still applies.
    }
  }
}

export const complexityStore = new ComplexityStore();

if (typeof window !== 'undefined') {
  try {
    if (window.localStorage) {
      complexityStore.attachStorage(window.localStorage);
      complexityStore.loadFromStorage();
    }
  } catch (_) {
    // Storage can be blocked in private browsing modes.
  }
  window.complexityStore = complexityStore;
}
