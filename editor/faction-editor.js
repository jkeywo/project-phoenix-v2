import { parse, stringify } from 'smol-toml';

/**
 * faction-editor.js
 *
 * Pure-logic data model for the Definitions Mode faction editor:
 *   Left pane  — file list (assets/factions/*.toml)
 *   Centre pane — form for the selected faction (uuid read-only, name editable,
 *                 enemies multi-select from other factions by name)
 *
 * No DOM manipulation is performed here; this module exports plain JS
 * classes/functions that can be tested in Node without a browser.
 */

// ── TOML helpers ──────────────────────────────────────────────────────────────

/**
 * Parse a faction TOML string into a plain JS object.
 * @param {string} text
 * @returns {{ uuid: string, name: string, enemies: string[] }}
 */
export function parseFactionToml(text) {
  const obj = parse(text);
  if (!obj.uuid || typeof obj.uuid !== 'string') {
    throw new Error('Faction TOML must have a string uuid field');
  }
  if (!obj.name || typeof obj.name !== 'string') {
    throw new Error('Faction TOML must have a string name field');
  }
  if (!Array.isArray(obj.enemies)) {
    throw new Error('Faction TOML must have an enemies array');
  }
  return {
    uuid: obj.uuid,
    name: obj.name,
    enemies: obj.enemies.map(String),
  };
}

/**
 * Serialize a faction data object back to a TOML string.
 * enemies is always written as a UUID array.
 * @param {{ uuid: string, name: string, enemies: string[] }} faction
 * @returns {string}
 */
export function stringifyFactionToml(faction) {
  return stringify({
    uuid: faction.uuid,
    name: faction.name,
    enemies: faction.enemies,
  });
}

// ── FactionEditor ─────────────────────────────────────────────────────────────

/**
 * Data model for the faction editor pane.
 *
 * Usage:
 *   const editor = new FactionEditor();
 *   editor.loadAll([
 *     { path: 'assets/factions/federation.toml', content: '...' },
 *     ...
 *   ]);
 *   editor.openFile('assets/factions/federation.toml');
 *   editor.setName('New Federation');
 *   editor.setEnemies(['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb']);
 *   const toml = editor.serialize();
 */
export class FactionEditor {
  constructor() {
    /** @type {string[]} ordered list of faction file paths */
    this._fileList = [];

    /**
     * Map of path → parsed faction data
     * @type {Map<string, { uuid: string, name: string, enemies: string[] }>}
     */
    this._factions = new Map();

    /** @type {string|null} currently active file path */
    this._activeFile = null;

    /**
     * Mutable form state for the currently open faction.
     * @type {{ uuid: string, name: string, enemies: string[] }|null}
     */
    this._formState = null;
  }

  // ── Load ──────────────────────────────────────────────────────────────────

  /**
   * Load all faction files into the editor.
   * Clears any previously loaded data.
   *
   * @param {Array<{ path: string, content: string }>} files
   *   Each element is { path: 'assets/factions/foo.toml', content: '<toml text>' }
   */
  loadAll(files) {
    this._fileList = [];
    this._factions = new Map();
    this._activeFile = null;
    this._formState = null;

    for (const { path, content } of files) {
      try {
        const faction = parseFactionToml(content);
        this._factions.set(path, faction);
        this._fileList.push(path);
      } catch {
        // skip malformed faction files
      }
    }
  }

  // ── File list ─────────────────────────────────────────────────────────────

  /**
   * Return a copy of all faction file paths (left pane).
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

  // ── Open / form state ─────────────────────────────────────────────────────

  /**
   * Open a faction file and populate the form state.
   * @param {string} path
   * @returns {boolean} true if the file was found and opened
   */
  openFile(path) {
    const faction = this._factions.get(path);
    if (!faction) return false;
    this._activeFile = path;
    this._formState = {
      uuid: faction.uuid,
      name: faction.name,
      enemies: [...faction.enemies],
    };
    return true;
  }

  /**
   * Return the current form state, or null if no file is open.
   * @returns {{ uuid: string, name: string, enemies: string[] }|null}
   */
  getFormState() {
    if (!this._formState) return null;
    return {
      uuid: this._formState.uuid,
      name: this._formState.name,
      enemies: [...this._formState.enemies],
    };
  }

  // ── Form mutations ────────────────────────────────────────────────────────

  /**
   * Update the name field in the form state.
   * No-op if no file is open.
   * @param {string} name
   */
  setName(name) {
    if (!this._formState) return;
    this._formState.name = name;
  }

  /**
   * Replace the enemies list in the form state with an array of UUIDs.
   * No-op if no file is open.
   * @param {string[]} uuids
   */
  setEnemies(uuids) {
    if (!this._formState) return;
    this._formState.enemies = [...uuids];
  }

  // ── Enemy multi-select options ────────────────────────────────────────────

  /**
   * Return enemy option objects for the multi-select — every other faction
   * except the one currently open, displayed by name.
   *
   * @returns {Array<{ uuid: string, name: string, path: string }>}
   */
  getEnemyOptions() {
    const options = [];
    for (const [path, faction] of this._factions) {
      if (path === this._activeFile) continue;
      options.push({ uuid: faction.uuid, name: faction.name, path });
    }
    return options;
  }

  // ── Serialization ─────────────────────────────────────────────────────────

  /**
   * Serialize the current form state to a TOML string.
   * enemies is always written as a UUID array.
   * Throws if no file is open.
   * @returns {string}
   */
  serialize() {
    if (!this._formState) {
      throw new Error('No faction file is open');
    }
    return stringifyFactionToml(this._formState);
  }
}
