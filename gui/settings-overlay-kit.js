/**
 * gui/settings-overlay-kit.js — the DOM shell shared by the two settings
 * cogs (issue #1238, de-duping #939's host cog and #940's phone cog).
 *
 * `gui/server-settings.js` (host) and `gui/settings-panel.js` (client) each
 * mount a gear button + a tabbed modal overlay, and the surrounding chrome —
 * find-or-create the button and overlay, wire the shared focus trap, open and
 * close, the backdrop-click-to-dismiss, the tab-bar row, and the section/hint/
 * row DOM primitives — was byte-for-byte the same shape twice, differing only
 * in element ids and CSS class names. That shell lives here now; both mount
 * functions call it instead of rebuilding it.
 *
 * What is DELIBERATELY NOT here: the two pages' tab BODIES. The host paints
 * its Debug/Cheat and Audio tabs synchronously from direct WASM binding calls
 * and repaints them from a `requestAnimationFrame` poll; the client fires a
 * `ClientMessage` and repaints only when the host's next state push says the
 * flag actually moved. That push-vs-poll split is a real behavioural
 * difference, not incidental duplication, so `buildDebugTab`, `buildAudioTab`
 * and friends stay in their own files. Collapsing them into one descriptor
 * would fork that behaviour rather than share it.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

import { t } from './strings.js';
import { createFocusTrap } from './focus-trap.js';

/** The slider's own resolution, not a tunable: master volume is a 0..1 scale
 *  factor and both pages offer it in whole percent. */
export const VOLUME_MIN = 0;
export const VOLUME_MAX = 1;
export const VOLUME_STEP = 0.01;

/**
 * Mount (or re-adopt) the gear button and its modal overlay, and wire the
 * shared focus-trap contract (issue #1174): Tab/Shift+Tab cycle only inside
 * the panel, Escape closes it, focus returns to the cog on close, backdrop
 * click closes it, and clicking the cog toggles it.
 *
 * The caller supplies `shell.buildContent` (a hoisted function declaration
 * reference is fine — this only needs to exist by the time a click actually
 * opens the panel) to rebuild the tab bar and body from current state; this
 * module owns only the reveal/hide mechanics around that rebuild.
 *
 * @param {Document} doc
 * @param {{
 *   buttonId: string, overlayId: string,
 *   buttonClass: string, overlayClass: string,
 *   titleId?: string, glyph?: string,
 *   stopPropagationOnToggle?: boolean,
 * }} opts
 * @returns {{
 *   btn: Element, overlay: Element, focusTrap: object,
 *   buildContent: (function|null),
 *   isOpen: () => boolean, open: () => void, close: () => void,
 * }}
 */
export function mountOverlayShell(doc, {
  buttonId,
  overlayId,
  buttonClass,
  overlayClass,
  titleId = 'settings.title',
  glyph = '⚙',
  stopPropagationOnToggle = false,
} = {}) {
  let btn = doc.getElementById(buttonId);
  if (!btn) {
    btn = doc.createElement('button');
    btn.id = buttonId;
    btn.className = buttonClass;
    btn.type = 'button';
    doc.body.appendChild(btn);
  }
  btn.textContent = glyph;
  btn.title = t(titleId);
  btn.setAttribute('aria-label', t(titleId));
  btn.setAttribute('aria-expanded', 'false');

  let overlay = doc.getElementById(overlayId);
  if (!overlay) {
    overlay = doc.createElement('div');
    overlay.id = overlayId;
    overlay.className = overlayClass;
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    doc.body.appendChild(overlay);
  }
  overlay.hidden = true;
  overlay.setAttribute('aria-hidden', 'true');

  // `shell.buildContent` is set by the caller after this returns (a hoisted
  // function declaration, so the order of the two statements at the call
  // site doesn't matter) — `open()` below reads it through the object so it
  // always sees the latest value rather than closing over `null`.
  const shell = { btn, overlay, buildContent: null };

  shell.focusTrap = createFocusTrap(overlay, { doc, onEscape: () => shell.close() });

  shell.isOpen = () => overlay.hidden === false;

  shell.open = () => {
    // Rebuilt on every open so a build-flag gate or read-back state is
    // re-evaluated against the latest truth rather than a stale render.
    if (typeof shell.buildContent === 'function') shell.buildContent();
    overlay.hidden = false;
    overlay.setAttribute('aria-hidden', 'false');
    overlay.classList.add('open');
    btn.setAttribute('aria-expanded', 'true');
    shell.focusTrap.activate();
  };

  shell.close = () => {
    // Lift the trap first so it hands focus back to the cog before the panel
    // is hidden out from under it.
    shell.focusTrap.release();
    overlay.hidden = true;
    overlay.setAttribute('aria-hidden', 'true');
    overlay.classList.remove('open');
    btn.setAttribute('aria-expanded', 'false');
  };

  btn.addEventListener('click', (e) => {
    if (e && typeof e.preventDefault === 'function') e.preventDefault();
    if (stopPropagationOnToggle && e && typeof e.stopPropagation === 'function') e.stopPropagation();
    if (shell.isOpen()) shell.close();
    else shell.open();
  });

  overlay.addEventListener('click', (e) => {
    if (e && e.target === overlay) shell.close();
  });

  return shell;
}

/**
 * Render the tab strip into `container` (already empty — both pages rebuild
 * their whole overlay from scratch on every repaint, so there is never a
 * stale button to clear first).
 *
 * @param {Document} doc
 * @param {Element} container
 * @param {Array<{id: string, labelId: string}>} tabs
 * @param {string|null} activeTabId
 * @param {string} tabClass — e.g. `'settings-tab'` or `'server-settings-tab'`.
 * @param {(id: string) => void} onSelect
 */
export function renderTabBar(doc, container, tabs, activeTabId, tabClass, onSelect) {
  for (const tab of tabs) {
    const el = doc.createElement('button');
    el.type = 'button';
    el.className = tabClass + (tab.id === activeTabId ? ' active' : '');
    el.setAttribute('data-tab', tab.id);
    el.textContent = t(tab.labelId);
    el.addEventListener('click', (e) => {
      if (e && typeof e.preventDefault === 'function') e.preventDefault();
      onSelect(tab.id);
    });
    container.appendChild(el);
  }
}

/**
 * The `section`/`hint` DOM primitives both panels build every tab body from.
 * Class names are supplied explicitly rather than derived from one prefix:
 * the two pages' existing class names are not a simple prefix substitution of
 * each other (the host's heading class is `server-settings-heading`, not
 * `server-settings-section-heading`), and this must not rename either page's
 * CSS hooks.
 *
 * @param {Document} doc
 * @param {{ sectionClass: string, headingClass: string, hintClass: string }} classes
 */
export function makeSectionBuilders(doc, { sectionClass, headingClass, hintClass }) {
  function section(labelId) {
    const el = doc.createElement('div');
    el.className = sectionClass;
    const heading = doc.createElement('div');
    heading.className = headingClass;
    heading.textContent = t(labelId);
    el.appendChild(heading);
    return el;
  }

  function hint(labelId) {
    const el = doc.createElement('div');
    el.className = hintClass;
    el.textContent = t(labelId);
    return el;
  }

  return { section, hint };
}

/**
 * A row `<div>` factory. The client picks a different class per row
 * (`settings-rating-row`, `settings-vol-row`, ...) so every call passes one
 * explicitly; the host always wants the same class, so it can call the
 * returned function with no argument and get `defaultClass`.
 *
 * @param {Document} doc
 * @param {string} [defaultClass]
 * @returns {(className?: string) => Element}
 */
export function makeRowBuilder(doc, defaultClass) {
  return function row(className) {
    const el = doc.createElement('div');
    el.className = className || defaultClass;
    return el;
  };
}
