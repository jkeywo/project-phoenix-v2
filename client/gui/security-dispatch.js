/**
 * gui/security-dispatch.js — the Security console's outbound command surface
 * (issue #1346, PRD #1337).
 *
 * The client-side dispatch seam for the Security System: it owns the translation
 * from what the player at the owning station did ("send team 1 up to the tether
 * head to help with the evacuation", "bring team 0 home") into the wire payloads
 * `SystemControlPayload::DispatchSecurityTeam { team_idx, target, action }` and
 * `RecallSecurityTeam { team_idx }`, and hands them to the explicit client
 * command gateway.
 *
 * The `import` on the next line is the whole point of the file, exactly as it is
 * in `repair-dispatch.js`: the console reaches the network *through*
 * `client-network-interface`, and that is an observable code edge rather than a
 * prose claim.
 *
 * # Why all three parts are named
 *
 * External repair dispatch (#1161) sends a fieldless command and lets the server
 * resolve the target off the ship's one Tactical lock. Security cannot: a hull
 * musters several teams working several places at once, so which team, which
 * target and which of that target's authored actions are three separate
 * questions a single lock cannot answer. The server still owns every RULE — the
 * reach, the team's availability, whether the target offers the action — and
 * refuses with a `strings.csv` id the console shows. Nothing here decides
 * eligibility; the blackboard's `in_range` flag is read, never re-derived.
 */
import { sendControlSystem } from './command-gateway.js';

/**
 * SystemId the host routes Security commands to. Structural addressing, not a
 * tunable gameplay value — it mirrors `SECURITY_SYSTEM_ID` in
 * `src/ship/system_registry.rs`.
 */
export const SECURITY_SYSTEM_ID = 'security';

/**
 * The generic action vocabulary, mirroring `SecurityAction` in
 * `src/security/teams.rs`. The engine knows these four and nothing about any
 * scenario; WHICH of them a given target offers, and what each costs and leaves
 * behind there, is authored on that target and arrives on the blackboard.
 */
export const SECURITY_ACTIONS = Object.freeze([
  'secure_contain',
  'assist_evacuation',
  'board',
  'place_charges',
]);

function assertTeamIdx(teamIdx) {
  if (!Number.isInteger(teamIdx) || teamIdx < 0 || teamIdx > 255) {
    throw new TypeError('security-dispatch: team_idx must be an integer 0-255');
  }
}

/**
 * Build the `DispatchSecurityTeam` payload without sending it. Exposed so tests
 * (and any console control) can assert on the exact wire shape.
 *
 * @param {number} teamIdx 0-based index into the hull's authored teams
 * @param {string} target the target entity's uuid, as the blackboard carries it
 * @param {string} action one of {@link SECURITY_ACTIONS}
 * @returns {{type: string, data: object}}
 */
export function dispatchSecurityTeamPayload(teamIdx, target, action) {
  assertTeamIdx(teamIdx);
  if (typeof target !== 'string' || target.length === 0) {
    throw new TypeError('security-dispatch: target must be a non-empty entity uuid');
  }
  if (!SECURITY_ACTIONS.includes(action)) {
    throw new TypeError(`security-dispatch: unknown action '${action}'`);
  }
  return {
    type: 'DispatchSecurityTeam',
    data: { team_idx: teamIdx, target, action },
  };
}

/**
 * Send one Security team through the explicit command gateway.
 *
 * @param {number} teamIdx
 * @param {string} target target entity uuid
 * @param {string} action one of {@link SECURITY_ACTIONS}
 * @param {((type: string, data?: object) => void)} [send] explicit transport;
 *   omitted when called from a context that has the page's live link.
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function dispatchSecurityTeam(teamIdx, target, action, send) {
  return sendControlSystem(
    SECURITY_SYSTEM_ID,
    dispatchSecurityTeamPayload(teamIdx, target, action),
    send,
  );
}

/**
 * Build the `RecallSecurityTeam` payload without sending it.
 *
 * @param {number} teamIdx 0-based index into the hull's authored teams
 * @returns {{type: string, data: object}}
 */
export function recallSecurityTeamPayload(teamIdx) {
  assertTeamIdx(teamIdx);
  return { type: 'RecallSecurityTeam', data: { team_idx: teamIdx } };
}

/**
 * Bring one Security team home through the explicit command gateway.
 *
 * @param {number} teamIdx
 * @param {((type: string, data?: object) => void)} [send]
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function recallSecurityTeam(teamIdx, send) {
  return sendControlSystem(SECURITY_SYSTEM_ID, recallSecurityTeamPayload(teamIdx), send);
}

/**
 * The rows a Security panel renders: every team the hull musters, paired with the
 * assignment it is on. A pure projection of the blackboard — it re-derives
 * nothing, because everything it needs (the state, the progress, the risk, the
 * resolved target name) is already server-decided.
 *
 * @param {object|null|undefined} blackboard the `SecurityBlackboard` payload
 * @returns {Array<object>} one row per team, in authored index order
 */
export function securityTeamRows(blackboard) {
  const teams = (blackboard && blackboard.teams) || [];
  return teams.map((team, index) => ({
    teamIdx: index,
    state: team.state || 'available',
    target: team.target || null,
    targetName: team.target_name || null,
    action: team.action || null,
    progress: typeof team.progress === 'number' ? team.progress : 0,
    risk: typeof team.risk === 'number' ? team.risk : 0,
    // A team that is out can be recalled; one that is home or off the board
    // cannot. The same reading the server's own handler takes, so the control is
    // greyed exactly when the command would be a no-op.
    canRecall: team.state === 'deploying' || team.state === 'working',
    canDispatch: team.state === 'available',
  }));
}

/**
 * The eligible (target, action) pairs a Security panel offers, flattened out of
 * the blackboard's target list.
 *
 * `in_range` is the SERVER's answer and is passed straight through as
 * `eligible`: the console must not re-derive a range rule from `separation`,
 * because two readings of one rule is how a panel comes to offer a dispatch the
 * server refuses.
 *
 * @param {object|null|undefined} blackboard the `SecurityBlackboard` payload
 * @param {boolean} [onlyEligible] when true, drop the out-of-reach pairs
 * @returns {Array<object>} one row per (target, action)
 */
export function securityActionRows(blackboard, onlyEligible = false) {
  const targets = (blackboard && blackboard.targets) || [];
  const rows = [];
  for (const target of targets) {
    for (const action of target.actions || []) {
      rows.push({
        target: target.uuid,
        targetName: target.name || null,
        separation: typeof target.separation === 'number' ? target.separation : 0,
        eligible: target.in_range === true,
        action: action.action,
        durationSecs: typeof action.duration_secs === 'number' ? action.duration_secs : 0,
        risk: typeof action.risk === 'number' ? action.risk : 0,
        priority: action.priority || 'optional',
        // A strings.csv id, never English — resolved by the console's own t().
        warning: action.warning || null,
      });
    }
  }
  return onlyEligible ? rows.filter((row) => row.eligible) : rows;
}
