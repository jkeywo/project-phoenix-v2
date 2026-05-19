/**
 * component-schema.js
 *
 * Pure schema data for every TOML section in EntityConfig (src/entities/config.rs).
 * Each entry describes the fields of a top-level TOML section so the editor can
 * render structured component cards.
 *
 * Field descriptor shape:
 *   { key, type, default?, optional?, items?, dropdownSource? }
 *
 *   type: 'string' | 'number' | 'boolean' | 'array' | 'object' | 'uuid-faction' | 'path-complexity'
 *   items: type of array elements (for type:'array')
 *   dropdownSource: 'factions' | 'complexity' (drives dropdown resolution)
 *   optional: true if the field may be omitted from TOML
 */

export const COMPONENT_SCHEMA = {
  tags: {
    section: 'tags',
    label: 'Tags',
    fields: [
      { key: 'tags', type: 'array', items: 'string', default: [] },
    ],
  },

  faction: {
    section: 'faction',
    label: 'Faction',
    fields: [
      { key: 'faction', type: 'uuid-faction', optional: true, dropdownSource: 'factions' },
    ],
  },

  hull: {
    section: 'hull',
    label: 'Hull',
    fields: [
      { key: 'hull_integrity', type: 'number', default: 0, optional: true },
      { key: 'captain_chair', type: 'number', optional: true },
      { key: 'repair_team_count', type: 'number', default: 0, optional: true },
      { key: 'console_hull', type: 'array', items: 'object', optional: true, default: [] },
    ],
  },

  collider: {
    section: 'collider',
    label: 'Collider',
    fields: [
      { key: 'shape', type: 'string', enum: ['Ball', 'Capsule'] },
      { key: 'radius', type: 'number' },
      { key: 'length', type: 'number', default: 0 },
    ],
  },

  appearance: {
    section: 'appearance',
    label: 'Appearance',
    fields: [
      { key: 'colour', type: 'string' },
      { key: 'size_min', type: 'number' },
      { key: 'size_max', type: 'number' },
    ],
  },

  radar_appearance: {
    section: 'radar_appearance',
    label: 'Radar Appearance',
    fields: [
      { key: 'colour', type: 'array', items: 'number' },
      { key: 'radius', type: 'number', optional: true },
    ],
  },

  helm_console: {
    section: 'helm_console',
    label: 'Helm Console',
    fields: [
      { key: 'max_speed', type: 'number', default: 0 },
      { key: 'max_reverse_speed', type: 'number', default: 0 },
      { key: 'acceleration', type: 'number', default: 0 },
      { key: 'deceleration', type: 'number', default: 0 },
      { key: 'max_yaw_rate', type: 'number', default: 0 },
      { key: 'radar_range', type: 'number', default: 0 },
      { key: 'radar_shows', type: 'boolean', default: false },
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'impulse_charge_duration', type: 'number', default: 3.0 },
      { key: 'impulse_speed_multiplier', type: 'number', default: 10.0 },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  weapons_console: {
    section: 'weapons_console',
    label: 'Weapons Console',
    fields: [
      { key: 'radar_range', type: 'number', default: 0 },
      { key: 'target_range', type: 'number', default: 0 },
      { key: 'fire_arc', type: 'number', default: 0 },
      { key: 'beam_range', type: 'number', default: 0 },
      { key: 'beam_damage_per_sec', type: 'number', default: 0 },
      { key: 'beam_duration_secs', type: 'number', default: 0 },
      { key: 'cooldown_secs', type: 'number', default: 0 },
      { key: 'beam_color', type: 'array', items: 'number', default: [] },
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  engineering_console: {
    section: 'engineering_console',
    label: 'Engineering Console',
    fields: [
      { key: 'repair_rate', type: 'number', default: 0 },
      { key: 'repair_hp_per_cycle', type: 'number', default: 0 },
      { key: 'repair_cooldown_secs', type: 'number', default: 0 },
      { key: 'cooldown_secs', type: 'number', default: 0 },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  captain_console: {
    section: 'captain_console',
    label: 'Captain Console',
    fields: [
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  power: {
    section: 'power',
    label: 'Power',
    fields: [
      { key: 'capacity', type: 'number' },
      { key: 'rates', type: 'array', items: 'number' },
      { key: 'emergency_threshold', type: 'number' },
    ],
  },

  science_console: {
    section: 'science_console',
    label: 'Science Console',
    fields: [
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'long_range_radar', type: 'object', optional: true },
      { key: 'system_map', type: 'object', optional: true },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  sensors_console: {
    section: 'sensors_console',
    label: 'Sensors Console',
    fields: [
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'long_range_radar', type: 'object', optional: true },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  shields_console: {
    section: 'shields_console',
    label: 'Shields Console',
    fields: [
      { key: 'focus_bonus_max_hp', type: 'number', default: 50 },
      { key: 'focus_bonus_regen', type: 'number', default: 5.0 },
      { key: 'focus_penalty_max_hp', type: 'number', default: 25 },
      { key: 'focus_penalty_regen', type: 'number', default: 2.5 },
      { key: 'focus_decay_rate', type: 'number', default: 10.0 },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  star: {
    section: 'star',
    label: 'Star',
    fields: [
      { key: 'name', type: 'string', default: '' },
      { key: 'radius', type: 'number' },
      { key: 'colour', type: 'array', items: 'number' },
      { key: 'position', type: 'array', items: 'number', optional: true, default: [] },
      { key: 'tags', type: 'array', items: 'string', optional: true, default: [] },
      { key: 'light_range', type: 'number', optional: true },
      { key: 'light_intensity', type: 'number', optional: true },
      { key: 'light_colour', type: 'array', items: 'number', optional: true },
    ],
  },

  planet: {
    section: 'planet',
    label: 'Planet',
    fields: [
      { key: 'name', type: 'string', default: '' },
      { key: 'radius', type: 'number' },
      { key: 'colour', type: 'array', items: 'number' },
      { key: 'position', type: 'array', items: 'number', optional: true, default: [] },
      { key: 'tags', type: 'array', items: 'string', optional: true, default: [] },
    ],
  },

  asteroid_field: {
    section: 'asteroid_field',
    label: 'Asteroid Field',
    fields: [
      { key: 'inner_radius', type: 'number' },
      { key: 'outer_radius', type: 'number' },
      { key: 'density', type: 'number' },
      { key: 'spawn_distance', type: 'number', default: 150.0 },
      { key: 'despawn_distance', type: 'number', default: 250.0 },
      { key: 'asteroid_type_paths', type: 'array', items: 'string', default: [] },
      { key: 'cosmetic_type_paths', type: 'array', items: 'string', default: [] },
      { key: 'tags', type: 'array', items: 'string', default: [] },
      { key: 'grid', type: 'object', optional: true },
    ],
  },

  shape: {
    section: 'shape',
    label: 'Region Shape',
    fields: [
      { key: 'type', type: 'string', enum: ['sphere', 'box', 'torus'] },
      // sphere
      { key: 'radius', type: 'number', optional: true },
      // box
      { key: 'half_extents', type: 'array', items: 'number', optional: true },
      { key: 'yaw', type: 'number', optional: true, default: 0 },
      // torus
      { key: 'inner_radius', type: 'number', optional: true },
      { key: 'outer_radius', type: 'number', optional: true },
    ],
  },

  effects: {
    section: 'effects',
    label: 'Region Effects',
    fields: [
      { key: 'comms_jammed', type: 'object', optional: true },
      { key: 'sensor_blind', type: 'object', optional: true },
      { key: 'blocks_impulse', type: 'object', optional: true },
    ],
  },

  station: {
    section: 'station',
    label: 'Station',
    fields: [
      { key: 'name', type: 'string' },
      { key: 'shape', type: 'string', enum: ['sphere', 'cylinder', 'torus'] },
      { key: 'radius', type: 'number' },
      { key: 'hull_integrity', type: 'number' },
    ],
  },

  behaviour: {
    section: 'behaviour',
    label: 'Behaviour',
    fields: [
      { key: 'initial_state', type: 'string' },
      { key: 'state', type: 'array', items: 'object', default: [] },
      { key: 'transition', type: 'array', items: 'object', default: [] },
    ],
  },

  stations: {
    section: 'stations',
    label: 'Stations',
    fields: [
      { key: 'min_players', type: 'number' },
      { key: 'max_players', type: 'number' },
    ],
  },
};

/**
 * Ordered list of all EntityConfig section keys (matching Rust struct fields).
 * Used by completeness tests.
 */
export const ENTITY_CONFIG_SECTIONS = [
  'tags',
  'faction',
  'hull',
  'collider',
  'appearance',
  'radar_appearance',
  'helm_console',
  'weapons_console',
  'engineering_console',
  'captain_console',
  'power',
  'science_console',
  'sensors_console',
  'shields_console',
  'star',
  'planet',
  'asteroid_field',
  'shape',
  'effects',
  'station',
  'behaviour',
  'stations',
];

/**
 * Return the schema entry for a given section key, or null if not found.
 * @param {string} section
 * @returns {object|null}
 */
export function getComponentSchema(section) {
  return COMPONENT_SCHEMA[section] ?? null;
}

/**
 * Return all sections that have a complexity_toml field.
 * @returns {string[]}
 */
export function getSectionsWithComplexityToml() {
  return Object.entries(COMPONENT_SCHEMA)
    .filter(([, schema]) => schema.fields.some((f) => f.dropdownSource === 'complexity'))
    .map(([key]) => key);
}

/**
 * Return all sections that have a faction field.
 * @returns {string[]}
 */
export function getSectionsWithFaction() {
  return Object.entries(COMPONENT_SCHEMA)
    .filter(([, schema]) => schema.fields.some((f) => f.dropdownSource === 'factions'))
    .map(([key]) => key);
}
