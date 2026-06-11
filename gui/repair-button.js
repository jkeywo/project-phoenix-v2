/**
 * gui/repair-button.js — Pure JS port of the shell-level repair button logic
 * from the Bevy client (formerly src/client/app.rs; ported in #462, the Rust
 * original deleted in #463): `handle_repair_button_press` (the all-busy
 * dispatch guard + message shape) and `refresh_repair_button` (the label /
 * colour / disabled state derivation).
 *
 * This is the *shell* repair affordance (a single "REPAIR" button + hull
 * readout that any bridge console can surface), distinct from the full Repair
 * console iframe (gui/repair-console.html), which owns per-team dispatch with
 * its own DOM and already sends `dispatch_repair_team` actions. This module
 * does NOT duplicate that iframe — it only provides the small derived-state
 * helpers the shell button needs, kept pure so they are unit-testable.
 *
 * TeamSlot wire shape (mirrors src/core/messages.rs TeamSlot): either the
 * string 'Idle', or a tagged object `{ Travelling | Repairing | Returning: {...} }`.
 * A team is "busy" when it is anything other than the 'Idle' string.
 *
 * DOM-free; import in client.html and apply the returned descriptors to the
 * button/readout elements there.
 */

import { repairMessage } from './sim-state.js';

/** True when a single team slot is not the 'Idle' string (i.e. active/busy). */
export function isTeamBusy(slot) {
  return slot !== 'Idle';
}

/**
 * True when every repair team is busy — the press guard from
 * `handle_repair_button_press`. An empty team list is NOT all-busy (matches
 * `Iterator::all` over an empty iterator returning true in Rust, but the shell
 * button is only meaningful with teams present; callers gate on that). We
 * return `false` for an empty list so an empty fleet never blocks the button.
 *
 * @param {Array<string|object>} repairTeams
 * @returns {boolean}
 */
export function allTeamsBusy(repairTeams) {
  const teams = repairTeams || [];
  if (teams.length === 0) return false;
  return teams.every(isTeamBusy);
}

/** True when any repair team is active — drives the refresh label/colour. */
export function anyTeamActive(repairTeams) {
  return (repairTeams || []).some(isTeamBusy);
}

/**
 * Decide whether pressing the shell repair button should send a message, and
 * if so, which one. Mirrors `handle_repair_button_press`: suppress when all
 * teams are busy, otherwise emit the default `repair_message()` (team 0 → Helm).
 *
 * @param {Array<string|object>} repairTeams
 * @returns {{ type: string, data?: object } | null}  message to send, or null
 *   when the press is suppressed.
 */
export function repairButtonPress(repairTeams) {
  if (allTeamsBusy(repairTeams)) return null;
  return repairMessage();
}

/**
 * Derive the shell repair button's visual state from the team list. Mirrors
 * `refresh_repair_button`: when any team is active the button reads
 * "TEAMS DISPATCHED" with the active (dim green) background; otherwise
 * "REPAIR" with the ready (brighter green) background. The label text colour
 * is green in both states (matching the Rust `TextColor`).
 *
 * Colours are returned as CSS strings derived from the Bevy srgb values.
 *
 * @param {Array<string|object>} repairTeams
 * @returns {{ label: string, color: string, background: string, disabled: boolean }}
 */
export function refreshRepairButton(repairTeams) {
  const active = anyTeamActive(repairTeams);
  return {
    label: active ? 'TEAMS DISPATCHED' : 'REPAIR',
    // srgb(0.5, 1.0, 0.5) in both states.
    color: 'rgb(128, 255, 128)',
    // active: srgb(0.05,0.30,0.05); ready: srgb(0.13,0.27,0.13).
    background: active ? 'rgb(13, 77, 13)' : 'rgb(33, 69, 33)',
    // The press itself is guarded by allTeamsBusy; the button is only fully
    // disabled when every team is busy (no dispatch possible).
    disabled: allTeamsBusy(repairTeams),
  };
}
