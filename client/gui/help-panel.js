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
// Keyed by the PascalCase Console enum variant that each panel maps to. The
// Rust enum `HelpPanel` is keyed by panel; the per-console mapping below
// replicates the Console → HelpPanel relationship.

/** @type {Record<string, Array<[string, string]>>} */
const HELP_SECTIONS = {
  CaptainChair: [
    ['Red Alert', 'Toggle ship-wide alert status.'],
    ['View Selector', 'Switch viewscreen camera angle.'],
  ],
  Helm: [
    ['Thrust', 'Drag up to accelerate, down to reverse.'],
    ['Steering', 'Drag left/right to yaw the ship.'],
    ['On Screen', 'Push your radar to the viewscreen.'],
    ['Impulse Drive', '10× speed burst. Cancelled by damage.'],
  ],
  Tactical: [
    ['Target Lock', 'Select a target within range and arc.'],
    ['Phasers', 'Fire at locked target. Auto mode fires when in arc.'],
    ['Torpedoes', 'Launch homing torpedoes from loaded tubes.'],
  ],
  Repair: [
    ['Hull Status', 'Aggregate hull integrity across all systems.'],
    ['Repair Teams', 'Dispatch teams to damaged consoles.'],
    ['Target Console', 'Select which console to repair.'],
  ],
  Power: [
    ['Power Allocation', 'Distribute 6 base power points.'],
    ['Battery Reserve', 'Up to 2 emergency points. Exhaustion locks all.'],
    ['Level Effects', 'Higher levels improve system performance.'],
  ],
  Shields: [
    ['Shield Facings', 'Four quadrants: Fore, Aft, Port, Starboard.'],
    ['Focus', 'Direct capacity to one facing.'],
  ],
  Sensors: [
    ['Long-Range Scan', 'Extended-range radar overlay.'],
    ['Target Hand-off', 'Suggest targets to Tactical.'],
  ],
  Navigation: [
    ['System Chart', 'Push the navigation chart to the viewscreen.'],
    ['Cancel Impulse', 'Abort an active impulse drive charge.'],
  ],
  Comms: [
    ['Contacts', 'List of hailable ships and stations.'],
    ['Messages', 'Inbox of incoming transmissions.'],
    ['Objectives', 'Current mission objectives.'],
  ],
};

/**
 * Return the help sections (array of [title, body] tuples) for `panel`.
 * Mirrors `help_sections(HelpPanel)` in elements.rs. Returns an empty array
 * for an unknown key rather than throwing.
 *
 * @param {string} panel — PascalCase HelpPanel key (e.g. 'Helm', 'Repair').
 * @returns {Array<[string, string]>}
 */
export function helpSections(panel) {
  return HELP_SECTIONS[panel] || [];
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
  heading.textContent = 'HELP — tap to dismiss';
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
 * @param {string} panel — PascalCase HelpPanel key.
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
 * @param {string} panel — PascalCase HelpPanel key.
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
 * @param {string} panel — PascalCase HelpPanel key.
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
