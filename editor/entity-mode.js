/**
 * entity-mode.js
 *
 * Pure-logic data model for the Entity Mode three-pane shell:
 *   Left pane  — file list (assets/entities/*.toml)
 *   Centre pane — component cards for the active entity
 *   Right pane  — preview pane (placeholder)
 *
 * No DOM manipulation is performed here; this module exports plain JS
 * classes/functions that can be tested in Node without a browser.
 */

import { parseEntityToml, stringifyEntityToml, validateEntitySections } from './entity-toml.js';
import { COMPONENT_SCHEMA, ENTITY_CONFIG_SECTIONS } from './component-schema.js';

// ── Component Card ────────────────────────────────────────────────────────────

/**
 * Represents a single TOML section rendered as a collapsible card.
 */
export class ComponentCard {
  /**
   * @param {string} section  TOML key (e.g. 'hull', 'helm_console')
   * @param {object} data     Parsed section data from the entity TOML
   * @param {object} schema   Schema entry from COMPONENT_SCHEMA
   */
  constructor(section, data, schema) {
    this.section = section;
    this.data = data;
    this.schema = schema;
    this._collapsed = false;
    this._showRaw = false;
  }

  /** Whether the card is collapsed (header visible, body hidden). */
  get collapsed() { return this._collapsed; }
  toggle() { this._collapsed = !this._collapsed; }

  /** Whether the card is showing raw TOML instead of structured fields. */
  get showRaw() { return this._showRaw; }
  toggleRaw() { this._showRaw = !this._showRaw; }

  /** Return the raw TOML string for just this section. */
  getRawToml() {
    const wrapper = { [this.section]: this.data };
    return stringifyEntityToml(wrapper);
  }

  /**
   * Return fields that have a faction dropdown source, paired with the
   * current value in this card's data.
   * @returns {Array<{key: string, value: any}>}
   */
  getFactionFields() {
    return (this.schema?.fields ?? [])
      .filter((f) => f.dropdownSource === 'factions')
      .map((f) => ({ key: f.key, value: this.data?.[f.key] ?? null }));
  }

  /**
   * Return fields that have a complexity dropdown source, paired with the
   * current value in this card's data.
   * @returns {Array<{key: string, value: any}>}
   */
  getComplexityFields() {
    return (this.schema?.fields ?? [])
      .filter((f) => f.dropdownSource === 'complexity')
      .map((f) => ({ key: f.key, value: this.data?.[f.key] ?? null }));
  }
}

// ── Entity Mode Shell ─────────────────────────────────────────────────────────

/**
 * Three-pane shell data model for Entity Mode.
 *
 * Usage:
 *   const mode = new EntityModeShell();
 *   mode.setFileList(['assets/entities/player_ship.toml', ...]);
 *   mode.setFactionMap(new Map([['uuid1', 'Federation'], ...]));
 *   mode.setComplexityPaths(['assets/complexity/tactical.toml', ...]);
 *   mode.openFile('assets/entities/player_ship.toml', tomlText);
 *   const cards = mode.getComponentCards();
 */
export class EntityModeShell {
  constructor() {
    /** @type {string[]} all entity TOML paths known to the editor */
    this._fileList = [];

    /** @type {string|null} currently active file path */
    this._activeFile = null;

    /** @type {object|null} parsed TOML of the active file */
    this._parsedEntity = null;

    /** @type {string|null} raw TOML text of the active file */
    this._rawText = null;

    /** @type {Map<string, string>} uuid → faction name */
    this._factionMap = new Map();

    /** @type {string[]} available complexity paths */
    this._complexityPaths = [];

    /** @type {ComponentCard[]} cards for the current entity */
    this._cards = [];
  }

  // ── Left pane: file list ──────────────────────────────────────────────────

  /** Set the full list of entity file paths (left pane). */
  setFileList(paths) {
    this._fileList = [...paths];
  }

  /** Return a copy of the entity file list. */
  getFileList() {
    return [...this._fileList];
  }

  /** Return the currently active file path, or null. */
  getActiveFile() {
    return this._activeFile;
  }

  // ── Centre pane: open + component cards ──────────────────────────────────

  /**
   * Open an entity file: parse it and build component cards.
   * @param {string} filePath
   * @param {string} tomlText
   * @returns {{ ok: boolean, errors: string[] }}
   */
  openFile(filePath, tomlText) {
    let parsed;
    try {
      parsed = parseEntityToml(tomlText);
    } catch (err) {
      return { ok: false, errors: [`TOML parse error: ${err.message}`] };
    }

    const sectionValidation = validateEntitySections(parsed);
    if (!sectionValidation.valid) {
      return { ok: false, errors: sectionValidation.errors };
    }

    this._activeFile = filePath;
    this._rawText = tomlText;
    this._parsedEntity = parsed;
    this._cards = this._buildCards(parsed);
    return { ok: true, errors: [] };
  }

  /**
   * Return all component cards for the active entity.
   * @returns {ComponentCard[]}
   */
  getComponentCards() {
    return [...this._cards];
  }

  /**
   * Return the card for a given section key, or null.
   * @param {string} section
   * @returns {ComponentCard|null}
   */
  getCard(section) {
    return this._cards.find((c) => c.section === section) ?? null;
  }

  // ── Right pane: preview placeholder ──────────────────────────────────────

  /**
   * Return preview pane data.  Currently a placeholder stub.
   * @returns {{ placeholder: true, activeFile: string|null }}
   */
  getPreviewPane() {
    return { placeholder: true, activeFile: this._activeFile };
  }

  // ── Faction dropdown ──────────────────────────────────────────────────────

  /**
   * Set the uuid→name faction map (built from assets/factions/*.toml).
   * @param {Map<string, string>} map
   */
  setFactionMap(map) {
    this._factionMap = new Map(map);
  }

  /** Return a copy of the faction map. */
  getFactionMap() {
    return new Map(this._factionMap);
  }

  /**
   * Resolve a faction UUID to its display name, or return the UUID itself.
   * @param {string} uuid
   * @returns {string}
   */
  resolveFactionName(uuid) {
    return this._factionMap.get(uuid) ?? uuid;
  }

  /**
   * Return an array of { uuid, name } for all known factions (for dropdowns).
   * @returns {Array<{uuid: string, name: string}>}
   */
  getFactionDropdownOptions() {
    return [...this._factionMap.entries()].map(([uuid, name]) => ({ uuid, name }));
  }

  // ── Complexity dropdown ───────────────────────────────────────────────────

  /**
   * Set available complexity TOML paths (built from assets/complexity/*.toml).
   * @param {string[]} paths
   */
  setComplexityPaths(paths) {
    this._complexityPaths = [...paths];
  }

  /** Return all complexity paths (for dropdowns). */
  getComplexityPaths() {
    return [...this._complexityPaths];
  }

  // ── Private helpers ───────────────────────────────────────────────────────

  /**
   * Build ComponentCard array from a parsed entity object.
   * Only sections that are present in the parsed data are included.
   * Order follows ENTITY_CONFIG_SECTIONS.
   * @param {object} parsed
   * @returns {ComponentCard[]}
   */
  _buildCards(parsed) {
    const cards = [];
    for (const section of ENTITY_CONFIG_SECTIONS) {
      // tags is always present (may be empty array)
      const value = parsed[section];
      const hasTags = section === 'tags' && Array.isArray(value);
      const hasSection = value !== undefined && value !== null;
      if (!hasTags && !hasSection) continue;
      if (section === 'tags' && Array.isArray(value) && value.length === 0) continue;

      const schema = COMPONENT_SCHEMA[section] ?? null;
      cards.push(new ComponentCard(section, value, schema));
    }

    // Also pick up any extra sections not in the canonical list (e.g. 'stations', 'weapons')
    for (const key of Object.keys(parsed)) {
      if (!ENTITY_CONFIG_SECTIONS.includes(key) && parsed[key] !== null && parsed[key] !== undefined) {
        const schema = COMPONENT_SCHEMA[key] ?? null;
        cards.push(new ComponentCard(key, parsed[key], schema));
      }
    }

    return cards;
  }
}
