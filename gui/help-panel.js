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
const HELP_SECTIONS = {
  captain: [
    ['Command', 'You set the ship\'s posture. Coordinate the crew and call the shots — no one else has the full picture.'],
    ['Red Alert', 'Raises ship-wide combat readiness. Call it before entering a fight, not after taking the first hit.'],
    ['View Selector', 'Keep the main screen camera updated so the whole bridge sees what matters right now.'],
  ],
  helm: [
    ['Pilot', 'Keep the ship moving and the target in arc for Tactical. You control where the fight happens.'],
    ['Thrust & Steering', 'Drag to accelerate, reverse, or steer. WASD or the arrow keys steer too, as does a gamepad stick.'],
    ['Impulse Drive', '10× speed burst for rapid travel. Damage cancels it, so it\'s best for non-combat travel. Press Ctrl or gamepad B to charge, again to cancel.'],
    ['Boost', 'Hold Shift or gamepad A for 3× speed while the battery lasts. It drains in a few seconds and takes far longer to refill, so save it for closing a gap.'],
    ['On Screen', 'Push your radar to the viewscreen when someone needs to see your situation.'],
  ],
  tactical: [
    ['Weapons Officer', 'Deliver firepower to the enemy.'],
    ['Target Lock', 'Lock first — phasers and torpedoes both require an active lock by clicking on the target.'],
    ['Phasers', 'Fast and continuous but arc-limited. Enable Auto so they fire the instant the target crosses your fire arc.'],
    ['Torpedoes', 'Use them when on priority target. They do less damage to shields than hull, sensors can monitor target shields.'],
  ],
  repair: [
    ['Damage Control', 'Keep the ship in the fight. Damaged systems degrade everyone\'s performance — act early.'],
    ['Hull Status', 'Your ship health gauge, it doesn\'t show where the damage is, the other bridge officers should ask for repairs.'],
    ['Repair Teams', 'Dispatch teams to damaged consoles, sooner rather than later.'],
  ],
  power: [
    ['Power Officer', 'You decide how much performance each system gets. Shift allocations as the battle changes.'],
    ['Power Allocation', 'Distribute 6 base points across systems. Higher level means better performance from that station.'],
    ['Battery Reserve', 'Holds up to 2 emergency power points. Let it drain completely and everyone gets locked out for a time.'],
  ],
  shields: [
    ['Shield Officer', 'Absorb incoming fire and keep the ship alive.'],
    ['Shield Facings', 'Four quadrants: Fore, Aft, Port, Starboard.'],
    ['Focus', 'Concentrate capacity on one facing to tank heavy fire by tapping it. Tap again to rebalance shields'],
  ],
  sensors: [
    ['Sensors Officer', 'Extend the crew\'s awareness beyond visual range.'],
    ['Long-Range Scan', 'Detect contacts before they enter combat range and provides some extra information on their status'],
  ],
  navigation: [
    ['Navigator', 'Read the map to keep the ship heading in the right direction.'],
    ['System Chart', 'Overlay the nav chart on the main screen so the Captain and Helm see the strategic picture.'],
    ['Cancel Impulse', 'Abort an active impulse charge if Helm is about to overshoot or fly into a hazard.'],
  ],
  comms: [
    ['Comms Officer', 'Connect the ship to the outside world. You are the first to know about orders, threats, and opportunities.'],
    ['Contacts', 'Track who is in hailing range.'],
    ['Messages', 'Incoming transmissions can carry mission-critical intelligence.'],
    ['Objectives', 'Mission goals update as the situation changes. Alert the Captain when new orders arrive.'],
  ],
  engineering: [
    ['Engineering Officer', 'Keep the ship running under fire. You manage shields, power, and repairs from a single station.'],
    ['Shields', 'Four facings: Fore, Aft, Port, Starboard. Focus one to tank heavy fire, tap again to rebalance.'],
    ['Power Allocation', 'Distribute base points across systems. Higher level means better performance. Keep an eye on the battery reserve.'],
    ['Repair Teams', 'Dispatch teams to damaged systems. Damaged systems degrade everyone\'s performance — act early.'],
  ],
  science: [
    ['Science Officer', 'Extend the crew\'s awareness and keep the shields up. You monitor long-range sensors and manage defensive coverage.'],
    ['Long-Range Scan', 'Detect contacts before they enter combat range and provide extra information on their status.'],
    ['Shield Facings', 'Four quadrants: Fore, Aft, Port, Starboard. Focus capacity on one facing to tank heavy fire.'],
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
    const label = (typeof window !== 'undefined' && window.CONSOLE_LABEL && window.CONSOLE_LABEL[consoleName]) || consoleName;
    heading.textContent = label;
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
