import { describe, it, expect } from 'vitest';
import {
  SECURITY_SYSTEM_ID,
  SECURITY_ACTIONS,
  dispatchSecurityTeamPayload,
  dispatchSecurityTeam,
  recallSecurityTeamPayload,
  recallSecurityTeam,
  securityTeamRows,
  securityActionRows,
} from '../../gui/security-dispatch.js';

const TARGET = '00000000-0000-8000-8000-000000000042';

describe('security-dispatch payloads', () => {
  it('addresses the security system id the host routes on', () => {
    expect(SECURITY_SYSTEM_ID).toBe('security');
  });

  // The four verbs issue #1346 names as the required vocabulary, spelled the way
  // `SecurityAction::as_str` spells them in src/security/teams.rs.
  it('carries the whole generic action vocabulary and nothing scenario-specific', () => {
    expect(SECURITY_ACTIONS).toEqual([
      'secure_contain',
      'assist_evacuation',
      'board',
      'place_charges',
    ]);
  });

  it('builds the DispatchSecurityTeam payload naming team, target and action', () => {
    expect(dispatchSecurityTeamPayload(1, TARGET, 'assist_evacuation')).toEqual({
      type: 'DispatchSecurityTeam',
      data: { team_idx: 1, target: TARGET, action: 'assist_evacuation' },
    });
  });

  it('builds the RecallSecurityTeam payload naming only the team', () => {
    const payload = recallSecurityTeamPayload(0);
    expect(payload).toEqual({ type: 'RecallSecurityTeam', data: { team_idx: 0 } });
    expect(Object.keys(payload.data)).toEqual(['team_idx']);
  });

  it('rejects a bad team index, an empty target and an unknown verb', () => {
    expect(() => dispatchSecurityTeamPayload(1.5, TARGET, 'board')).toThrow(TypeError);
    expect(() => dispatchSecurityTeamPayload(-1, TARGET, 'board')).toThrow(TypeError);
    expect(() => dispatchSecurityTeamPayload(0, '', 'board')).toThrow(TypeError);
    expect(() => dispatchSecurityTeamPayload(0, TARGET, 'vent_the_deck')).toThrow(TypeError);
    expect(() => recallSecurityTeamPayload(-1)).toThrow(TypeError);
  });
});

describe('security-dispatch sends through the command gateway', () => {
  it('sends a ControlSystem envelope targeting the security system', () => {
    const calls = [];
    const env = dispatchSecurityTeam(1, TARGET, 'secure_contain', (type, data) =>
      calls.push([type, data]),
    );
    expect(calls).toEqual([
      [
        'ControlSystem',
        {
          target: 'security',
          payload: {
            type: 'DispatchSecurityTeam',
            data: { team_idx: 1, target: TARGET, action: 'secure_contain' },
          },
        },
      ],
    ]);
    expect(env.type).toBe('ControlSystem');
  });

  it('sends the recall the same way', () => {
    const calls = [];
    recallSecurityTeam(0, (type, data) => calls.push([type, data]));
    expect(calls).toEqual([
      [
        'ControlSystem',
        {
          target: 'security',
          payload: { type: 'RecallSecurityTeam', data: { team_idx: 0 } },
        },
      ],
    ]);
  });
});

// The repair-team-style readout: a team list with state, assignment, progress and
// risk, and the eligible targets/actions beside it.
const BLACKBOARD = {
  range: 400,
  teams: [
    {
      state: 'working',
      target: TARGET,
      target_name: 'world.falling_skyway.entity.skyhook.name',
      action: 'assist_evacuation',
      progress: 0.4,
      risk: 0.65,
    },
    { state: 'available' },
  ],
  targets: [
    {
      uuid: TARGET,
      name: 'world.falling_skyway.entity.skyhook.name',
      separation: 180,
      in_range: true,
      actions: [
        {
          action: 'assist_evacuation',
          duration_secs: 45,
          risk: 0.65,
          priority: 'life_safety',
          warning: 'world.falling_skyway.security.warning.head_evacuation',
        },
      ],
    },
    {
      uuid: 'far-away',
      separation: 9000,
      in_range: false,
      actions: [{ action: 'secure_contain', duration_secs: 60, risk: 0.4, priority: 'threat_containment' }],
    },
  ],
};

describe('security-dispatch console projection', () => {
  it('lists every team with its state, assignment, progress and risk', () => {
    const rows = securityTeamRows(BLACKBOARD);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      teamIdx: 0,
      state: 'working',
      target: TARGET,
      targetName: 'world.falling_skyway.entity.skyhook.name',
      action: 'assist_evacuation',
      progress: 0.4,
      risk: 0.65,
      canRecall: true,
      canDispatch: false,
    });
    expect(rows[1]).toMatchObject({
      teamIdx: 1,
      state: 'available',
      target: null,
      action: null,
      canRecall: false,
      canDispatch: true,
    });
  });

  it('offers no recall for a team that is home, withdrawing or off the board', () => {
    for (const state of ['available', 'withdrawing', 'unavailable']) {
      const [row] = securityTeamRows({ teams: [{ state }] });
      expect(row.canRecall).toBe(false);
    }
  });

  it('renders an empty muster rather than throwing on a missing blackboard', () => {
    expect(securityTeamRows(null)).toEqual([]);
    expect(securityActionRows(undefined)).toEqual([]);
  });

  it('flattens the targets into (target, action) rows carrying the authored terms', () => {
    const rows = securityActionRows(BLACKBOARD);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toEqual({
      target: TARGET,
      targetName: 'world.falling_skyway.entity.skyhook.name',
      separation: 180,
      eligible: true,
      action: 'assist_evacuation',
      durationSecs: 45,
      risk: 0.65,
      priority: 'life_safety',
      warning: 'world.falling_skyway.security.warning.head_evacuation',
    });
    // A warning is a strings.csv id or nothing — never English out of the panel.
    expect(rows[1].warning).toBeNull();
  });

  // The console must not re-derive the range rule: `eligible` is the server's own
  // `in_range`, so a panel can never offer a dispatch the server would refuse for
  // a reason the panel computed differently.
  it('takes eligibility from the server rather than from the separation', () => {
    const rows = securityActionRows(BLACKBOARD);
    expect(rows.map((row) => row.eligible)).toEqual([true, false]);
    expect(securityActionRows(BLACKBOARD, true)).toHaveLength(1);

    const lying = securityActionRows({
      targets: [{ uuid: 'x', separation: 1, in_range: false, actions: [{ action: 'board' }] }],
    });
    expect(lying[0].eligible).toBe(false);
  });
});
