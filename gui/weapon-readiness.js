// Shared weapon-readiness rendering path for the three Tactical weapon panels
// (phasers, blasters, torpedoes) — issue #764.
//
// The server publishes a common `readiness` contract on every weapon-family
// per-instance state (`WeaponReadiness` in core/messages.rs):
//
//   { ready: bool, blocking_reason: string, target_range: number|null,
//     target_arc: number|null }
//
// where `blocking_reason` is the serialised `WeaponBlockReason` variant name.
// This module maps that contract onto ONE render-friendly view + display label
// so all three panels render equivalent ready / blocked(reason) / unavailable
// states. Keeping the mapping here is what stops the three panels drifting
// apart the way they had before this issue.
import { t } from './strings.js';

// Blocking reason (server enum variant name) → shared display-label string id.
// `Ready` has no label (the weapon can fire). Reuses the existing common ids
// for cooldown / no-target so all three panels map reasons to text identically.
const REASON_LABEL_IDS = {
  NoTarget: 'console.common.no_target',
  OutOfRange: 'console.common.out_of_range',
  OutOfArc: 'console.common.out_of_arc',
  Cooldown: 'console.common.cooldown',
  Loading: 'console.common.loading',
  NoAmmo: 'console.common.no_ammo',
  Offline: 'console.common.offline',
};

/**
 * Display label for a blocking reason, via the shared string table. Returns
 * '' for the not-blocked (`Ready`) state or an unknown reason.
 * @param {string} reason - serialised WeaponBlockReason variant name
 * @returns {string}
 */
export function weaponBlockLabel(reason) {
  const id = REASON_LABEL_IDS[reason];
  return id ? t(id) : '';
}

/**
 * Normalise a weapon `readiness` contract into the shared render view every
 * panel consumes. Tolerates a missing contract (legacy/no-readiness states)
 * by reporting `unknown: true` so callers can fall back to their own derivation.
 *
 * @param {{ready?: boolean, blocking_reason?: string,
 *          target_range?: number|null, target_arc?: number|null}} [readiness]
 * @returns {{ present: boolean, ready: boolean, unavailable: boolean,
 *            blocked: boolean, reason: string, label: string,
 *            range: number|null, arc: number|null }}
 */
export function weaponReadinessView(readiness) {
  if (!readiness || typeof readiness.blocking_reason !== 'string') {
    return {
      present: false,
      ready: false,
      unavailable: false,
      blocked: false,
      reason: '',
      label: '',
      range: null,
      arc: null,
    };
  }
  const reason = readiness.blocking_reason;
  const ready = !!readiness.ready && reason === 'Ready';
  // "Unavailable" is the offline state — the weapon is not merely blocked by a
  // transient condition but disabled/destroyed. Rendered distinctly.
  const unavailable = reason === 'Offline';
  return {
    present: true,
    ready,
    unavailable,
    blocked: !ready,
    reason,
    label: ready ? '' : weaponBlockLabel(reason),
    range: readiness.target_range ?? null,
    arc: readiness.target_arc ?? null,
  };
}
