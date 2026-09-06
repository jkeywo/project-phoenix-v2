import { describe, it, expect } from 'vitest';
import {
  REPAIR_SYSTEM_ID,
  repairTargetFor,
  dispatchRepairTeamPayload,
  dispatchRepairTeam,
  recallRepairTeamPayload,
  recallRepairTeam,
  setRepairTargetPriorityPayload,
  setRepairTargetPriority,
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

  // Issue #1015: the damaged-systems taps carry a system id and NO ordinal —
  // the host ranks the team's remaining work, because #737 hides most of the
  // candidates from this console.
  it('builds the SetRepairTargetPriority payload with no ordinal at all', () => {
    const payload = setRepairTargetPriorityPayload('helm-engine-port');
    expect(payload).toEqual({
      type: 'SetRepairTargetPriority',
      data: { system_id: 'helm-engine-port' },
    });
    expect(Object.keys(payload.data)).toEqual(['system_id']);
  });

  // Issue #1385: the internal RECALL names its team, where the fieldless
  // external recall beside it names nothing — a ship sends one team abroad at a
  // time, but every internal team can be out on a different job at once.
  it('builds the RecallRepairTeam payload naming only the team', () => {
    const payload = recallRepairTeamPayload(2);
    expect(payload).toEqual({ type: 'RecallRepairTeam', data: { team_idx: 2 } });
    expect(Object.keys(payload.data)).toEqual(['team_idx']);
  });

  it('rejects a team index that is not a slot number', () => {
    expect(() => recallRepairTeamPayload(1.5)).toThrow(TypeError);
    expect(() => recallRepairTeamPayload(-1)).toThrow(TypeError);
    expect(() => recallRepairTeamPayload(256)).toThrow(TypeError);
    expect(() => recallRepairTeamPayload(undefined)).toThrow(TypeError);
  });

  it('rejects an empty system id', () => {
    expect(() => setRepairTargetPriorityPayload('')).toThrow(TypeError);
    expect(() => setRepairTargetPriorityPayload(undefined)).toThrow(TypeError);
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

  it('sends SetRepairTargetPriority naming a system and nothing else', () => {
    const calls = [];
    setRepairTargetPriority('hull-plating', (type, data) => calls.push([type, data]));
    expect(calls).toEqual([[
      'ControlSystem',
      {
        target: 'repair',
        payload: { type: 'SetRepairTargetPriority', data: { system_id: 'hull-plating' } },
      },
    ]]);
  });

  it('sends RecallRepairTeam through the same gateway and repair owner', () => {
    const calls = [];
    const env = recallRepairTeam(1, (type, data) => calls.push([type, data]));
    expect(calls).toEqual([[
      'ControlSystem',
      {
        target: 'repair',
        payload: { type: 'RecallRepairTeam', data: { team_idx: 1 } },
      },
    ]]);
    expect(env.type).toBe('ControlSystem');
  });

  it('addresses the exact authored Repair owner a console names', () => {
    const calls = [];
    recallRepairTeam(0, (type, data) => calls.push([type, data]), 'damage_control');
    expect(calls[0][1].target).toBe('damage_control');
  });

  it('is the path the recall_repair_team console action takes', () => {
    const calls = [];
    ACTION_MAP.recall_repair_team(
      { action: 'recall_repair_team', team_idx: 3 },
      (type, data) => calls.push([type, data]),
    );
    expect(calls).toEqual([[
      'ControlSystem',
      {
        target: 'repair',
        payload: { type: 'RecallRepairTeam', data: { team_idx: 3 } },
      },
    ]]);
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
