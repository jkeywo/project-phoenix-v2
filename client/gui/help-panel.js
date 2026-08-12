/**
 * gui/help-panel.js — Station help content for the phone Settings menu.
 *
 * The static station reference text is rendered in the parent client's
 * Settings dialog. Console iframes deliberately mount no duplicate help
 * buttons or overlays.
 */

import { t } from './strings.js';
import { stationDisplayName } from './console-state.js';

const HELP_SECTIONS = {
  captain: [['help.captain.0.heading', 'help.captain.0.body'], ['help.captain.1.heading', 'help.captain.1.body'], ['help.captain.2.heading', 'help.captain.2.body']],
  helm: [['help.helm.0.heading', 'help.helm.0.body'], ['help.helm.1.heading', 'help.helm.1.body'], ['help.helm.2.heading', 'help.helm.2.body'], ['help.helm.3.heading', 'help.helm.3.body'], ['help.helm.4.heading', 'help.helm.4.body']],
  tactical: [['help.tactical.0.heading', 'help.tactical.0.body'], ['help.tactical.1.heading', 'help.tactical.1.body'], ['help.tactical.2.heading', 'help.tactical.2.body'], ['help.tactical.3.heading', 'help.tactical.3.body']],
  repair: [['help.repair.0.heading', 'help.repair.0.body'], ['help.repair.1.heading', 'help.repair.1.body'], ['help.repair.2.heading', 'help.repair.2.body']],
  power: [['help.power.0.heading', 'help.power.0.body'], ['help.power.1.heading', 'help.power.1.body'], ['help.power.2.heading', 'help.power.2.body']],
  shields: [['help.shields.0.heading', 'help.shields.0.body'], ['help.shields.1.heading', 'help.shields.1.body'], ['help.shields.2.heading', 'help.shields.2.body']],
  sensors: [['help.sensors.0.heading', 'help.sensors.0.body'], ['help.sensors.1.heading', 'help.sensors.1.body']],
  navigation: [['help.navigation.0.heading', 'help.navigation.0.body'], ['help.navigation.1.heading', 'help.navigation.1.body'], ['help.navigation.2.heading', 'help.navigation.2.body'], ['help.navigation.3.heading', 'help.navigation.3.body']],
  comms: [['help.comms.0.heading', 'help.comms.0.body'], ['help.comms.1.heading', 'help.comms.1.body'], ['help.comms.2.heading', 'help.comms.2.body'], ['help.comms.3.heading', 'help.comms.3.body']],
  engineering: [['help.engineering.0.heading', 'help.engineering.0.body'], ['help.engineering.1.heading', 'help.engineering.1.body'], ['help.engineering.2.heading', 'help.engineering.2.body'], ['help.engineering.3.heading', 'help.engineering.3.body']],
  science: [['help.science.0.heading', 'help.science.0.body'], ['help.science.1.heading', 'help.science.1.body'], ['help.science.2.heading', 'help.science.2.body']],
};

/** Return the localized help sections for one lowercase station id. */
export function helpSections(stationId) {
  return (HELP_SECTIONS[stationId] || []).map(([heading, body]) => [t(heading), t(body)]);
}

/** True when a station has authored client help. */
export function hasHelp(stationId) {
  return Object.prototype.hasOwnProperty.call(HELP_SECTIONS, stationId);
}

/**
 * Render one station's help into a Settings tab body.
 *
 * @returns {boolean} whether help content was rendered
 */
export function renderStationHelp(root, stationId) {
  if (!root) return false;
  const doc = root.ownerDocument || (typeof document !== 'undefined' ? document : null);
  if (!doc || !hasHelp(stationId)) return false;

  const group = doc.createElement('div');
  group.className = 'station-help-group';
  const heading = doc.createElement('div');
  heading.className = 'station-help-heading';
  heading.textContent = stationDisplayName(stationId);
  group.appendChild(heading);

  const sections = doc.createElement('div');
  sections.className = 'station-help-sections';
  for (const [label, description] of helpSections(stationId)) {
    const section = doc.createElement('div');
    section.className = 'station-help-section';
    const title = doc.createElement('div');
    title.className = 'station-help-section-title';
    title.textContent = label;
    section.appendChild(title);
    const body = doc.createElement('div');
    body.className = 'station-help-section-body';
    body.textContent = description;
    section.appendChild(body);
    sections.appendChild(section);
  }
  group.appendChild(sections);
  root.appendChild(group);
  return true;
}
