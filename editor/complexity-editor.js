import { parse, stringify } from 'smol-toml';

/**
 * complexity-editor.js
 *
 * Pure-logic data model for the Definitions Mode complexity preset editor:
 *   Left pane  — file list (assets/complexity/*.toml)
 *   Centre pane — preset list + structured editors for each preset:
 *                   hidden_elements  — multi-select of UI element names
 *                   delegated        — table of control → delegate console
 *                   ai               — key/value tuning blocks (kind-specific)
 *
 * No DOM manipulation is performed here; this module exports plain JS
 * classes/functions that can be tested in Node without a browser.
 */

// ── Known UI element names per console (used for hidden_elements multi-select) ─

/**
 * Map of console name (derived from complexity filename) → known hideable element IDs.
 * Derived from the actual values used in the shipped complexity TOML files.
 */
export const KNOWN_UI_ELEMENTS = {
  tactical: [
    'phaser_mode_selector',
    'torpedo_tube_selector',
    'target_lock_button',
    'fire_confirm',
    'auto_fire_toggle',
  ],
  power: [
    'power_overflow_controls',
    'battery_level_readout',
    'warp_power_slider',
  ],
  helm: [
    'warp_drive_control',
    'impulse_slider',
    'course_lock_button',
  ],
  // Includes the elements formerly listed under "science" — the Science
  // console's complexity file was merged into sensors.toml.
  sensors: [
    'sensor_range_slider',
    'passive_scan_toggle',
    'shield_frequency_readout',
    'science_scan_button',
    'beam_aim_assist',
  ],
  shields: [
    'shield_arc_selector',
    'shield_boost_button',
  ],
  navigation: [
    'waypoint_editor',
    'autopilot_toggle',
  ],
};

// ── TOML parse / stringify helpers ────────────────────────────────────────────

/**
 * Parse a complexity TOML string.
 * Returns an array of preset objects, each shaped:
 *   { name, hidden_elements, delegated, ai }
 *
 * @param {string} text
 * @returns {Array<ComplexityPreset>}
 */
export function parseComplexityToml(text) {
  const obj = parse(text);
  if (!Array.isArray(obj.preset)) {
    throw new Error('Complexity TOML must contain [[preset]] array');
  }
  return obj.preset.map(normalisePreset);
}

/**
 * Serialize an array of preset objects back to a TOML string.
 * Preserves [[preset]] block shape (array of tables).
 *
 * @param {Array<ComplexityPreset>} presets
 * @returns {string}
 */
export function stringifyComplexityToml(presets) {
  const obj = {
    preset: presets.map(serialisePreset),
  };
  return stringify(obj);
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/**
 * Normalise a raw parsed preset object into a clean ComplexityPreset shape.
 * @param {object} raw
 * @returns {ComplexityPreset}
 */
function normalisePreset(raw) {
  return {
    name: String(raw.name ?? ''),
    hidden_elements: Array.isArray(raw.hidden_elements)
      ? raw.hidden_elements.map(String)
      : [],
    delegated: normaliseDelegated(raw.delegated),
    ai: normaliseAi(raw.ai),
  };
}

/**
 * Normalise the `delegated` section.
 * Shape: { [consoleKey]: { controls: string[] } }
 * @param {object|undefined} raw
 * @returns {Object.<string, {controls: string[]}>}
 */
function normaliseDelegated(raw) {
  if (!raw || typeof raw !== 'object') return {};
  const result = {};
  for (const [key, val] of Object.entries(raw)) {
    result[key] = {
      controls: Array.isArray(val?.controls)
        ? val.controls.map(String)
        : [],
    };
  }
  return result;
}

/**
 * Normalise the `ai` section.
 * Shape: { [behaviorKey]: { [paramKey]: number|boolean|string } }
 * @param {object|undefined} raw
 * @returns {Object.<string, Object.<string, any>>}
 */
function normaliseAi(raw) {
  if (!raw || typeof raw !== 'object') return {};
  const result = {};
  for (const [key, val] of Object.entries(raw)) {
    result[key] = val && typeof val === 'object' ? { ...val } : {};
  }
  return result;
}

/**
 * Serialise a normalised preset back to a plain object suitable for stringify.
 * @param {ComplexityPreset} preset
 * @returns {object}
 */
function serialisePreset(preset) {
  const obj = { name: preset.name };

  if (preset.hidden_elements.length > 0) {
    obj.hidden_elements = [...preset.hidden_elements];
  } else {
    obj.hidden_elements = [];
  }

  if (Object.keys(preset.delegated).length > 0) {
    obj.delegated = {};
    for (const [key, val] of Object.entries(preset.delegated)) {
      obj.delegated[key] = { controls: [...val.controls] };
    }
  }

  if (Object.keys(preset.ai).length > 0) {
    obj.ai = {};
    for (const [key, val] of Object.entries(preset.ai)) {
      obj.ai[key] = { ...val };
    }
  }

  return obj;
}

// ── ComplexityEditor ──────────────────────────────────────────────────────────

/**
 * @typedef {{ name: string, hidden_elements: string[], delegated: Object.<string, {controls: string[]}>, ai: Object.<string, Object.<string, any>> }} ComplexityPreset
 */

/**
 * Data model for the complexity preset editor pane.
 *
 * Usage:
 *   const editor = new ComplexityEditor();
 *   editor.loadAll([
 *     { path: 'assets/complexity/tactical.toml', content: '...' },
 *     ...
 *   ]);
 *   editor.openFile('assets/complexity/tactical.toml');
 *   const presets = editor.getPresets();
 *   editor.setHiddenElements(0, ['phaser_mode_selector']);
 *   editor.setDelegated(0, 'Tactical', ['auto_fire_torpedoes']);
 *   editor.setAiParam(0, 'torpedo_auto_fire', 'min_accuracy', 0.8);
 *   const toml = editor.serialize();
 */
export class ComplexityEditor {
  constructor() {
    /** @type {string[]} ordered list of file paths */
    this._fileList = [];

    /**
     * Map of path → raw content string
     * @type {Map<string, string>}
     */
    this._rawContents = new Map();

    /** @type {string|null} currently active file path */
    this._activeFile = null;

    /**
     * Mutable presets for the currently open file.
     * @type {ComplexityPreset[]|null}
     */
    this._presets = null;
  }

  // ── Load ──────────────────────────────────────────────────────────────────

  /**
   * Load all complexity files into the editor.
   * Clears any previously loaded data.
   *
   * @param {Array<{ path: string, content: string }>} files
   */
  loadAll(files) {
    this._fileList = [];
    this._rawContents = new Map();
    this._activeFile = null;
    this._presets = null;

    for (const { path, content } of files) {
      this._rawContents.set(path, content);
      this._fileList.push(path);
    }
  }

  // ── File list ─────────────────────────────────────────────────────────────

  /**
   * Return a copy of all complexity file paths (left pane).
   * @returns {string[]}
   */
  getFileList() {
    return [...this._fileList];
  }

  /**
   * Return the currently active file path, or null.
   * @returns {string|null}
   */
  getActiveFile() {
    return this._activeFile;
  }

  // ── Open / preset list ────────────────────────────────────────────────────

  /**
   * Open a complexity file and parse its presets.
   * @param {string} path
   * @returns {boolean} true if the file was found and opened
   */
  openFile(path) {
    const content = this._rawContents.get(path);
    if (content === undefined) return false;

    try {
      this._presets = parseComplexityToml(content);
      this._activeFile = path;
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Return a deep copy of the current preset list, or null if no file is open.
   * @returns {ComplexityPreset[]|null}
   */
  getPresets() {
    if (!this._presets) return null;
    return this._presets.map(deepCopyPreset);
  }

  /**
   * Return a deep copy of a single preset by index, or null.
   * @param {number} index
   * @returns {ComplexityPreset|null}
   */
  getPreset(index) {
    if (!this._presets || index < 0 || index >= this._presets.length) return null;
    return deepCopyPreset(this._presets[index]);
  }

  // ── hidden_elements ───────────────────────────────────────────────────────

  /**
   * Replace the hidden_elements list for a preset.
   * No-op if no file is open or index is out of range.
   * @param {number} presetIndex
   * @param {string[]} elements
   */
  setHiddenElements(presetIndex, elements) {
    if (!this._presets || presetIndex < 0 || presetIndex >= this._presets.length) return;
    this._presets[presetIndex].hidden_elements = [...elements];
  }

  /**
   * Return the known UI element names for the active file's console.
   * Falls back to an empty array for unknown consoles.
   * @returns {string[]}
   */
  getKnownUiElements() {
    if (!this._activeFile) return [];
    const consoleName = this._activeFile.split('/').pop().replace('.toml', '').toLowerCase();
    return KNOWN_UI_ELEMENTS[consoleName] ?? [];
  }

  // ── delegated ─────────────────────────────────────────────────────────────

  /**
   * Set the list of delegated controls for a console key in a preset.
   * Creates the delegated entry if it doesn't exist.
   * No-op if no file is open or index is out of range.
   * @param {number} presetIndex
   * @param {string} consoleKey  e.g. "Tactical"
   * @param {string[]} controls
   */
  setDelegated(presetIndex, consoleKey, controls) {
    if (!this._presets || presetIndex < 0 || presetIndex >= this._presets.length) return;
    this._presets[presetIndex].delegated[consoleKey] = { controls: [...controls] };
  }

  /**
   * Remove a console key from the delegated block of a preset.
   * No-op if the key doesn't exist.
   * @param {number} presetIndex
   * @param {string} consoleKey
   */
  removeDelegated(presetIndex, consoleKey) {
    if (!this._presets || presetIndex < 0 || presetIndex >= this._presets.length) return;
    delete this._presets[presetIndex].delegated[consoleKey];
  }

  // ── ai tuning ─────────────────────────────────────────────────────────────

  /**
   * Set a single AI tuning parameter.
   * Creates the behavior entry if it doesn't exist.
   * No-op if no file is open or index is out of range.
   * @param {number} presetIndex
   * @param {string} behaviorKey  e.g. "torpedo_auto_fire"
   * @param {string} paramKey     e.g. "min_accuracy"
   * @param {number|boolean|string} value
   */
  setAiParam(presetIndex, behaviorKey, paramKey, value) {
    if (!this._presets || presetIndex < 0 || presetIndex >= this._presets.length) return;
    const ai = this._presets[presetIndex].ai;
    if (!ai[behaviorKey]) ai[behaviorKey] = {};
    ai[behaviorKey][paramKey] = value;
  }

  /**
   * Replace all params for a given AI behavior block.
   * @param {number} presetIndex
   * @param {string} behaviorKey
   * @param {Object.<string, any>} params
   */
  setAiBlock(presetIndex, behaviorKey, params) {
    if (!this._presets || presetIndex < 0 || presetIndex >= this._presets.length) return;
    this._presets[presetIndex].ai[behaviorKey] = { ...params };
  }

  /**
   * Remove an AI behavior block from a preset.
   * @param {number} presetIndex
   * @param {string} behaviorKey
   */
  removeAiBlock(presetIndex, behaviorKey) {
    if (!this._presets || presetIndex < 0 || presetIndex >= this._presets.length) return;
    delete this._presets[presetIndex].ai[behaviorKey];
  }

  // ── Serialization ─────────────────────────────────────────────────────────

  /**
   * Serialize the current presets to a TOML string preserving [[preset]] shape.
   * Throws if no file is open.
   * @returns {string}
   */
  serialize() {
    if (!this._presets) {
      throw new Error('No complexity file is open');
    }
    return stringifyComplexityToml(this._presets);
  }
}

// ── Private deep-copy helper ──────────────────────────────────────────────────

/**
 * Deep copy a ComplexityPreset to prevent external mutation of internal state.
 * @param {ComplexityPreset} preset
 * @returns {ComplexityPreset}
 */
function deepCopyPreset(preset) {
  const delegated = {};
  for (const [key, val] of Object.entries(preset.delegated)) {
    delegated[key] = { controls: [...val.controls] };
  }
  const ai = {};
  for (const [key, val] of Object.entries(preset.ai)) {
    ai[key] = { ...val };
  }
  return {
    name: preset.name,
    hidden_elements: [...preset.hidden_elements],
    delegated,
    ai,
  };
}
