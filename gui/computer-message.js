import { has, localiseTree, t } from './strings.js';

/** The current permitted ship's-computer message, shared by both Viewscreens.
 * Only the existing HUD payload is consumed. No message history or audio trigger.
 */
export function createComputerMessageBanner(doc = document) {
  const banner = doc.createElement('section');
  banner.id = 'hud-computer-message'; banner.className = 'vs-computer-message';
  banner.hidden = true; banner.setAttribute('role', 'status');
  banner.setAttribute('aria-atomic', 'true');
  const source = doc.createElement('div'); source.className = 'vs-computer-message-source';
  const label = doc.createElement('span');
  const severity = doc.createElement('span'); severity.id = 'hud-computer-message-severity';
  const station = doc.createElement('span'); station.id = 'hud-computer-message-station';
  source.append(label, severity, station);
  const text = doc.createElement('div'); text.id = 'hud-computer-message-text';
  text.className = 'vs-computer-message-text'; banner.append(source, text); doc.body.appendChild(banner);
  let previous = null;
  function update(value) {
    const valid = value && typeof value.text === 'string'
      && ['info', 'advisory', 'warning', 'critical'].includes(value.severity);
    const key = valid ? JSON.stringify(value) : null;
    if (key === previous) return;
    previous = key; banner.hidden = !valid;
    label.textContent = valid ? localiseTree('server.computer_message.label') : '';
    banner.className = 'vs-computer-message' + (valid ? ' shown sev-' + value.severity : '');
    text.textContent = valid ? localiseTree(value.text) : '';
    severity.textContent = valid ? ' · ' + t('server.computer_message.severity.' + value.severity) : '';
    const stationId = valid && value.station ? 'station.' + value.station + '.name' : null;
    station.textContent = stationId && has(stationId) ? ' · ' + t(stationId) : '';
  }
  return { update, destroy() { banner.remove(); } };
}
