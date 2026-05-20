import { describe, it, expect } from 'vitest';
import * as pickers from '../trigger-pickers.js';

describe('getEntityNameOptions', () => {
  it('returns all named entities from layers', () => {
    const layers = [
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [
            { name: 'raider_alpha' },
            { name: 'station_beta' },
          ],
        },
      },
    ];
    const result = pickers.getEntityNameOptions(layers);
    expect(result).toEqual([
      { value: 'raider_alpha', label: 'raider_alpha', layerPath: 'worlds/default.toml' },
      { value: 'station_beta', label: 'station_beta', layerPath: 'worlds/default.toml' },
    ]);
  });

  it('disambiguates with layer suffix when names collide across layers', () => {
    const layers = [
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [{ name: 'raider_alpha' }],
        },
      },
      {
        path: 'worlds/patrol.toml',
        worldState: {
          entity: [{ name: 'raider_alpha' }],
        },
      },
    ];
    const result = pickers.getEntityNameOptions(layers);
    expect(result).toHaveLength(2);
    for (const opt of result) {
      expect(opt.value).toBe('raider_alpha');
      expect(opt.label).toBe(`raider_alpha (${opt.layerPath})`);
    }
  });

  it('returns empty for no layers', () => {
    expect(pickers.getEntityNameOptions([])).toEqual([]);
  });
});

describe('getObjectiveIdOptions', () => {
  it('extracts IDs from add_objective actions', () => {
    const worldState = {
      trigger: [
        {
          condition: 'on_destroyed',
          entity: 'raider_alpha',
          action: [
            { type: 'add_objective', id: 'obj-raider-destroyed' },
            { type: 'add_objective', id: 'obj-collect-resources' },
          ],
        },
      ],
    };
    const result = pickers.getObjectiveIdOptions(worldState);
    expect(result).toEqual([
      { value: 'obj-raider-destroyed', label: 'obj-raider-destroyed' },
      { value: 'obj-collect-resources', label: 'obj-collect-resources' },
    ]);
  });

  it('returns empty when no add_objective actions', () => {
    const worldState = {
      trigger: [
        {
          condition: 'on_destroyed',
          entity: 'raider_alpha',
          action: [{ type: 'game_over', message: 'Defeat' }],
        },
      ],
    };
    expect(pickers.getObjectiveIdOptions(worldState)).toEqual([]);
  });
});

describe('getAiStateOptions', () => {
  it('returns states from target entity behaviour', () => {
    const worldState = {
      entity: [
        {
          name: 'raider_alpha',
          behaviour: {
            initial_state: 'patrol',
            state: [
              { name: 'idle', kind: 'idle', parameters: {} },
              { name: 'patrol', kind: 'patrol', parameters: { anchor: 'alpha' } },
              { name: 'attack', kind: 'attack', parameters: { target_entity: '', range: 500 } },
            ],
          },
        },
      ],
    };
    const result = pickers.getAiStateOptions(worldState, 'raider_alpha');
    expect(result).toEqual([
      { value: 'idle', label: 'idle' },
      { value: 'patrol', label: 'patrol' },
      { value: 'attack', label: 'attack' },
    ]);
  });

  it('returns empty when entity has no behaviour', () => {
    const worldState = {
      entity: [{ name: 'raider_alpha' }],
    };
    expect(pickers.getAiStateOptions(worldState, 'raider_alpha')).toEqual([]);
  });

  it('returns empty when entity not found', () => {
    const worldState = {
      entity: [{ name: 'station_beta' }],
    };
    expect(pickers.getAiStateOptions(worldState, 'nonexistent')).toEqual([]);
  });
});

describe('getModifierSlotOptions', () => {
  it('returns the correct slots', () => {
    const result = pickers.getModifierSlotOptions();
    expect(result).toEqual([
      { value: 'MaxSpeed', label: 'MaxSpeed' },
      { value: 'MaxYawRate', label: 'MaxYawRate' },
      { value: 'RadarRange', label: 'RadarRange' },
      { value: 'PhaserDamage', label: 'PhaserDamage' },
      { value: 'HullDamageTaken', label: 'HullDamageTaken' },
      { value: 'RepairRate', label: 'RepairRate' },
    ]);
  });
});

describe('getFlagKindOptions', () => {
  it('returns the correct kinds', () => {
    const result = pickers.getFlagKindOptions();
    expect(result).toEqual([
      { value: 'CommsJammed', label: 'CommsJammed' },
      { value: 'SensorBlind', label: 'SensorBlind' },
    ]);
  });
});
