/**
 * gui/coordination-popup.js — Pure payload normaliser for the AI-to-human
 * coordination popup (issues #494, #827) AND the viewscreen chatter bubble
 * (issue #975).
 *
 * Maps every CoordinationPayload variant onto { sender, title, body }. This is
 * the ONE place a coordination payload becomes a sentence: both the phone popup
 * (client.html `showCoordinationPopup`) and the host viewscreen chatter widget
 * (server.html `__updateChatter`) resolve through it, so the same event renders
 * identical text on both surfaces (issue #975). The host previously composed a
 * parallel English sentence in Rust (`format_coordination_chatter`); that is
 * gone — Rust now emits the typed payload and the ids, and the words live only
 * in `assets/strings/strings.csv`, resolved here through `t()`.
 *
 * The DOM show/dismiss (element writes, auto-dismiss timer) stays inline in the
 * host pages.
 */

import { t } from './strings.js';

/**
 * Weapon-family display name (issue #767). Literal `t(...)` calls, one per
 * family, so scripts/check-strings.mjs can verify each id has a CSV row.
 * @param {string|undefined} family serialised WeaponFamily variant
 */
function weaponFamilyLabel(family) {
  if (family === 'Blasters') return t('coordination.weapon_family.blasters');
  if (family === 'Torpedoes') return t('coordination.weapon_family.torpedoes');
  // Phasers by default, including pre-#767 payloads that omit the field.
  return t('coordination.weapon_family.phasers');
}

/**
 * IntentAdvisory headline (issue #879). A `switch` of literal `t(...)` calls
 * rather than a map, so check-strings validates every id; an unknown kind
 * renders its own token, matching the host's closed IntentKind set drifting
 * ahead of the client.
 * @param {string|undefined} kind serialised IntentKind variant
 */
function intentTitle(kind) {
  switch (kind) {
    case 'TargetAcquired': return t('coordination.intent.target_acquired');
    case 'TargetSwitched': return t('coordination.intent.target_switched');
    case 'CombatPostureEntered': return t('coordination.intent.combat_posture_entered');
    case 'CombatPostureLeft': return t('coordination.intent.combat_posture_left');
    case 'BreakingOff': return t('coordination.intent.breaking_off');
    case 'ShieldArcFocused': return t('coordination.intent.shield_arc_focused');
    case 'PowerBrownout': return t('coordination.intent.power_brownout');
    case 'ManoeuvreBegun': return t('coordination.intent.manoeuvre_begun');
    default: return kind || t('coordination.advisory.fallback_title');
  }
}

/**
 * Normalise a CoordinationPopup payload to display strings.
 *
 * `senderLabel` and any id-bearing field in `payload` are already resolved by
 * `localiseTree` at the wire boundary (issue #949) before this runs, so the
 * only ids this function resolves are the sentence templates it introduces.
 *
 * @param {{ type?: string, target?: string, data?: object }} payload
 * @param {string|null|undefined} senderLabel  the localised origin label
 * @returns {{ sender: string, title: string, body: string }}
 */
export function normalizeCoordinationPayload(payload, senderLabel) {
  const label = senderLabel || t('chatter.sender.ai');
  const sender = payload.target ? (label + ' ? ' + payload.target) : label;
  const data = payload.data || {};

  let title;
  let body = '';
  const type = payload.type;

  if (type === 'Advisory') {
    title = label || t('coordination.advisory.fallback_title');
    body = data.message ?? payload.message ?? '';
  } else if (type === 'Alert') {
    title = data.title ?? payload.title ?? t('coordination.alert.fallback_title');
    body = data.body ?? payload.body ?? '';
  } else if (type === 'FrequencyHint') {
    title = t('coordination.frequency_hint.title');
    body = t('coordination.frequency_hint.body', {
      frequency: data.frequency ?? payload.frequency ?? '?',
    });
  } else if (type === 'ShieldFacingDown') {
    title = t('coordination.shield_offline.title', {
      label: data.label || payload.label || t('coordination.shield.fallback_label'),
    });
  } else if (type === 'ShieldFacingRestored') {
    title = t('coordination.shield_restored.title', {
      label: data.label || payload.label || t('coordination.shield.fallback_label'),
    });
  } else if (type === 'TargetDesignation') {
    title = t('coordination.target_designation.title', {
      label: data.label || payload.label || '?',
    });
  } else if (type === 'ArcBearingRequest') {
    title = t('coordination.arc_bearing.title', {
      weapon: weaponFamilyLabel(data.family || payload.family),
    });
    body = data.label || payload.label || '';
  } else if (type === 'ArcBearingWithdraw') {
    title = t('coordination.arc_withdraw.title', {
      weapon: weaponFamilyLabel(data.family || payload.family),
    });
  } else if (type === 'PowerBrownout') {
    title = t('coordination.power_brownout.title', {
      label: data.label || payload.label || t('coordination.system.fallback_label'),
    });
    body = t('coordination.power_brownout.body', {
      level: data.allocated_level ?? payload.allocated_level ?? '?',
    });
  } else if (type === 'IntentAdvisory') {
    title = intentTitle(data.kind || payload.kind);
    body = data.subject ?? payload.subject ?? '';
  } else if (type === 'NavigateTo') {
    title = t('coordination.navigate.title', {
      label: data.label || payload.label || '?',
    });
  } else if (type === 'RepairRequest') {
    title = t('coordination.repair.title', {
      label: data.station_label || payload.station_label || '?',
    });
  } else if (type === 'ThreatBearing') {
    const rad = data.bearing_rad ?? payload.bearing_rad ?? 0;
    const deg = Math.round((((rad * 180) / Math.PI) % 360 + 360) % 360);
    title = t('coordination.threat_bearing.title', {
      deg,
      label: data.label || payload.label || '?',
    });
  } else {
    title = payload.type || t('coordination.advisory.fallback_title');
  }

  return { sender, title, body };
}

// Expose for the non-module inline scripts in client.html and server.html.
if (typeof window !== 'undefined') {
  window.normalizeCoordinationPayload = normalizeCoordinationPayload;
}
