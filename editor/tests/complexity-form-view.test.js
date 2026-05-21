import { describe, it, expect } from 'vitest';
import { installDom, FakeElement, fireInput, fireChange, fireClick } from './slice-5-helpers.js';
import { renderComplexityFormView, KNOWN_CONSOLE_KEYS } from '../complexity-form-view.js';

function mount(opts) {
  installDom();
  const host = new FakeElement('div');
  renderComplexityFormView(host, opts);
  return host;
}

const SAMPLE_PRESETS = [
  {
    name: 'Low',
    hidden_elements: ['phaser_mode_selector', 'unknown_custom_element'],
    delegated: {
      Tactical: { controls: ['auto_fire_torpedoes', 'auto_frequency_match'] },
    },
    ai: {
      torpedo_auto_fire: { lead_prediction: true, min_accuracy: 0.7 },
    },
  },
  {
    name: 'Std',
    hidden_elements: [],
    delegated: {},
    ai: {},
  },
];

describe('renderComplexityFormView', () => {
  it('renders one preset tab per preset and marks the active one', () => {
    const host = mount({
      presets: SAMPLE_PRESETS,
      knownUiElements: [],
      activePresetIndex: 0,
      callbacks: {},
    });
    const tabs = host.querySelectorAll('.def-preset-tab');
    expect(tabs.length).toBe(2);
    expect(tabs[0].textContent).toBe('Low');
    expect(tabs[1].textContent).toBe('Std');
    expect(tabs[0].classList.contains('def-preset-tab-active')).toBe(true);
    expect(tabs[1].classList.contains('def-preset-tab-active')).toBe(false);
  });

  it('clicking a preset tab fires onSwitchPreset with the index', () => {
    let switched = null;
    const host = mount({
      presets: SAMPLE_PRESETS,
      knownUiElements: [],
      activePresetIndex: 0,
      callbacks: {
        onSwitchPreset: (i) => { switched = i; },
      },
    });
    const tabs = host.querySelectorAll('.def-preset-tab');
    fireClick(tabs[1]);
    expect(switched).toBe(1);
  });

  it('hidden_elements dedupes known + authored values', () => {
    const host = mount({
      presets: SAMPLE_PRESETS,
      knownUiElements: ['phaser_mode_selector', 'torpedo_tube_selector'],
      activePresetIndex: 0,
      callbacks: {},
    });
    const select = host.querySelector('.def-hidden-elements-select');
    const options = select.querySelectorAll('OPTION');
    const values = options.map((o) => o.value);
    // Two knowns + one extra authored = 3 distinct entries.
    expect(values).toEqual([
      'phaser_mode_selector',
      'torpedo_tube_selector',
      'unknown_custom_element',
    ]);
    // The two authored values are marked selected; the other known is not.
    const selectedValues = options.filter((o) => o.selected).map((o) => o.value).sort();
    expect(selectedValues).toEqual(['phaser_mode_selector', 'unknown_custom_element']);
  });

  it('hidden_elements change fires onSetHiddenElements with selected values + preset index', () => {
    let called = null;
    const host = mount({
      presets: SAMPLE_PRESETS,
      knownUiElements: ['phaser_mode_selector', 'torpedo_tube_selector'],
      activePresetIndex: 0,
      callbacks: {
        onSetHiddenElements: (i, list) => { called = { i, list }; },
      },
    });
    const select = host.querySelector('.def-hidden-elements-select');
    const options = select.querySelectorAll('OPTION');
    options[0].selected = false;
    options[1].selected = true;
    options[2].selected = true;
    select.dispatchEvent({ type: 'change', target: select });
    expect(called).toEqual({
      i: 0,
      list: ['torpedo_tube_selector', 'unknown_custom_element'],
    });
  });

  it('renders one delegated row per console key, with controls CSV; editing CSV fires onSetDelegated', () => {
    let called = null;
    const host = mount({
      presets: SAMPLE_PRESETS,
      knownUiElements: [],
      activePresetIndex: 0,
      callbacks: {
        onSetDelegated: (i, key, controls) => { called = { i, key, controls }; },
      },
    });
    const rows = host.querySelectorAll('.def-delegated-row');
    expect(rows.length).toBe(1);
    expect(rows[0].dataset.consoleKey).toBe('Tactical');

    const csv = rows[0].querySelector('.def-delegated-controls');
    expect(csv.value).toBe('auto_fire_torpedoes, auto_frequency_match');
    fireInput(csv, 'a, b , c');
    expect(called).toEqual({
      i: 0,
      key: 'Tactical',
      controls: ['a', 'b', 'c'],
    });

    // The console-key dropdown lists every known console.
    const select = rows[0].querySelector('.def-delegated-console');
    const consoleOpts = select.querySelectorAll('OPTION');
    const consoleVals = consoleOpts.map((o) => o.value);
    for (const k of KNOWN_CONSOLE_KEYS) expect(consoleVals).toContain(k);
  });

  it('renders one AI block per behavior with typed inputs (number / boolean / string)', () => {
    const presets = [{
      name: 'Low',
      hidden_elements: [],
      delegated: {},
      ai: {
        block_a: { min_accuracy: 0.7, enabled: true, label: 'hello' },
      },
    }];
    let lastSet = null;
    const host = mount({
      presets,
      knownUiElements: [],
      activePresetIndex: 0,
      callbacks: {
        onSetAiParam: (i, b, k, v) => { lastSet = { i, b, k, v }; },
      },
    });

    const blocks = host.querySelectorAll('.def-ai-block');
    expect(blocks.length).toBe(1);
    expect(blocks[0].dataset.behaviorKey).toBe('block_a');

    const rows = blocks[0].querySelectorAll('.def-ai-param-row');
    expect(rows.length).toBe(3);

    // number row
    const numRow = rows.find((r) => r.dataset.paramKey === 'min_accuracy');
    const numInput = numRow.querySelector('.def-ai-param-input');
    expect(numInput.type).toBe('number');
    expect(numInput.step).toBe('0.1');
    expect(numInput.value).toBe('0.7');
    fireInput(numInput, '0.9');
    expect(lastSet).toEqual({ i: 0, b: 'block_a', k: 'min_accuracy', v: 0.9 });

    // boolean row
    const boolRow = rows.find((r) => r.dataset.paramKey === 'enabled');
    const boolInput = boolRow.querySelector('.def-ai-param-input');
    expect(boolInput.type).toBe('checkbox');
    expect(boolInput.checked).toBe(true);
    boolInput.checked = false;
    boolInput.dispatchEvent({ type: 'change', target: boolInput });
    expect(lastSet).toEqual({ i: 0, b: 'block_a', k: 'enabled', v: false });

    // string row
    const strRow = rows.find((r) => r.dataset.paramKey === 'label');
    const strInput = strRow.querySelector('.def-ai-param-input');
    expect(strInput.type).toBe('text');
    expect(strInput.value).toBe('hello');
    fireInput(strInput, 'world');
    expect(lastSet).toEqual({ i: 0, b: 'block_a', k: 'label', v: 'world' });
  });
});
