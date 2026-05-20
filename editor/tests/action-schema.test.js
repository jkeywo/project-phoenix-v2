import { describe, it, expect } from 'vitest';
import {
  ACTION_SCHEMA,
  validateAction,
  validateTriggerActions,
  MODIFIER_SLOTS,
  INT_MODIFIER_SLOTS,
  FLAG_KINDS,
} from '../action-schema.js';

describe('ACTION_SCHEMA', () => {
  it('covers every action type', () => {
    const expectedTypes = [
      'add_objective',
      'complete_objective',
      'fail_objective',
      'set_ai_state',
      'apply_modifier',
      'remove_modifier',
      'apply_flag',
      'remove_flag',
      'apply_int_modifier',
      'remove_int_modifier',
      'game_over',
      'load_scenario',
      'load_world',
      'unload_world',
    ];
    const schemaTypes = Object.keys(ACTION_SCHEMA).sort();
    expect(schemaTypes).toEqual([...expectedTypes].sort());
  });
});

describe('validateAction', () => {
  it('valid add_objective returns valid', () => {
    const action = { type: 'add_objective', id: 'obj1', text: 'Do the thing', mandatory: true };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it('add_objective missing id returns error', () => {
    const action = { type: 'add_objective', text: 'Do the thing' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors).toContain('"add_objective": missing required field "id"');
  });

  it('add_objective missing text returns error', () => {
    const action = { type: 'add_objective', id: 'obj1' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors).toContain('"add_objective": missing required field "text"');
  });

  it('complete_objective missing id returns error', () => {
    const action = { type: 'complete_objective' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors).toContain('"complete_objective": missing required field "id"');
  });

  it('apply_modifier with valid slot returns valid', () => {
    for (const slot of MODIFIER_SLOTS) {
      const action = { type: 'apply_modifier', entity: 'ship', tag: 'boost', slot, bonus: 1.5 };
      const result = validateAction(action);
      expect(result.valid).toBe(true);
    }
  });

  it('apply_modifier with invalid slot returns error', () => {
    const action = { type: 'apply_modifier', entity: 'ship', tag: 'boost', slot: 'InvalidSlot', bonus: 1.5 };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.errors[0]).toContain('InvalidSlot');
  });

  it('apply_flag with invalid kind returns error', () => {
    const action = { type: 'apply_flag', entity: 'ship', tag: 'comm', kind: 'InvalidKind' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.errors[0]).toContain('InvalidKind');
  });

  it('game_over with no message returns valid (optional)', () => {
    const action = { type: 'game_over' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it('load_world with path returns valid', () => {
    const action = { type: 'load_world', path: 'assets/worlds/sub.toml' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it('load_world missing path returns error', () => {
    const action = { type: 'load_world' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors).toContain('"load_world": missing required field "path"');
  });

  it('multiple errors in one pass', () => {
    const action = { type: 'apply_modifier', slot: 'BadSlot' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors.filter((e) => e.includes('missing required'))).toHaveLength(3);
    expect(result.errors.filter((e) => e.includes('invalid value'))).toHaveLength(1);
  });

  it('set_ai_state with no target returns valid (optional)', () => {
    const action = { type: 'set_ai_state', entity: 'raider', state: 'patrol' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('unknown action type returns error', () => {
    const action = { type: 'nonexistent', foo: 'bar' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors[0]).toContain('Unknown action type');
  });

  it('null/undefined action returns error', () => {
    expect(validateAction(null).valid).toBe(false);
    expect(validateAction(undefined).valid).toBe(false);
  });

  it('apply_flag with valid kind returns valid', () => {
    for (const kind of FLAG_KINDS) {
      const action = { type: 'apply_flag', entity: 'ship', tag: 'jam', kind };
      expect(validateAction(action).valid).toBe(true);
    }
  });

  it('remove_flag with valid kind returns valid', () => {
    for (const kind of FLAG_KINDS) {
      const action = { type: 'remove_flag', entity: 'ship', tag: 'jam', kind };
      expect(validateAction(action).valid).toBe(true);
    }
  });

  it('apply_int_modifier with valid slot and int_bonus returns valid', () => {
    const action = { type: 'apply_int_modifier', entity: 'ship', tag: 'teams', slot: 'RepairTeams', int_bonus: 2 };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('apply_int_modifier missing int_bonus returns error', () => {
    const action = { type: 'apply_int_modifier', entity: 'ship', tag: 'teams', slot: 'RepairTeams' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors[0]).toContain('int_bonus');
  });

  it('remove_int_modifier with valid slot returns valid', () => {
    const action = { type: 'remove_int_modifier', entity: 'ship', tag: 'teams', slot: 'RepairTeams' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('apply_int_modifier with invalid slot returns error', () => {
    const action = { type: 'apply_int_modifier', entity: 'ship', tag: 'teams', slot: 'BadSlot', int_bonus: 1 };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors[0]).toContain('BadSlot');
  });

  it('load_scenario with load_scenario field returns valid', () => {
    const action = { type: 'load_scenario', load_scenario: 'assets/worlds/story.toml' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('load_scenario missing load_scenario field returns error', () => {
    const action = { type: 'load_scenario' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors[0]).toContain('load_scenario');
  });

  it('unload_world with path returns valid', () => {
    const action = { type: 'unload_world', path: 'assets/worlds/sub.toml' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('unload_world missing path returns error', () => {
    const action = { type: 'unload_world' };
    const result = validateAction(action);
    expect(result.valid).toBe(false);
    expect(result.errors[0]).toContain('path');
  });

  it('fail_objective with id returns valid', () => {
    const action = { type: 'fail_objective', id: 'obj1' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('remove_modifier with valid slot returns valid', () => {
    for (const slot of MODIFIER_SLOTS) {
      const action = { type: 'remove_modifier', entity: 'ship', tag: 'boost', slot };
      expect(validateAction(action).valid).toBe(true);
    }
  });

  it('game_over with message returns valid', () => {
    const action = { type: 'game_over', message: 'You lose!' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });

  it('set_ai_state with target returns valid', () => {
    const action = { type: 'set_ai_state', entity: 'raider', state: 'attack', target: 'player_ship' };
    const result = validateAction(action);
    expect(result.valid).toBe(true);
  });
});

describe('validateTriggerActions', () => {
  it('validates all actions and collects all errors', () => {
    const actions = [
      { type: 'add_objective', id: 'obj1', text: 'Do it' },
      { type: 'complete_objective' },
      { type: 'load_world' },
      { type: 'apply_modifier', slot: 'Bad' },
    ];
    const result = validateTriggerActions(actions);
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThanOrEqual(3);
    expect(result.errors[0]).toContain('[1]');
    expect(result.errors[0]).toContain('missing required field "id"');
    expect(result.errors.some((e) => e.startsWith('[2]'))).toBe(true);
    expect(result.errors.some((e) => e.startsWith('[3]'))).toBe(true);
  });

  it('returns valid for empty array', () => {
    const result = validateTriggerActions([]);
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it('returns error for non-array input', () => {
    const result = validateTriggerActions(null);
    expect(result.valid).toBe(false);
    expect(result.errors[0]).toContain('array');
  });
});
