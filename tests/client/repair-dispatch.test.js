import { describe, it, expect } from 'vitest';
import {
  REPAIR_SYSTEM_ID,
  repairTargetFor,
  dispatchRepairTeamPayload,
  dispatchRepairTeam,
} from '../../gui/repair-dispatch.js';
import { ACTION_MAP } from '../../gui/action-map.js';

describe('repair-dispatch target mapping', () => {
  it('maps the core bucket to RepairTarget::Core', () => {
    expect(repairTargetFor('core')).toEqual({ type: 'Core' });
  });

  it('maps a station id to RepairTarget::Station', () => {
    expect(repairTargetFor('helm')).toEqual({ type: 'Station', data: 'helm' });
  });

  it('rejects an empty target', () => {
    expect(() => repairTargetFor('')).toThrow(TypeError);
  });

  it('rejects a non-integer team index', () => {
    expect(() => dispatchRepairTeamPayload(1.5, 'helm')).toThrow(TypeError);
    expect(() => dispatchRepairTeamPayload(-1, 'helm')).toThrow(TypeError);
  });

  it('builds the DispatchRepairTeam payload', () => {
    expect(dispatchRepairTeamPayload(2, 'power')).toEqual({
      type: 'DispatchRepairTeam',
      data: { team_idx: 2, target: { type: 'Station', data: 'power' } },
    });
  });
});

describe('repair-dispatch sends through the command gateway', () => {
  it('sends a ControlSystem envelope targeting the repair system', () => {
    const calls = [];
    const env = dispatchRepairTeam(0, 'core', (type, data) => calls.push([type, data]));
    expect(REPAIR_SYSTEM_ID).toBe('repair');
    expect(calls).toEqual([[
      'ControlSystem',
      {
        target: 'repair',
        payload: { type: 'DispatchRepairTeam', data: { team_idx: 0, target: { type: 'Core' } } },
      },
    ]]);
    expect(env.type).toBe('ControlSystem');
  });

  it('is the path the dispatch_repair_team console action takes', () => {
    const calls = [];
    ACTION_MAP.dispatch_repair_team(
      { action: 'dispatch_repair_team', team_idx: 3, target: 'shields' },
      (type, data) => calls.push([type, data]),
    );
    expect(calls).toEqual([[
      'ControlSystem',
      {
        target: 'repair',
        payload: {
          type: 'DispatchRepairTeam',
          data: { team_idx: 3, target: { type: 'Station', data: 'shields' } },
        },
      },
    ]]);
  });
});
