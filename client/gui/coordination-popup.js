/**
 * gui/coordination-popup.js — Pure payload normaliser for the AI-to-human
 * coordination popup (issues #494, #827).
 *
 * Extracted from client.html's showCoordinationPopup(): maps every
 * CoordinationPopup payload variant onto { sender, title, body }. The DOM
 * show/dismiss (element writes, 8s auto-dismiss timer) stays inline in
 * client.html.
 */

/**
 * Normalise a CoordinationPopup payload to display strings.
 *
 * @param {{ type?: string, target?: string, data?: object }} payload
 * @param {string|null|undefined} senderLabel  msg.data.sender_label
 * @returns {{ sender: string, title: string, body: string }}
 */
export function normalizeCoordinationPayload(payload, senderLabel) {
  const label = senderLabel || 'AI';
  const sender = payload.target ? (label + ' ? ' + payload.target) : label;

  let title, body;
  if (payload.type === 'Advisory') {
    title = label || 'Advisory';
    body = payload.data?.message || payload.message || '';
  } else if (payload.type === 'Alert') {
    title = payload.data?.title || payload.title || 'Alert';
    body = payload.data?.body || payload.body || '';
  } else if (payload.type === 'FrequencyHint') {
    title = 'Frequency Hint';
    body = 'Tune to: ' + (payload.data?.frequency ?? payload.frequency ?? '?');
  } else if (payload.type === 'ShieldFacingDown') {
    title = (payload.data?.label || payload.label || 'Shield') + ' Offline';
    body = '';
  } else if (payload.type === 'ShieldFacingRestored') {
    title = (payload.data?.label || payload.label || 'Shield') + ' Restored';
    body = '';
  } else if (payload.type === 'TargetDesignation') {
    title = 'Sensors designates: ' + (payload.data?.label || payload.label || '?');
    body = '';
  } else if (payload.type === 'ArcBearingRequest') {
    // Family-aware (issue #767): name the weapon family that needs the bearing.
    // `family` is the serialised WeaponFamily variant; defaults to phasers for
    // pre-#767 payloads that omit it.
    const family = payload.data?.family || payload.family;
    const weapon = family === 'Blasters' ? 'blasters'
      : family === 'Torpedoes' ? 'torpedoes'
      : 'phasers';
    title = 'Tactical: come about, bring ' + weapon + ' to bear';
    body = payload.data?.label || payload.label || '';
  } else if (payload.type === 'ArcBearingWithdraw') {
    // Issue #932: the standing request above is pulled because its family
    // went unusable, not because the bearing itself changed.
    const family = payload.data?.family || payload.family;
    const weapon = family === 'Blasters' ? 'blasters'
      : family === 'Torpedoes' ? 'torpedoes'
      : 'phasers';
    title = 'Belay that — ' + weapon + ' no longer able to bear';
    body = '';
  } else if (payload.type === 'IntentAdvisory') {
    // Issue #879: a backfilled seat telling the rest of the crew what it just
    // decided. The host sends a typed kind plus at most one label — it never
    // sends the sentence, and never a figure — so the sentence is built here,
    // following this file's existing inline-English chatter pattern.
    const kind = payload.data?.kind || payload.kind;
    const subject = payload.data?.subject ?? payload.subject ?? '';
    const INTENT_TITLES = {
      TargetAcquired: 'Target acquired',
      TargetSwitched: 'Switching target',
      CombatPostureEntered: 'Combat posture',
      CombatPostureLeft: 'Standing down',
      BreakingOff: 'Breaking off',
      ShieldArcFocused: 'Focusing shields',
      PowerBrownout: 'Power brownout',
      ManoeuvreBegun: 'Manoeuvring',
    };
    title = INTENT_TITLES[kind] || kind || 'Advisory';
    body = subject;
  } else if (payload.type === 'PowerBrownout') {
    const sysLabel = payload.data?.label || payload.label || 'System';
    const level = payload.data?.allocated_level ?? payload.allocated_level ?? '?';
    title = sysLabel + ' Power Brownout';
    body = 'Allocation: ' + level;
  } else {
    title = payload.type || 'Advisory';
    body = '';
  }

  return { sender, title, body };
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.normalizeCoordinationPayload = normalizeCoordinationPayload;
}
