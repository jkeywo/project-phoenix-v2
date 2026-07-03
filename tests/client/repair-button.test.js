import { describe, it, expect } from 'vitest';
import {
  isTeamBusy,
  allTeamsBusy,
  anyTeamActive,
  repairButtonPress,
  refreshRepairButton,
} from '../../gui/repair-button.js';

// TeamSlot wire shape: 'Idle' string, or { Travelling|Repairing|Returning: {...} }.
const IDLE = 'Idle';
const TRAVEL = { Travelling: { system_id: 'helm', display_name: 'Helm', elapsed: 1.2 } };
const REPAIR = { Repairing: { system_id: 'power', display_name: 'Power' } };
const RETURN = { Returning: { remaining: 2.0 } };

// ── busy / active predicates ─────────────────────────────────────────────────

describe('isTeamBusy', () => {
  it('is false for the Idle string', () => {
    expect(isTeamBusy(IDLE)).toBe(false);
  });
  it('is true for any tagged variant', () => {
    expect(isTeamBusy(TRAVEL)).toBe(true);
    expect(isTeamBusy(REPAIR)).toBe(true);
    expect(isTeamBusy(RETURN)).toBe(true);
  });
});

describe('allTeamsBusy (press guard)', () => {
  it('is false when at least one team is idle', () => {
    expect(allTeamsBusy([IDLE, TRAVEL])).toBe(false);
    expect(allTeamsBusy([TRAVEL, IDLE, REPAIR])).toBe(false);
  });
  it('is true when every team is busy', () => {
    expect(allTeamsBusy([TRAVEL, REPAIR, RETURN])).toBe(true);
  });
  it('is false for an empty team list (never block an empty fleet)', () => {
    expect(allTeamsBusy([])).toBe(false);
    expect(allTeamsBusy(undefined)).toBe(false);
  });
});

describe('anyTeamActive', () => {
  it('is false when all idle', () => {
    expect(anyTeamActive([IDLE, IDLE])).toBe(false);
  });
  it('is true when any team is busy', () => {
    expect(anyTeamActive([IDLE, RETURN])).toBe(true);
  });
});

// ── repairButtonPress (message shape + all-busy guard) ───────────────────────

describe('repairButtonPress', () => {
  it('returns the default ControlSystem/DispatchRepairTeam message when a team is free', () => {
    expect(repairButtonPress([IDLE, TRAVEL])).toEqual({
      type: 'ControlSystem',
      data: {
        target: 'repair',
        payload: {
          type: 'DispatchRepairTeam',
          data: { team_idx: 0, target: { type: 'Station', data: 'helm' } },
        },
      },
    });
  });

  it('targets team 0 → helm station in the default press', () => {
    const msg = repairButtonPress([IDLE]);
    expect(msg.type).toBe('ControlSystem');
    expect(msg.data.target).toBe('repair');
    expect(msg.data.payload.type).toBe('DispatchRepairTeam');
    expect(msg.data.payload.data.team_idx).toBe(0);
    expect(msg.data.payload.data.target).toEqual({ type: 'Station', data: 'helm' });
  });

  it('suppresses (returns null) when all teams are busy', () => {
    expect(repairButtonPress([TRAVEL, REPAIR, RETURN])).toBeNull();
  });

  it('dispatches for an empty fleet (guard only fires when all present-teams busy)', () => {
    expect(repairButtonPress([])).not.toBeNull();
  });
});

// ── refreshRepairButton (label / colour / disabled) ──────────────────────────

describe('refreshRepairButton', () => {
  it('shows REPAIR (ready) when no team is active', () => {
    const s = refreshRepairButton([IDLE, IDLE]);
    expect(s.label).toBe('REPAIR');
    expect(s.background).toBe('rgb(33, 69, 33)');
    expect(s.color).toBe('rgb(128, 255, 128)');
    expect(s.disabled).toBe(false);
  });

  it('shows TEAMS DISPATCHED (active) when any team is busy', () => {
    const s = refreshRepairButton([IDLE, TRAVEL]);
    expect(s.label).toBe('TEAMS DISPATCHED');
    expect(s.background).toBe('rgb(13, 77, 13)');
    expect(s.color).toBe('rgb(128, 255, 128)');
    // One team still idle -> a dispatch is possible -> not disabled.
    expect(s.disabled).toBe(false);
  });

  it('disables the button only when every team is busy', () => {
    const s = refreshRepairButton([TRAVEL, REPAIR, RETURN]);
    expect(s.label).toBe('TEAMS DISPATCHED');
    expect(s.disabled).toBe(true);
  });
});
