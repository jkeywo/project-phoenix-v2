/**
 * gui/manual-panel.js — Read-only ship manual content for the phone Settings menu.
 *
 * The caller owns the Settings dialog; this module only renders the replicated
 * manual and never creates buttons, overlays, or network messages.
 */

import { t, has } from './strings.js';
import { stationDisplayName } from './console-state.js';

/** Format a manual metric without an unnecessary decimal tail. */
export function formatMetricValue(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return String(value);
  return Number.isInteger(number) ? String(number) : String(Number(number.toFixed(2)));
}

/** Resolve a rating identifier to display text. */
export function ratingCaption(rating) {
  const id = 'station.rating.' + String(rating).toLowerCase() + '.name';
  return has(id) ? t(id) : String(rating);
}

/** Render one system section from the structured manual replica. */
export function renderSection(doc, section) {
  const el = doc.createElement('div');
  el.className = 'manual-section';
  const heading = doc.createElement('div');
  heading.className = 'manual-section-heading';
  heading.textContent = t('manual.section.' + section.kind);
  el.appendChild(heading);

  for (const metric of section.metrics || []) {
    const row = doc.createElement('div');
    row.className = 'manual-metric';
    row.textContent = t('manual.' + section.kind + '.' + metric.code, { value: formatMetricValue(metric.value) });
    el.appendChild(row);
  }
  for (const capability of section.capabilities || []) {
    const row = doc.createElement('div');
    row.className = 'manual-capability';
    const valueId = 'manual.' + section.kind + '.' + capability.code + '.' + capability.value_code;
    const value = has(valueId) ? t(valueId) : String(capability.value_code);
    row.textContent = t('manual.' + section.kind + '.' + capability.code, { value });
    el.appendChild(row);
  }

  if ((section.automation || []).length > 0) {
    const automationHeading = doc.createElement('div');
    automationHeading.className = 'manual-automation-heading';
    automationHeading.textContent = t('manual.automated_systems');
    el.appendChild(automationHeading);
    for (const automation of section.automation) {
      const row = doc.createElement('div');
      row.className = 'manual-automation-row';
      const label = doc.createElement('span');
      label.className = 'manual-rating-name';
      label.textContent = ratingCaption(automation.rating);
      row.appendChild(label);
      const systems = doc.createElement('span');
      systems.className = 'manual-rating-systems';
      const ids = automation.automated_systems || [];
      systems.textContent = ids.length > 0 ? ids.join(', ') : t('manual.rating.none');
      row.appendChild(systems);
      el.appendChild(row);
    }
  }
  return el;
}

/** Render one station's authored overview and generated sections. */
export function renderStationPanel(doc, station) {
  const panel = doc.createElement('div');
  panel.className = 'manual-station-panel';
  if (station.overview) {
    const overview = doc.createElement('div');
    overview.className = 'manual-overview';
    overview.textContent = station.overview;
    panel.appendChild(overview);
  }
  for (const section of station.sections || []) panel.appendChild(renderSection(doc, section));
  return panel;
}

/** Render the manual's station selector and active station detail. */
export function renderManual(root, manual, activeIndex = 0) {
  if (!root) return 0;
  const doc = root.ownerDocument || (typeof document !== 'undefined' ? document : null);
  if (!doc) return 0;
  root.innerHTML = '';
  const stations = (manual && manual.stations) || [];
  if (stations.length === 0) {
    const empty = doc.createElement('div');
    empty.className = 'manual-empty';
    empty.textContent = t('manual.empty');
    root.appendChild(empty);
    return 0;
  }

  let active = Math.max(0, Math.min(activeIndex, stations.length - 1));
  const tabBar = doc.createElement('div');
  tabBar.className = 'manual-tabs';
  root.appendChild(tabBar);
  const panelHost = doc.createElement('div');
  panelHost.className = 'manual-panel-host';
  root.appendChild(panelHost);
  const drawPanel = () => {
    panelHost.innerHTML = '';
    panelHost.appendChild(renderStationPanel(doc, stations[active]));
    for (let i = 0; i < tabBar.children.length; i += 1) {
      if (i === active) tabBar.children[i].classList.add('active');
      else tabBar.children[i].classList.remove('active');
    }
  };
  stations.forEach((station, index) => {
    const tab = doc.createElement('button');
    tab.type = 'button';
    tab.className = 'manual-tab' + (index === active ? ' active' : '');
    tab.textContent = stationDisplayName(station.station_id);
    tab.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      active = index;
      drawPanel();
    });
    tabBar.appendChild(tab);
  });
  drawPanel();
  return stations.length;
}
