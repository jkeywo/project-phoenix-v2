/**
 * action-schema.js
 *
 * Pure schema data + validators for trigger action types.
 * Mirrors the TriggerAction enum from src/world/config.rs.
 *
 * Each action type has a schema entry describing its fields:
 *   { key, type, required?, enum?, default? }
 *
 * type: 'string' | 'number' | 'boolean'
 * enum: array of allowed string values (for constrained fields)
 * required: true if the field must be present (default false)
 * default: value used when field is absent and not required
 */

/** Enum values used by modifier slot fields. */
export const MODIFIER_SLOTS = [
  'MaxSpeed',
  'MaxYawRate',
  'RadarRange',
  'PhaserDamage',
  'HullDamageTaken',
  'RepairRate',
];

/** Enum values used by int-modifier slot fields. */
export const INT_MODIFIER_SLOTS = ['RepairTeams'];

/** Enum values used by flag kind fields. */
export const FLAG_KINDS = ['CommsJammed', 'SensorBlind'];

export const ACTION_SCHEMA = {
  add_objective: {
    type: 'add_objective',
    label: 'Add Objective',
    fields: [
      { key: 'id', type: 'string', required: true },
      { key: 'text', type: 'string', required: true },
      { key: 'mandatory', type: 'boolean', default: false },
    ],
  },

  complete_objective: {
    type: 'complete_objective',
    label: 'Complete Objective',
    fields: [
      { key: 'id', type: 'string', required: true },
    ],
  },

  fail_objective: {
    type: 'fail_objective',
    label: 'Fail Objective',
    fields: [
      { key: 'id', type: 'string', required: true },
    ],
  },

  set_ai_state: {
    type: 'set_ai_state',
    label: 'Set AI State',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'state', type: 'string', required: true },
      { key: 'target', type: 'string' },
    ],
  },

  apply_modifier: {
    type: 'apply_modifier',
    label: 'Apply Modifier',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'tag', type: 'string', required: true },
      { key: 'slot', type: 'string', required: true, enum: MODIFIER_SLOTS },
      { key: 'bonus', type: 'number', required: true },
    ],
  },

  remove_modifier: {
    type: 'remove_modifier',
    label: 'Remove Modifier',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'tag', type: 'string', required: true },
      { key: 'slot', type: 'string', required: true, enum: MODIFIER_SLOTS },
    ],
  },

  apply_flag: {
    type: 'apply_flag',
    label: 'Apply Flag',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'tag', type: 'string', required: true },
      { key: 'kind', type: 'string', required: true, enum: FLAG_KINDS },
    ],
  },

  remove_flag: {
    type: 'remove_flag',
    label: 'Remove Flag',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'tag', type: 'string', required: true },
      { key: 'kind', type: 'string', required: true, enum: FLAG_KINDS },
    ],
  },

  apply_int_modifier: {
    type: 'apply_int_modifier',
    label: 'Apply Int Modifier',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'tag', type: 'string', required: true },
      { key: 'slot', type: 'string', required: true, enum: INT_MODIFIER_SLOTS },
      { key: 'int_bonus', type: 'number', required: true },
    ],
  },

  remove_int_modifier: {
    type: 'remove_int_modifier',
    label: 'Remove Int Modifier',
    fields: [
      { key: 'entity', type: 'string', required: true },
      { key: 'tag', type: 'string', required: true },
      { key: 'slot', type: 'string', required: true, enum: INT_MODIFIER_SLOTS },
    ],
  },

  game_over: {
    type: 'game_over',
    label: 'Game Over',
    fields: [
      { key: 'message', type: 'string' },
    ],
  },

  load_world: {
    type: 'load_world',
    label: 'Load World',
    fields: [
      { key: 'path', type: 'string', required: true },
    ],
  },

  unload_world: {
    type: 'unload_world',
    label: 'Unload World',
    fields: [
      { key: 'path', type: 'string', required: true },
    ],
  },
};

/**
 * Validate a single action object against its schema.
 *
 * @param {object} action  The parsed action object (must have a `type` field).
 * @returns {{ valid: boolean, errors: string[] }}
 */
export function validateAction(action) {
  const errors = [];

  if (!action || typeof action !== 'object') {
    return { valid: false, errors: ['Action must be an object'] };
  }

  const schema = ACTION_SCHEMA[action.type];
  if (!schema) {
    return { valid: false, errors: [`Unknown action type: "${action.type}"`] };
  }

  for (const field of schema.fields) {
    const value = action[field.key];

    if (field.required && (value === undefined || value === null)) {
      errors.push(`"${action.type}": missing required field "${field.key}"`);
      continue;
    }

    if (!field.required && (value === undefined || value === null)) {
      continue;
    }

    if (field.enum && !field.enum.includes(value)) {
      const allowed = field.enum.join(', ');
      errors.push(`"${action.type}": "${field.key}" has invalid value "${value}"; allowed: ${allowed}`);
    }

    if (field.type === 'number' && typeof value !== 'number') {
      errors.push(`"${action.type}": "${field.key}" must be a number, got ${typeof value}`);
    }
  }

  return { valid: errors.length === 0, errors };
}

/**
 * Validate all actions in a trigger.
 *
 * @param {object[]} actions  Array of parsed action objects.
 * @returns {{ valid: boolean, errors: string[] }}
 */
export function validateTriggerActions(actions) {
  if (!Array.isArray(actions)) {
    return { valid: false, errors: ['Actions must be an array'] };
  }

  const allErrors = [];
  for (let i = 0; i < actions.length; i++) {
    const result = validateAction(actions[i]);
    for (const err of result.errors) {
      allErrors.push(`[${i}] ${err}`);
    }
  }

  return { valid: allErrors.length === 0, errors: allErrors };
}
