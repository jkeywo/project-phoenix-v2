/**
 * gui/repair-dispatch.js — the Repair console's outbound command surface.
 *
 * This module is the client-side dispatch seam for the PASM entity
 * `repair-console-interface`. It owns the translation from what the Engineering
 * player did ("send team 2 to helm", "send team 0 to core") into the wire
 * payload `SystemControlPayload::DispatchRepairTeam { team_idx, target }`, and
 * it hands that payload to the explicit client command gateway below.
 *
 * The `import` on the next line is the whole point of this file: the Repair
 * console reaches the network *through* `client-network-interface`, and that is
 * now an observable code edge rather than a prose claim.
 */
import { sendControlSystem } from './command-gateway.js';

/**
 * SystemId the host routes repair commands to. Structural addressing, not a
 * tunable gameplay value — it mirrors `REPAIR_SYSTEM_ID` in
 * `src/ship/system_registry.rs`.
 */
export const REPAIR_SYSTEM_ID = 'repair';

/**
 * Build the `RepairTarget` wire value for a console target string.
 *
 * The console addresses *stations* (`'helm'`, `'power'`, ...) plus the single
 * `'core'` bucket for ownerless ship-wide systems. The host resolves a station
 * to the concrete damaged system using the ship's TOML config — the client
 * never decides which system a station contains.
 *
 * @param {string} target lowercase station id, or `'core'`
 * @returns {{type: 'Core'} | {type: 'Station', data: string}}
 */
export function repairTargetFor(target) {
  if (typeof target !== 'string' || target.length === 0) {
    throw new TypeError('repair-dispatch: target must be a non-empty station id');
  }
  return target === 'core'
    ? { type: 'Core' }
    : { type: 'Station', data: target };
}

/**
 * Build the `DispatchRepairTeam` payload without sending it. Exposed so tests
 * (and the shell repair button) can assert on the exact wire shape.
 *
 * @param {number} teamIdx repair team slot index
 * @param {string} target station id or `'core'`
 */
export function dispatchRepairTeamPayload(teamIdx, target) {
  if (!Number.isInteger(teamIdx) || teamIdx < 0) {
    throw new TypeError('repair-dispatch: team_idx must be a non-negative integer');
  }
  return {
    type: 'DispatchRepairTeam',
    data: { team_idx: teamIdx, target: repairTargetFor(target) },
  };
}

/**
 * Dispatch a repair team through the explicit command gateway.
 *
 * @param {number} teamIdx
 * @param {string} target station id or `'core'`
 * @param {((type: string, data?: object) => void)} [send] explicit transport;
 *   omitted when called from a context that has the live ConnectionManager.
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function dispatchRepairTeam(teamIdx, target, send) {
  return sendControlSystem(REPAIR_SYSTEM_ID, dispatchRepairTeamPayload(teamIdx, target), send);
}

/**
 * Build the `SetRepairPriority` payload without sending it. Exposed so tests
 * can assert on the exact wire shape.
 *
 * @param {number} teamIdx repair team slot index
 * @param {number} priority priority value (higher = more urgent)
 * @returns {{type: string, data: object}}
 */
export function setRepairPriorityPayload(teamIdx, priority) {
  if (!Number.isInteger(teamIdx) || teamIdx < 0) {
    throw new TypeError('repair-dispatch: team_idx must be a non-negative integer');
  }
  if (!Number.isInteger(priority) || priority < 0 || priority > 255) {
    throw new TypeError('repair-dispatch: priority must be an integer 0-255');
  }
  return {
    type: 'SetRepairPriority',
    data: { team_idx: teamIdx, priority },
  };
}

/**
 * Set the on-site repair priority for a team through the explicit command
 * gateway. Only takes effect when the team is in `Repairing` state.
 *
 * @param {number} teamIdx
 * @param {number} priority
 * @param {((type: string, data?: object) => void)} [send]
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function setRepairPriority(teamIdx, priority, send) {
  return sendControlSystem(REPAIR_SYSTEM_ID, setRepairPriorityPayload(teamIdx, priority), send);
}
