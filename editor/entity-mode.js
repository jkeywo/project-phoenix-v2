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
import { getComboTemplate, getRawSectionDefaults } from './component-templates.js';
import { computeEntityPreview } from './entity-preview.js';
import { sectionOrigin as provenanceSectionOrigin } from './entity-includes.js';

/**
 * Deep-clone a default value so subsequent mutations on the returned card
 * data don't leak back into the shared template defaults table.
 * Falls back to JSON clone if structuredClone is unavailable.
 * @param {any} value
 * @returns {any}
 */
function cloneDefaults(value) {
  if (value === null || value === undefined) return value;
  if (typeof value !== 'object') return value;
  if (typeof structuredClone === 'function') return structuredClone(value);
  return JSON.parse(JSON.stringify(value));
}

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

    /** @type {object|null} parsed TOML of the active file — the AUTHORED
     * document (keeps `includes`); the edit + save target. */
    this._parsedEntity = null;

    /** @type {object|null} the RESOLVED document (fragment fields merged in),
     * or null when the entity is uncomposed. Preview reads this so a
     * fragment-sourced hull does not appear to have no systems (issue #910). */
    this._resolvedEntity = null;

    /** @type {import('./entity-includes.js').Provenance|null} which fragment
     * authored each field of the resolved document. */
    this._provenance = null;

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
   *
   * @param {string} filePath
   * @param {string} tomlText  The hull's AUTHORED TOML (with `includes`).
   * @param {{ resolved?: object, provenance?: object }} [resolution]
   *   Optional include-resolution (issue #910). When supplied, `resolved` is the
   *   composed document preview reads, and `provenance` says which fragment
   *   authored each field. Omit it for an uncomposed entity — behaviour is then
   *   identical to before.
   * @returns {{ ok: boolean, errors: string[] }}
   */
  openFile(filePath, tomlText, resolution = null) {
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
    this._resolvedEntity = resolution?.resolved ?? null;
    this._provenance = resolution?.provenance ?? null;
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

  // ── Right pane: preview ──────────────────────────────────────────────────

  /**
   * Return preview pane data for the active entity, or a placeholder stub
   * when no file is open.
   * @returns {object|null}
   */
  getPreviewPane() {
    if (!this._parsedEntity) {
      return { placeholder: true, activeFile: this._activeFile };
    }
    // Preview the RESOLVED document (issue #910): a hull whose systems, hull or
    // consoles come from a fragment must not read as having none.
    return computeEntityPreview(this._resolvedEntity ?? this._parsedEntity, this._factionMap);
  }

  // ── Composition awareness (issue #910) ────────────────────────────────────

  /** The RESOLVED document (fragment fields merged in), or the authored one
   * when uncomposed. */
  getResolvedEntity() {
    return this._resolvedEntity ?? this._parsedEntity;
  }

  /** Provenance for the resolved document, or null when uncomposed. */
  getProvenance() {
    return this._provenance;
  }

  /** Whether the active entity is composed from at least one fragment. */
  isComposed() {
    return this._resolvedEntity != null && this._provenance != null;
  }

  /**
   * Classify a top-level section as authored by the hull, inherited from a
   * fragment, mixed, or absent — from provenance, not guesswork. Returns
   * `'authored'` for an uncomposed entity (every field is the hull's own).
   * @param {string} section
   * @returns {'authored'|'inherited'|'mixed'|'none'}
   */
  getSectionOrigin(section) {
    if (!this._provenance) {
      const doc = this._parsedEntity;
      return doc && doc[section] !== undefined && doc[section] !== null ? 'authored' : 'none';
    }
    return provenanceSectionOrigin(this._provenance, section, this._activeFile);
  }

  /**
   * Sections the resolved document carries that the hull does NOT author itself
   * — i.e. purely inherited from a fragment. A view renders these as inherited
   * (read-only until materialised); {@link materialiseSection} makes one
   * editable on the hull.
   * @returns {Array<{ section: string, origin: 'inherited' }>}
   */
  getInheritedSections() {
    if (!this._resolvedEntity || !this._parsedEntity) return [];
    const out = [];
    for (const key of Object.keys(this._resolvedEntity)) {
      const authored = this._parsedEntity[key];
      if (authored === undefined || authored === null) {
        out.push({ section: key, origin: 'inherited' });
      }
    }
    return out;
  }

  /**
   * Materialise an inherited section onto the hull's AUTHORED document so it
   * becomes an editable card. This is the DELIBERATE materialise-override
   * decision (issue #910, see entity-includes.js): editing an inherited field
   * is NOT read-only — it copies the resolved value onto the hull, which is the
   * same "includer wins" lever the resolver already honours, and the hull keeps
   * its `includes` so composition is preserved on save.
   * @param {string} section
   * @returns {{ ok: boolean, warning?: string }}
   */
  materialiseSection(section) {
    if (!this._parsedEntity) return { ok: false, warning: 'no entity open' };
    const source = this._resolvedEntity ?? this._parsedEntity;
    if (source[section] === undefined || source[section] === null) {
      return { ok: false, warning: `section '${section}' is not present to materialise` };
    }
    const value = cloneDefaults(source[section]);
    this._parsedEntity[section] = value;
    this._cards = this._buildCards(this._parsedEntity);
    return { ok: true };
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

  // ── Add component / combo ─────────────────────────────────────────────────

  /**
   * Add a single raw section with its schema defaults as a new ComponentCard.
   * If the section is already present, this is a no-op and a warning is logged.
   *
   * @param {string} sectionKey  e.g. 'hull', 'helm_console'
   * @returns {{ ok: boolean, warning?: string }}
   */
  addComponent(sectionKey) {
    const existing = this._cards.find((c) => c.section === sectionKey);
    if (existing) {
      const msg = `addComponent: section '${sectionKey}' is already present — skipping`;
      console.warn(msg);
      return { ok: false, warning: msg };
    }

    const defaults = cloneDefaults(getRawSectionDefaults(sectionKey) ?? {});
    const schema = COMPONENT_SCHEMA[sectionKey] ?? null;
    if (this._parsedEntity) {
      this._parsedEntity[sectionKey] = defaults;
    }
    const card = new ComponentCard(sectionKey, defaults, schema);
    this._cards.push(card);
    return { ok: true };
  }

  /**
   * Add all sections defined by a combo template as new ComponentCards.
   * Sections that are already present are skipped (no-op with a warning per
   * skipped section).  Unknown combo names produce a warning and return ok: false.
   *
   * @param {string} comboName  e.g. 'Ship', 'Station', 'Asteroid Field'
   * @returns {{ ok: boolean, warnings: string[] }}
   */
  addCombo(comboName) {
    const template = getComboTemplate(comboName);
    if (!template) {
      const msg = `addCombo: unknown combo '${comboName}'`;
      console.warn(msg);
      return { ok: false, warnings: [msg] };
    }

    const warnings = [];
    for (const { key, defaults } of template.sections) {
      const existing = this._cards.find((c) => c.section === key);
      if (existing) {
        const msg = `addCombo '${comboName}': section '${key}' is already present — skipping`;
        console.warn(msg);
        warnings.push(msg);
        continue;
      }
      const schema = COMPONENT_SCHEMA[key] ?? null;
      const data = cloneDefaults(defaults);
      if (this._parsedEntity) {
        this._parsedEntity[key] = data;
      }
      const card = new ComponentCard(key, data, schema);
      this._cards.push(card);
    }

    return { ok: true, warnings };
  }

  // ── Mutation: section update + parsed-state restoration ──────────────────

  /**
   * Replace one section's data in the active entity. The card list is
   * rebuilt so any newly-present or newly-removed section shows up.
   * No-ops if no file is open.
   * @param {string} sectionKey
   * @param {any} newData
   */
  setSection(sectionKey, newData) {
    if (!this._parsedEntity) return;
    if (newData === undefined || newData === null) {
      delete this._parsedEntity[sectionKey];
    } else {
      this._parsedEntity[sectionKey] = newData;
    }
    this._cards = this._buildCards(this._parsedEntity);
  }

  /**
   * Replace the entire parsed entity (used by the undo restore callback).
   * Card list is rebuilt. No-ops if `parsed` is falsy.
   * @param {object} parsed
   */
  restoreParsed(parsed) {
    if (!parsed || typeof parsed !== 'object') return;
    this._parsedEntity = parsed;
    this._cards = this._buildCards(parsed);
  }

  /** Internal getter used by the view to read the live parsed object. */
  getParsedEntity() {
    return this._parsedEntity;
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
