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
    title = 'Tactical: come about, bring phasers to bear';
    body = payload.data?.label || payload.label || '';
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
