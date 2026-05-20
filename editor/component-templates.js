/**
 * component-templates.js
 *
 * Common-combo templates for the "+ Add component" picker in Entity Mode.
 *
 * Each combo template specifies the set of TOML sections (with default values)
 * that together define a well-formed entity of that archetype.
 *
 * Exports:
 *   COMBO_TEMPLATES      — map of name → { sections: [{ key, defaults }] }
 *   getComboTemplate(name)   — lookup a single combo by name
 *   getAllComboNames()        — ordered list of all combo names
 *   getRawSectionDefaults(sectionKey) — default data object for a raw section
 */

import { COMPONENT_SCHEMA, ENTITY_CONFIG_SECTIONS } from './component-schema.js';

// ── Raw section defaults ──────────────────────────────────────────────────────

/**
 * Build a default data object for a section from its schema fields.
 * Fields with a `default` value contribute that value; optional fields without
 * a default are omitted.
 *
 * @param {string} sectionKey
 * @returns {object|null} defaults object, or null if the section is unknown
 */
export function getRawSectionDefaults(sectionKey) {
  const schema = COMPONENT_SCHEMA[sectionKey];
  if (!schema) return null;

  const obj = {};
  for (const field of schema.fields) {
    if ('default' in field) {
      obj[field.key] = field.default;
    }
    // optional fields without a default are not included
  }
  return obj;
}

// ── Combo templates ───────────────────────────────────────────────────────────

/**
 * COMBO_TEMPLATES
 *
 * Each entry is `name → { sections: [{ key: string, defaults: object }] }`.
 *
 * `defaults` is a plain JS object that will be used as the initial data for
 * the new ComponentCard.  It mirrors what a minimal but valid TOML would
 * produce after a parse → stringify round-trip.
 */
export const COMBO_TEMPLATES = {
  Ship: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['ship'] },
      },
      {
        key: 'collider',
        defaults: { shape: 'Capsule', radius: 3.0, length: 6.0 },
      },
      {
        key: 'hull',
        defaults: { hull_integrity: 100.0, repair_team_count: 0, console_hull: [] },
      },
      {
        key: 'helm_console',
        defaults: {
          max_speed: 50.0,
          max_reverse_speed: 0.0,
          acceleration: 16.7,
          deceleration: 50.0,
          max_yaw_rate: 1.5708,
          radar_range: 0,
          radar_shows: false,
          impulse_charge_duration: 3.0,
          impulse_speed_multiplier: 10.0,
        },
      },
      {
        key: 'radar_appearance',
        defaults: { colour: [0.6, 0.8, 1.0] },
      },
    ],
  },

  Station: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['station'] },
      },
      {
        key: 'station',
        defaults: {
          name: 'New Station',
          shape: 'torus',
          radius: 18.0,
          hull_integrity: 200.0,
        },
      },
      {
        key: 'radar_appearance',
        defaults: { colour: [0.3, 0.8, 0.6] },
      },
    ],
  },

  Region: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['region'] },
      },
      {
        key: 'shape',
        defaults: { type: 'sphere', radius: 100.0 },
      },
      {
        key: 'effects',
        defaults: {},
      },
    ],
  },

  NPC: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['ship', 'npc'] },
      },
      {
        key: 'collider',
        defaults: { shape: 'Capsule', radius: 2.0, length: 4.0 },
      },
      {
        key: 'hull',
        defaults: { hull_integrity: 60.0, repair_team_count: 0, console_hull: [] },
      },
      {
        key: 'helm_console',
        defaults: {
          max_speed: 50.0,
          max_reverse_speed: 0.0,
          acceleration: 16.7,
          deceleration: 50.0,
          max_yaw_rate: 1.5708,
          radar_range: 0,
          radar_shows: false,
          impulse_charge_duration: 3.0,
          impulse_speed_multiplier: 10.0,
        },
      },
      {
        key: 'radar_appearance',
        defaults: { colour: [1.0, 0.2, 0.2] },
      },
      {
        key: 'behaviour',
        defaults: { initial_state: 'idle', state: [], transition: [] },
      },
    ],
  },

  Asteroid: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['asteroid', 'gameplay'] },
      },
      {
        key: 'collider',
        defaults: { shape: 'Ball', radius: 5.0, length: 0.0 },
      },
      {
        key: 'hull',
        defaults: { hull_integrity: 50.0, repair_team_count: 0, console_hull: [] },
      },
    ],
  },

  'Asteroid Field': {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['asteroid_field'] },
      },
      {
        key: 'asteroid_field',
        defaults: {
          inner_radius: 100.0,
          outer_radius: 200.0,
          density: 0.005,
          spawn_distance: 150.0,
          despawn_distance: 250.0,
          asteroid_type_paths: [],
          cosmetic_type_paths: [],
          tags: [],
        },
      },
    ],
  },

  Star: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['star'] },
      },
      {
        key: 'star',
        defaults: {
          name: 'New Star',
          radius: 50.0,
          colour: [1.0, 0.8, 0.0],
          position: [0.0, 0.0, 0.0],
          tags: [],
        },
      },
      {
        key: 'collider',
        defaults: { shape: 'Ball', radius: 50.0, length: 0.0 },
      },
      {
        key: 'radar_appearance',
        defaults: { colour: [1.0, 0.85, 0.3] },
      },
    ],
  },

  Planet: {
    sections: [
      {
        key: 'tags',
        defaults: { tags: ['planet'] },
      },
      {
        key: 'planet',
        defaults: {
          name: 'New Planet',
          radius: 20.0,
          colour: [0.0, 0.5, 1.0],
          position: [0.0, 0.0, 0.0],
          tags: [],
        },
      },
      {
        key: 'collider',
        defaults: { shape: 'Ball', radius: 20.0, length: 0.0 },
      },
      {
        key: 'radar_appearance',
        defaults: { colour: [0.0, 0.6, 1.0] },
      },
    ],
  },
};

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Return the combo template for the given name, or null if not found.
 * @param {string} name  e.g. 'Ship', 'Station', 'Asteroid Field'
 * @returns {{ sections: Array<{ key: string, defaults: object }> }|null}
 */
export function getComboTemplate(name) {
  return COMBO_TEMPLATES[name] ?? null;
}

/**
 * Return the ordered list of all combo template names.
 * Order matches the display order in the picker.
 * @returns {string[]}
 */
export function getAllComboNames() {
  return Object.keys(COMBO_TEMPLATES);
}

/**
 * Return a composed picker data model for the two-tier component picker UI.
 *
 * The top tier contains combo entries (grouped archetypes); the second tier
 * contains raw section entries drawn from ENTITY_CONFIG_SECTIONS.
 *
 * @returns {{
 *   combos: Array<{ name: string, label: string }>,
 *   rawSections: Array<{ key: string, label: string }>
 * }}
 */
export function getPickerModel() {
  return {
    combos: getAllComboNames().map(name => ({ name, label: name })),
    rawSections: ENTITY_CONFIG_SECTIONS.map(key => ({
      key,
      label: COMPONENT_SCHEMA[key]?.label ?? key
    }))
  };
}
