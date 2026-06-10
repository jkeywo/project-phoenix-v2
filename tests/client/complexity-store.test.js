import { describe, it, expect } from 'vitest';
import {
  ComplexityChoice, ComplexityStore, complexityStore,
  defaultPresetsFor, setComplexityMessage, COMPLEXITY_STORAGE_KEY,
} from '../../gui/complexity-store.js';

/** Minimal Web-Storage fake for injection. */
function fakeStorage(initial = {}) {
  const data = { ...initial };
  return {
    data,
    getItem: k => (k in data ? data[k] : null),
    setItem: (k, v) => { data[k] = v; },
  };
}

describe('ComplexityChoice', () => {
  it('multiple presets, no choice: popup + dropdown, effective defaults to Low', () => {
    const c = new ComplexityChoice(['Low', 'Std'], null);
    expect(c.showDropdown()).toBe(true);
    expect(c.showPopup()).toBe(true);
    expect(c.effectivePreset()).toBe('Low');
  });

  it('stored choice suppresses popup and is the effective preset', () => {
    const c = new ComplexityChoice(['Low', 'Std'], 'Std');
    expect(c.showDropdown()).toBe(true);
    expect(c.showPopup()).toBe(false);
    expect(c.effectivePreset()).toBe('Std');
  });

  it('single preset hides dropdown and popup; effective is the only one', () => {
    const c = new ComplexityChoice(['Std'], null);
    expect(c.showDropdown()).toBe(false);
    expect(c.showPopup()).toBe(false);
    expect(c.effectivePreset()).toBe('Std');
  });

  it('select updates chosen, marks popup shown, rejects unknown names', () => {
    const c = new ComplexityChoice(['Low', 'Std'], null);
    expect(c.select('Std')).toBe(true);
    expect(c.chosen).toBe('Std');
    expect(c.popupShown).toBe(true);
    expect(c.showPopup()).toBe(false);
    expect(c.select('High')).toBe(false);
    expect(c.chosen).toBe('Std');
  });

  it('isStale detects a chosen preset missing from the available list', () => {
    expect(new ComplexityChoice(['Low', 'Std'], 'High').isStale()).toBe(true);
    expect(new ComplexityChoice(['Low', 'Std'], 'Std').isStale()).toBe(false);
    expect(new ComplexityChoice(['Low', 'Std'], null).isStale()).toBe(false);
  });
});

describe('defaultPresetsFor', () => {
  it('Tactical and Power get [Low, Std]; everything else [Std]', () => {
    expect(defaultPresetsFor('Tactical')).toEqual(['Low', 'Std']);
    expect(defaultPresetsFor('Power')).toEqual(['Low', 'Std']);
    for (const c of ['CaptainChair', 'Helm', 'Repair', 'Sensors', 'Shields', 'Navigation', 'Comms']) {
      expect(defaultPresetsFor(c)).toEqual(['Std']);
    }
  });
});

describe('ComplexityStore.forConsole', () => {
  it('creates default choices lazily and memoises them', () => {
    const store = new ComplexityStore();
    const tac = store.forConsole('Tactical');
    expect(tac.availablePresets).toEqual(['Low', 'Std']);
    expect(tac.showDropdown()).toBe(true);
    expect(store.forConsole('Tactical')).toBe(tac);
    const helm = store.forConsole('Helm');
    expect(helm.availablePresets).toEqual(['Std']);
    expect(helm.showDropdown()).toBe(false);
  });
});

describe('ComplexityStore.applyStored', () => {
  it('applies valid stored presets and suppresses the popup', () => {
    const store = new ComplexityStore();
    store.applyStored({ Tactical: 'Std' });
    const choice = store.forConsole('Tactical');
    expect(choice.effectivePreset()).toBe('Std');
    expect(choice.showPopup()).toBe(false);
  });

  it('discards stale presets so the popup re-prompts', () => {
    const store = new ComplexityStore();
    store.applyStored({ Tactical: 'High' });
    const choice = store.forConsole('Tactical');
    expect(choice.chosen).toBeNull();
    expect(choice.showPopup()).toBe(true);
    expect(choice.effectivePreset()).toBe('Low');
  });
});

describe('localStorage persistence', () => {
  it('loadFromStorage applies the stored JSON map', () => {
    const storage = fakeStorage({ [COMPLEXITY_STORAGE_KEY]: JSON.stringify({ Tactical: 'Std' }) });
    const store = new ComplexityStore(storage);
    store.loadFromStorage();
    expect(store.forConsole('Tactical').effectivePreset()).toBe('Std');
  });

  it('loadFromStorage tolerates missing or malformed storage values', () => {
    const store = new ComplexityStore(fakeStorage({ [COMPLEXITY_STORAGE_KEY]: '{not json' }));
    expect(() => store.loadFromStorage()).not.toThrow();
    const empty = new ComplexityStore(fakeStorage());
    expect(() => empty.loadFromStorage()).not.toThrow();
  });

  it('select persists chosen presets under "complexity-presets"', () => {
    const storage = fakeStorage();
    const store = new ComplexityStore(storage);
    expect(store.select('Tactical', 'Low')).toBe(true);
    expect(JSON.parse(storage.data[COMPLEXITY_STORAGE_KEY])).toEqual({ Tactical: 'Low' });
  });

  it('persist merges over presets already stored for other consoles', () => {
    const storage = fakeStorage({ [COMPLEXITY_STORAGE_KEY]: JSON.stringify({ Helm: 'Std' }) });
    const store = new ComplexityStore(storage);
    store.select('Power', 'Low');
    expect(JSON.parse(storage.data[COMPLEXITY_STORAGE_KEY])).toEqual({ Helm: 'Std', Power: 'Low' });
  });

  it('select of an invalid preset neither applies nor persists', () => {
    const storage = fakeStorage();
    const store = new ComplexityStore(storage);
    expect(store.select('Helm', 'Low')).toBe(false); // Helm only has Std
    expect(storage.data[COMPLEXITY_STORAGE_KEY]).toBeUndefined();
  });
});

describe('apply(ComplexityChanged)', () => {
  it('updates a materialised choice and persists', () => {
    const storage = fakeStorage();
    const store = new ComplexityStore(storage);
    store.forConsole('Tactical'); // materialise, as the complexity UI does
    store.apply({ type: 'ComplexityChanged', data: { console: 'Tactical', preset_name: 'Low' } });
    expect(store.forConsole('Tactical').effectivePreset()).toBe('Low');
    expect(JSON.parse(storage.data[COMPLEXITY_STORAGE_KEY])).toEqual({ Tactical: 'Low' });
  });

  it('ignores consoles without a materialised choice (other players)', () => {
    const storage = fakeStorage();
    const store = new ComplexityStore(storage);
    store.apply({ type: 'ComplexityChanged', data: { console: 'Tactical', preset_name: 'Low' } });
    expect(store.choices.has('Tactical')).toBe(false);
    expect(storage.data[COMPLEXITY_STORAGE_KEY]).toBeUndefined();
  });

  it('ignores other message types and invalid presets', () => {
    const storage = fakeStorage();
    const store = new ComplexityStore(storage);
    store.forConsole('Helm'); // materialise
    store.apply({ type: 'GameStarted' });
    store.apply({ type: 'ComplexityChanged', data: { console: 'Helm', preset_name: 'Bogus' } });
    expect(store.forConsole('Helm').chosen).toBeNull();
    expect(storage.data[COMPLEXITY_STORAGE_KEY]).toBeUndefined();
  });
});

describe('singleton', () => {
  it('is exported and works without attached storage (Node)', () => {
    expect(complexityStore).toBeInstanceOf(ComplexityStore);
    expect(() => complexityStore.persist()).not.toThrow();
  });
});

describe('setComplexityMessage', () => {
  it('builds the SetComplexity wire object', () => {
    expect(setComplexityMessage('Tactical', 'Low'))
      .toEqual({ type: 'SetComplexity', data: { console: 'Tactical', preset_name: 'Low' } });
  });
});
