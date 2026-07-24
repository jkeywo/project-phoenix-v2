/**
 * gui/manual-panel.js — Read-only ship manual surface (issue #772).
 *
 * A book button (bottom-left, beside the settings gear) opens a click-to-dismiss
 * overlay with a TAB for EVERY authored station on the selected ship — unlike
 * the console, which shows only the player's own station. Each tab shows the
 * station's ship-authored overview prose followed by generated, structured
 * system sections (shield strength, arc count, regen, and the rating→AI
 * automation mapping for the owning station).
 *
 * PRESENTATION STATE ONLY (AGENTS.md #3): this module reads the replicated
 * `simState.shipManual` and never sends, dispatches, or mutates anything — there
 * is no `send` handle anywhere in here.
 *
 * Localisation: the wire carries STRUCTURED data — a section `kind`, per-metric
 * machine `code`s, numeric values, and system/rating identifiers — never
 * composed English (see `src/ship/manual.rs`). This module maps those codes to
 * `assets/strings/strings.csv` ids and resolves them via `t()`, interpolating
 * the numbers. The one exception is `station.overview`, which is literal
 * authored prose replicated from the ship TOML and rendered as-is.
 *
 * DOM-free except for the explicitly DOM-taking helpers, which guard on
 * `document` so importing the module in Node (tests) is safe.
 */

import { t, has } from './strings.js';
import { stationDisplayName } from './console-state.js';

// ── Pure formatting helpers ──────────────────────────────────────────────────

/**
 * Format a manual metric value: whole numbers render without a decimal tail,
 * fractional values keep up to two decimals.
 * @param {number} value
 * @returns {string}
 */
export function formatMetricValue(value) {
  if (typeof value !== 'number' || !isFinite(value)) return '';
  if (Number.isInteger(value)) return String(value);
  return String(Math.round(value * 100) / 100);
}

/**
 * Resolve a rating name to a display caption, reusing the existing
 * `station.rating.<lower>.name` ids and falling back to the raw authored name.
 * @param {string} rating
 * @returns {string}
 */
export function ratingCaption(rating) {
  const id = 'station.rating.' + String(rating).toLowerCase() + '.name';
  return has(id) ? t(id) : String(rating);
}

// ── Pure section rendering ───────────────────────────────────────────────────

/**
 * Build the DOM for one generated system section: a heading, each metric as a
 * `label: value` row (label via `t()`, value interpolated), and the rating→AI
 * automation list.
 *
 * @param {Document} doc
 * @param {{ kind: string, metrics: Array, automation: Array }} section
 * @returns {HTMLElement}
 */
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
    // Label id is composed from the section kind + metric code; the value is
    // interpolated by t(), so all English stays in strings.csv.
    row.textContent = t('manual.' + section.kind + '.' + metric.code, {
      value: formatMetricValue(metric.value),
    });
    el.appendChild(row);
  }

  const automation = section.automation || [];
  if (automation.length > 0) {
    const autoHeading = doc.createElement('div');
    autoHeading.className = 'manual-automation-heading';
    autoHeading.textContent = t('manual.automated_systems');
    el.appendChild(autoHeading);

    for (const row of automation) {
      const line = doc.createElement('div');
      line.className = 'manual-automation-row';
      const label = doc.createElement('span');
      label.className = 'manual-rating-name';
      label.textContent = ratingCaption(row.rating);
      line.appendChild(label);
      const systems = doc.createElement('span');
      systems.className = 'manual-rating-systems';
      // System ids are machine identifiers (not English); render them as-is.
      const ids = row.automated_systems || [];
      systems.textContent = ids.length > 0 ? ids.join(', ') : t('manual.rating.none');
      line.appendChild(systems);
      el.appendChild(line);
    }
  }

  return el;
}

/**
 * Build the content panel for one station tab: authored overview + sections.
 *
 * @param {Document} doc
 * @param {{ station_id: string, overview?: string|null, sections?: Array }} station
 * @returns {HTMLElement}
 */
export function renderStationPanel(doc, station) {
  const panel = doc.createElement('div');
  panel.className = 'manual-station-panel';

  if (station.overview) {
    const overview = doc.createElement('div');
    overview.className = 'manual-overview';
    // Authored prose replicated from the ship TOML — rendered verbatim.
    overview.textContent = station.overview;
    panel.appendChild(overview);
  }

  for (const section of station.sections || []) {
    panel.appendChild(renderSection(doc, section));
  }

  return panel;
}

/**
 * Render the whole manual into `root`: a tab strip (one tab per authored
 * station) plus the active station's panel. Selecting a tab re-renders the
 * panel. Read-only — no server messages are ever sent.
 *
 * @param {HTMLElement} root
 * @param {{ stations: Array }|null} manual
 * @param {number} [activeIndex=0]
 * @returns {number} the number of station tabs rendered
 */
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

  stations.forEach((station, i) => {
    const tab = doc.createElement('button');
    tab.type = 'button';
    tab.className = 'manual-tab' + (i === active ? ' active' : '');
    tab.textContent = stationDisplayName(station.station_id);
    tab.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      active = i;
      drawPanel();
    });
    tabBar.appendChild(tab);
  });

  drawPanel();
  return stations.length;
}

// ── Modal machinery (DOM) ────────────────────────────────────────────────────

const OVERLAY_ID = 'manual-overlay';
const BUTTON_ID = 'manual-btn';

function ensureOverlay(doc) {
  let overlay = doc.getElementById(OVERLAY_ID);
  if (overlay) return overlay;

  overlay = doc.createElement('div');
  overlay.id = OVERLAY_ID;
  overlay.className = 'manual-overlay';
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-hidden', 'true');
  overlay.hidden = true;
  // Dismiss when the backdrop (but not the inner popup) is clicked.
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeManual(doc);
  });
  (doc.body || doc.documentElement).appendChild(overlay);
  return overlay;
}

/**
 * Open the manual overlay in `doc`, rendering the current replica from
 * `getManual()`.
 * @param {() => ({stations: Array}|null)} getManual
 * @param {Document} [doc=document]
 */
export function openManual(getManual, doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return;
  const overlay = ensureOverlay(doc);
  overlay.innerHTML = '';

  const popup = doc.createElement('div');
  popup.className = 'manual-popup';
  overlay.appendChild(popup);

  const heading = doc.createElement('div');
  heading.className = 'manual-heading';
  heading.textContent = t('manual.heading');
  popup.appendChild(heading);

  const body = doc.createElement('div');
  body.className = 'manual-body';
  popup.appendChild(body);
  renderManual(body, typeof getManual === 'function' ? getManual() : null);

  overlay.hidden = false;
  overlay.setAttribute('aria-hidden', 'false');
  overlay.classList.add('open');
}

/** Close the manual overlay in `doc` (no-op if never opened). */
export function closeManual(doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return;
  const overlay = doc.getElementById(OVERLAY_ID);
  if (!overlay) return;
  overlay.hidden = true;
  overlay.setAttribute('aria-hidden', 'true');
  overlay.classList.remove('open');
}

/** True when the manual overlay in `doc` is currently open. */
export function isManualOpen(doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return false;
  const overlay = doc.getElementById(OVERLAY_ID);
  return !!overlay && overlay.hidden === false;
}

/**
 * Mount the manual button + overlay. `getManual` returns the current read-only
 * replica (typically `() => window.simState && window.simState.shipManual`).
 *
 * @param {{ getManual: function, doc?: Document }} opts
 * @returns {{ open: function, close: function }}
 */
export function mountManual({ getManual, doc: _doc } = {}) {
  const doc = _doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return { open() {}, close() {} };

  let btn = doc.getElementById(BUTTON_ID);
  if (!btn) {
    btn = doc.createElement('button');
    btn.id = BUTTON_ID;
    btn.className = 'manual-btn';
    btn.setAttribute('aria-label', t('manual.title'));
    btn.title = t('manual.title');
    btn.textContent = '\u{1F4D6}'; // open-book glyph
    doc.body.appendChild(btn);
  }
  btn.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    openManual(getManual, doc);
  });

  ensureOverlay(doc);

  return {
    open: () => openManual(getManual, doc),
    close: () => closeManual(doc),
  };
}

// Expose for non-module bootstrap in client.html.
if (typeof window !== 'undefined') {
  window.mountManual = mountManual;
}
