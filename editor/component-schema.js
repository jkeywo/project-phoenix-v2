/**
 * component-schema.js
 *
 * Pure schema data for every TOML section in EntityConfig (src/entities/config.rs).
 * Each entry describes the fields of a top-level TOML section so the editor can
 * render structured component cards.
 *
 * Field descriptor shape:
 *   { key, type, default?, optional?, items?, dropdownSource?, subfields?, entryFields? }
 *
 *   type: 'string' | 'number' | 'boolean' | 'array' | 'object' | 'subobject' | 'array-of-tables'
 *   items: type of array elements (for type:'array')
 *   dropdownSource: 'factions' | 'complexity' (drives dropdown resolution)
 *   optional: true if the field may be omitted from TOML
 *   subfields: field descriptors for type:'subobject' inline editing
 *   entryFields: field descriptors for type:'array-of-tables' per-entry editing
 *   entryDefaults: default object for new entries (for type:'array-of-tables')
 */

/** Sub-field set reused for every RadarConfig sub-object. */
const RADAR_SUBFIELDS = [
  { key: 'range', type: 'number' },
  { key: 'shows', type: 'array', items: 'string' },
  { key: 'selects', type: 'array', items: 'string', optional: true },
];

/** Per-entry fields for [[weapons_console.phaser_banks]]. */
const PHASER_BANK_ENTRY_FIELDS = [
  { key: 'id', type: 'string' },
  { key: 'facing_deg', type: 'number' },
  { key: 'fire_arc_deg', type: 'number' },
  { key: 'auto_arc_deg', type: 'number' },
  { key: 'beam_range', type: 'number', default: 0 },
  { key: 'beam_damage_per_sec', type: 'number', default: 0 },
  { key: 'beam_duration_secs', type: 'number', default: 0 },
  { key: 'cooldown_secs', type: 'number', default: 0 },
  { key: 'beam_color', type: 'array', items: 'number', default: [] },
  { key: 'shield_pierce', type: 'number', default: 0 },
  { key: 'marker', type: 'string', optional: true },
];

/** Per-entry fields for [[torpedoes.tubes]]. */
const TORPEDO_TUBE_ENTRY_FIELDS = [
  { key: 'id', type: 'string' },
  { key: 'facing_deg', type: 'number' },
  { key: 'fire_arc_deg', type: 'number' },
  { key: 'load_time', type: 'number', optional: true },
  { key: 'marker', type: 'string', optional: true },
];

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

  name: {
    section: 'name',
    label: 'Name',
    fields: [
      { key: 'name', type: 'string', default: '' },
    ],
  },

  hull: {
    section: 'hull',
    label: 'Hull',
    fields: [
      { key: 'hull_integrity', type: 'number', default: 0, optional: true },
      { key: 'system_hull', type: 'array', items: 'object', optional: true, default: [] },
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
      { key: 'colour', type: 'array', items: 'number', optional: true },
      { key: 'size', type: 'number', optional: true },
      { key: 'icon', type: 'string', optional: true },
      { key: 'region_colour', type: 'array', items: 'number', optional: true },
    ],
  },

  mesh: {
    section: 'mesh',
    label: 'Mesh',
    fields: [
      { key: 'model', type: 'string', optional: true },
      { key: 'variant', type: 'string', optional: true },
      { key: 'shape', type: 'string', enum: ['sphere', 'cuboid', 'torus'] },
      { key: 'colour', type: 'array', items: 'number' },
      { key: 'radius', type: 'number', optional: true, default: 0 },
      { key: 'size', type: 'array', items: 'number', optional: true },
      { key: 'minor_radius', type: 'number', optional: true, default: 0 },
      { key: 'emissive', type: 'number', optional: true },
      { key: 'scale', type: 'number', optional: true },
      { key: 'rotation', type: 'array', items: 'number', optional: true },
    ],
  },

  // Array-of-tables: maps to TOML `[[light]]` blocks (Vec<LightConfig>).
  light: {
    section: 'light',
    label: 'Lights',
    arrayOfTables: true,
    fields: [
      { key: 'kind', type: 'string', enum: ['point', 'directional'] },
      { key: 'colour', type: 'array', items: 'number' },
      { key: 'intensity', type: 'number' },
      { key: 'range', type: 'number', optional: true },
      { key: 'face_player', type: 'boolean', optional: true },
    ],
    entryFields: [
      { key: 'kind', type: 'string', enum: ['point', 'directional'] },
      { key: 'colour', type: 'array', items: 'number' },
      { key: 'intensity', type: 'number' },
      { key: 'range', type: 'number', optional: true },
      { key: 'face_player', type: 'boolean', optional: true },
    ],
    entryDefaults: {
      kind: 'point',
      colour: [1.0, 1.0, 1.0],
      intensity: 1000.0,
    },
  },

  star: {
    section: 'star',
    label: 'Star',
    fields: [
      { key: 'radius', type: 'number', optional: true },
      { key: 'longitude_segments', type: 'number', optional: true },
      { key: 'latitude_segments', type: 'number', optional: true },
      { key: 'surface_colour', type: 'array', items: 'number', optional: true },
      { key: 'hot_colour', type: 'array', items: 'number', optional: true },
      { key: 'cell_colour', type: 'array', items: 'number', optional: true },
      { key: 'halo_colour', type: 'array', items: 'number', optional: true },
      { key: 'halo_radius_multiplier', type: 'number', optional: true },
      { key: 'animation_speed', type: 'number', optional: true },
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
      { key: 'low_speed_turn_boost', type: 'number', default: 0, optional: true },
      { key: 'max_bank_deg', type: 'number', default: 0, optional: true },
      { key: 'bank_lerp_rate', type: 'number', optional: true },
      { key: 'impulse_charge_duration', type: 'number', default: 3.0 },
      { key: 'impulse_speed_multiplier', type: 'number', default: 10.0 },
      { key: 'impulse_acceleration_multiplier', type: 'number', default: 5.0, optional: true },
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
      {
        key: 'radar',
        type: 'subobject',
        optional: true,
        subfields: RADAR_SUBFIELDS,
      },
    ],
  },

  weapons_console: {
    section: 'weapons_console',
    label: 'Weapons Console',
    fields: [
      { key: 'torpedo_arc_color', type: 'array', items: 'number', optional: true },
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
      {
        key: 'radar',
        type: 'subobject',
        optional: true,
        subfields: RADAR_SUBFIELDS,
      },
      {
        key: 'phaser_banks',
        type: 'array-of-tables',
        optional: true,
        entryFields: PHASER_BANK_ENTRY_FIELDS,
        entryDefaults: { id: '', facing_deg: 0.0, fire_arc_deg: 270.0, auto_arc_deg: 180.0 },
      },
    ],
  },

  engineering_console: {
    section: 'engineering_console',
    label: 'Engineering Console',
    fields: [
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
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
    ],
  },

  sensors_console: {
    section: 'sensors_console',
    label: 'Sensors Console',
    fields: [
      { key: 'power_multipliers', type: 'array', items: 'number', optional: true },
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
      {
        key: 'long_range_radar',
        type: 'subobject',
        optional: true,
        subfields: RADAR_SUBFIELDS,
      },
    ],
  },

  navigation_console: {
    section: 'navigation_console',
    label: 'Navigation Console',
    fields: [
      { key: 'complexity_toml', type: 'path-complexity', optional: true, dropdownSource: 'complexity' },
      {
        key: 'system_chart',
        type: 'subobject',
        optional: true,
        subfields: RADAR_SUBFIELDS,
      },
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
      {
        key: 'base',
        type: 'subobject',
        optional: true,
        subfields: [
          { key: 'num_facings', type: 'number', optional: true },
          { key: 'max_hp', type: 'number', optional: true },
          { key: 'regen_per_sec', type: 'number', optional: true },
          { key: 'offline_duration', type: 'number', optional: true },
        ],
      },
    ],
  },

  torpedoes: {
    section: 'torpedoes',
    label: 'Torpedoes',
    fields: [
      { key: 'count', type: 'number', default: 10 },
      { key: 'damage_hull', type: 'number', default: 50 },
      { key: 'damage_shields', type: 'number', default: 5 },
      { key: 'speed', type: 'number', default: 30.0 },
      { key: 'turn_rate_deg_per_sec', type: 'number', default: 45.0 },
      { key: 'lifespan', type: 'number', default: 20.0 },
      { key: 'load_time', type: 'number', default: 10.0 },
      { key: 'detonation_radius', type: 'number', default: 5.0 },
      { key: 'shield_pierce', type: 'number', optional: true },
      {
        key: 'tubes',
        type: 'array-of-tables',
        optional: true,
        entryFields: TORPEDO_TUBE_ENTRY_FIELDS,
        entryDefaults: { id: '', facing_deg: 0.0, fire_arc_deg: 90.0 },
      },
    ],
  },

  repair: {
    section: 'repair',
    label: 'Repair',
    fields: [
      { key: 'repair_team_count', type: 'number', default: 0, optional: true },
      { key: 'travel_duration_secs', type: 'number', default: 5.0 },
      { key: 'repair_rate_hp_per_sec', type: 'number', default: 0.5 },
    ],
  },

  comms: {
    section: 'comms',
    label: 'Comms',
    fields: [
      { key: 'range', type: 'number' },
    ],
  },

  target: {
    section: 'target',
    label: 'Target',
    fields: [
      { key: 'tags', type: 'array', items: 'string', default: [] },
      { key: 'threat_level', type: 'string', enum: ['none', 'low', 'medium', 'high'], optional: true },
      { key: 'description', type: 'string', optional: true },
    ],
  },

  asteroid_field: {
    section: 'asteroid_field',
    label: 'Asteroid Field',
    fields: [
      { key: 'inner_radius', type: 'number' },
      { key: 'outer_radius', type: 'number' },
      { key: 'density', type: 'number' },
      { key: 'weight', type: 'number', default: 1.0, optional: true },
      { key: 'shape', type: 'string', enum: ['torus'], optional: true },
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

  behaviour: {
    section: 'behaviour',
    label: 'Behaviour',
    fields: [
      { key: 'initial_state', type: 'string' },
      { key: 'state', type: 'array', items: 'object', default: [] },
      { key: 'transition', type: 'array', items: 'object', default: [] },
      { key: 'waypoint_arrival_radius', type: 'number', optional: true },
      { key: 'avoidance_buffer', type: 'number', optional: true },
      { key: 'avoidance_look_ahead_secs', type: 'number', optional: true },
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
  'name',
  'tags',
  'faction',
  'hull',
  'collider',
  'appearance',
  'mesh',
  'light',
  'star',
  'radar_appearance',
  'helm_console',
  'weapons_console',
  'engineering_console',
  'captain_console',
  'power',
  'sensors_console',
  'navigation_console',
  'shields_console',
  'torpedoes',
  'repair',
  'comms',
  'target',
  'asteroid_field',
  'shape',
  'effects',
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
