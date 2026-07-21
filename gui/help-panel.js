/**
 * gui/help-panel.js — Pure JS port of the client-side help system.
 *
 * Ports the 9 `HelpPanel` variants + `help_sections()` static text from the
 * Bevy help system (formerly src/client/elements.rs; ported in #462, the Rust
 * original deleted in #463). Each console can show a "?" help
 * button that opens a dark click-to-dismiss modal overlay describing its
 * controls, matching the look of the old Bevy overlay (dark ~90% alpha
 * background, cyan-ish heading, "HELP — tap to dismiss", click/tap anywhere
 * to dismiss).
 *
 * The shared open/close + render machinery is wired into every console by
 * gui/console-core.js; this module owns the static text and the DOM-building
 * helpers so the logic is unit-testable without a real browser.
 *
 * DOM-free except for the explicitly DOM-taking helpers, which guard on
 * `document` so importing the module in Node (tests) is safe.
 */

// ── Static help text (mirrors elements.rs help_sections) ────────────────────
// Keyed by the lowercase station id that each panel maps to. Pre-issue #618
// these keys were PascalCase Console enum variant names; the JS layer is
// migrating away from the Console enum toward the `StationId` newtype.

/** @type {Record<string, Array<[string, string]>>} */
import { t } from './strings.js';
import { stationDisplayName } from './console-state.js';

const HELP_SECTIONS = {
  captain: [
    ['help.captain.0.heading', 'help.captain.0.body'],
    ['help.captain.1.heading', 'help.captain.1.body'],
    ['help.captain.2.heading', 'help.captain.2.body'],
  ],
  helm: [
    ['help.helm.0.heading', 'help.helm.0.body'],
    ['help.helm.1.heading', 'help.helm.1.body'],
    ['help.helm.2.heading', 'help.helm.2.body'],
    ['help.helm.3.heading', 'help.helm.3.body'],
    ['help.helm.4.heading', 'help.helm.4.body'],
  ],
  tactical: [
    ['help.tactical.0.heading', 'help.tactical.0.body'],
    ['help.tactical.1.heading', 'help.tactical.1.body'],
    ['help.tactical.2.heading', 'help.tactical.2.body'],
    ['help.tactical.3.heading', 'help.tactical.3.body'],
  ],
  repair: [
    ['help.repair.0.heading', 'help.repair.0.body'],
    ['help.repair.1.heading', 'help.repair.1.body'],
    ['help.repair.2.heading', 'help.repair.2.body'],
  ],
  power: [
    ['help.power.0.heading', 'help.power.0.body'],
    ['help.power.1.heading', 'help.power.1.body'],
    ['help.power.2.heading', 'help.power.2.body'],
  ],
  shields: [
    ['help.shields.0.heading', 'help.shields.0.body'],
    ['help.shields.1.heading', 'help.shields.1.body'],
    ['help.shields.2.heading', 'help.shields.2.body'],
  ],
  sensors: [
    ['help.sensors.0.heading', 'help.sensors.0.body'],
    ['help.sensors.1.heading', 'help.sensors.1.body'],
  ],
  navigation: [
    ['help.navigation.0.heading', 'help.navigation.0.body'],
    ['help.navigation.1.heading', 'help.navigation.1.body'],
    ['help.navigation.2.heading', 'help.navigation.2.body'],
  ],
  comms: [
    ['help.comms.0.heading', 'help.comms.0.body'],
    ['help.comms.1.heading', 'help.comms.1.body'],
    ['help.comms.2.heading', 'help.comms.2.body'],
    ['help.comms.3.heading', 'help.comms.3.body'],
  ],
  engineering: [
    ['help.engineering.0.heading', 'help.engineering.0.body'],
    ['help.engineering.1.heading', 'help.engineering.1.body'],
    ['help.engineering.2.heading', 'help.engineering.2.body'],
    ['help.engineering.3.heading', 'help.engineering.3.body'],
  ],
  science: [
    ['help.science.0.heading', 'help.science.0.body'],
    ['help.science.1.heading', 'help.science.1.body'],
    ['help.science.2.heading', 'help.science.2.body'],
  ],
};

/**
 * Return the help sections (array of [title, body] tuples) for `panel`.
 * Mirrors `help_sections(HelpPanel)` in elements.rs. Returns an empty array
 * for an unknown key rather than throwing.
 *
 * @param {string} panel — lowercase station id (e.g. 'helm', 'repair').
 * @returns {Array<[string, string]>}
 */
export function helpSections(panel) {
  // Entries are string-id pairs; resolve to display text at read time so a
  // late-loaded table (or locale switch) is picked up on next open.
  return (HELP_SECTIONS[panel] || []).map(([h, b]) => [t(h), t(b)]);
}

/** True when `panel` has help text defined. */
export function hasHelp(panel) {
  return Object.prototype.hasOwnProperty.call(HELP_SECTIONS, panel);
}

// ── Modal machinery (DOM) ────────────────────────────────────────────────────
//
// A single shared overlay element is created lazily per document and reused
// for every open/close. The overlay dismisses on any click/tap. The trigger
// button is built per console by `createHelpButton`.

const OVERLAY_ID = 'help-overlay';

/**
 * Build (once) and return the shared help overlay element for `doc`.
 * Idempotent: subsequent calls return the existing element.
 *
 * @param {Document} doc
 * @returns {HTMLElement}
 */
function ensureOverlay(doc) {
  let overlay = doc.getElementById(OVERLAY_ID);
  if (overlay) return overlay;

  overlay = doc.createElement('div');
  overlay.id = OVERLAY_ID;
  overlay.className = 'help-overlay';
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-hidden', 'true');
  overlay.hidden = true;

  // Dismiss on click/tap anywhere on the overlay.
  overlay.addEventListener('click', () => closeHelp(doc));

  (doc.body || doc.documentElement).appendChild(overlay);
  return overlay;
}

/**
 * Populate the overlay element with the heading + sections for `panel`.
 * @param {HTMLElement} overlay
 * @param {string} panel
 */
function renderOverlayContent(overlay, panel) {
  const doc = overlay.ownerDocument;
  overlay.innerHTML = '';

  const heading = doc.createElement('div');
  heading.className = 'help-heading';
  heading.textContent = t('help.modal_heading');
  overlay.appendChild(heading);

  const body = doc.createElement('div');
  body.className = 'help-sections';
  for (const [label, desc] of helpSections(panel)) {
    const section = doc.createElement('div');
    section.className = 'help-section';

    const title = doc.createElement('div');
    title.className = 'help-section-title';
    title.textContent = label;
    section.appendChild(title);

    const text = doc.createElement('div');
    text.className = 'help-section-body';
    text.textContent = desc;
    section.appendChild(text);

    body.appendChild(section);
  }
  overlay.appendChild(body);
}

/**
 * Open the help modal for `panel` in `doc`. Builds the overlay on first use,
 * fills it with the panel's sections, and reveals it.
 *
 * @param {string} panel — lowercase station id (e.g. 'helm').
 * @param {Document} [doc=document]
 */
export function openHelp(panel, doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return;
  const overlay = ensureOverlay(doc);
  renderOverlayContent(overlay, panel);
  overlay.hidden = false;
  overlay.setAttribute('aria-hidden', 'false');
  overlay.classList.add('open');
}

/**
 * Close the help modal in `doc` (no-op if it was never opened).
 * @param {Document} [doc=document]
 */
export function closeHelp(doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return;
  const overlay = doc.getElementById(OVERLAY_ID);
  if (!overlay) return;
  overlay.hidden = true;
  overlay.setAttribute('aria-hidden', 'true');
  overlay.classList.remove('open');
}

/** True when the help modal in `doc` is currently open. */
export function isHelpOpen(doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return false;
  const overlay = doc.getElementById(OVERLAY_ID);
  return !!overlay && overlay.hidden === false;
}

/**
 * Create a "?" help button element wired to open `panel`'s help modal.
 * The caller appends it wherever it wants the trigger to live.
 *
 * @param {string} panel — lowercase station id (e.g. 'helm').
 * @param {Document} [doc=document]
 * @returns {HTMLButtonElement|null}
 */
export function createHelpButton(panel, doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return null;
  const btn = doc.createElement('button');
  btn.type = 'button';
  btn.className = 'help-btn';
  btn.setAttribute('aria-label', 'Help');
  btn.title = 'Help';
  btn.textContent = '?';
  btn.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    openHelp(panel, doc);
  });
  return btn;
}

/**
 * Mount the help system for a console: find or create a help button and wire
 * it to `panel`. If the document already contains an element with
 * `[data-help-button]`, that element becomes the trigger; otherwise a new
 * "?" button is appended to the `.frame` (or body) so every console gets help
 * with minimal per-file HTML.
 *
 * Returns the trigger element (or null when no document is available).
 *
 * @param {string} panel — lowercase station id (e.g. 'helm').
 * @param {Document} [doc=document]
 * @returns {HTMLElement|null}
 */
export function mountHelp(panel, doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc || !hasHelp(panel)) return null;

  // Pre-build the overlay so the first tap is instant.
  ensureOverlay(doc);

  let trigger = doc.querySelector('[data-help-button]');
  if (trigger) {
    trigger.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      openHelp(panel, doc);
    });
    return trigger;
  }

  trigger = createHelpButton(panel, doc);
  if (!trigger) return null;
  const host = doc.querySelector('.frame') || doc.body || doc.documentElement;
  host.appendChild(trigger);
  return trigger;
}

// ── Inline help panel (rendered in the parent page under the tab bar) ─────

/**
 * Render help sections for multiple consoles into an inline container.
 * Each console gets a heading group with its help sections listed below.
 * Used by client.html's render() to display all help text under the tab bar
 * when a console is selected and non-selected tabs are hidden.
 *
 * @param {HTMLElement} root - the container element to populate
 * @param {string[]} consoles - array of console names (see caller for casing)
 */
export function renderInlineHelp(root, consoles) {
  if (!root) return;
  const doc = root.ownerDocument || document;
  root.innerHTML = '';
  for (const consoleName of consoles || []) {
    const sections = helpSections(consoleName);
    if (sections.length === 0) continue;
    const group = doc.createElement('div');
    group.className = 'help-console-group';
    const heading = doc.createElement('div');
    heading.className = 'help-console-heading';
    // Station labels resolve through the string table (station.<id>.name) —
    // the tab-bar CONSOLE_LABEL map was deleted with the tab bar (#827).
    heading.textContent = stationDisplayName(consoleName);
    group.appendChild(heading);
    const body = doc.createElement('div');
    body.className = 'help-sections';
    for (const [sectionLabel, desc] of sections) {
      const section = doc.createElement('div');
      section.className = 'help-section';
      const title = doc.createElement('div');
      title.className = 'help-section-title';
      title.textContent = sectionLabel;
      section.appendChild(title);
      const text = doc.createElement('div');
      text.className = 'help-section-body';
      text.textContent = desc;
      section.appendChild(text);
      body.appendChild(section);
    }
    group.appendChild(body);
    root.appendChild(group);
  }
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.renderInlineHelp = renderInlineHelp;
}
