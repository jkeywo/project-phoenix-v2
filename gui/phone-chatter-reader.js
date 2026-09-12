/**
 * gui/phone-chatter-reader.js — the phone Viewscreen's level-3 Coordination
 * reading surface (issue #1429, PRD #1418 stories 20-21).
 *
 * Level-3 AI-to-AI Coordination bubbles in `server.html`'s `#chatter-container`
 * stay compact at any text scale on a phone-shaped Viewscreen (see
 * `gui/phone-viewscreen.js` and the `.chatter-bubble` phone media query in
 * server.html) so background chatter cannot fill a small screen. Tapping one
 * opens the SAME message's full text here, at the operator's chosen text
 * scale, in a dismissible reading surface.
 *
 * `open(content)` takes an already-resolved content SNAPSHOT — the shape
 * `normalizeCoordinationPresentation` (`gui/coordination-popup.js`) returns —
 * never a live binding to the bubble or the chatter stream. That is what
 * "survives arriving messages" means in practice: nothing in this module
 * re-renders the open surface when a new bubble is built, because nothing
 * wires the two together. A second `open()` call — a NEW tap on a NEW bubble
 * — replaces the pinned content deliberately; that is a fresh reading choice,
 * not the stream reaching in.
 *
 * The reading surface itself carries no motion, routing, admission or
 * simulation authority: it reads the same producer-owned envelope the compact
 * bubble already rendered, and closing it changes nothing about the message,
 * its recipients or its priority.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

import { t } from './strings.js';
import { createFocusTrap } from './focus-trap.js';

/** `data-control`/element ids, exported so a test and the CSS name the same
 *  nodes rather than three copies of the same string. */
export const CHATTER_READER_IDS = Object.freeze({
  overlay: 'chatter-reader',
  panel: 'chatter-reader-panel',
  heading: 'chatter-reader-heading',
  close: 'chatter-reader-close',
  sender: 'chatter-reader-sender',
  title: 'chatter-reader-title',
  body: 'chatter-reader-body',
});

/**
 * Find-or-create the reading surface on `doc` and wire its dismiss contract:
 * the shared modal focus trap (Tab/Shift+Tab cycle inside it, Escape closes
 * it, focus returns to whatever had focus before it opened — normally the
 * tapped bubble), plus a close button and a backdrop click.
 *
 * Idempotent: a second mount on the same document adopts the existing nodes
 * rather than duplicating them, the same rule `mountOverlayShell` follows.
 * Safe to call unconditionally — mounting costs nothing on a full display,
 * where nothing calls `open()`.
 *
 * @param {Document} doc
 * @returns {{
 *   open: (content: {sender?: string, from?: string, title?: string, body?: string}) => void,
 *   close: () => void,
 *   isOpen: () => boolean,
 *   overlay: Element,
 * }}
 */
export function mountPhoneChatterReader(doc) {
  const ids = CHATTER_READER_IDS;
  let overlay = doc.getElementById(ids.overlay);
  let sender;
  let title;
  let body;
  let closeBtn;

  if (!overlay) {
    overlay = doc.createElement('div');
    overlay.id = ids.overlay;
    overlay.className = 'chatter-reader-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-labelledby', ids.heading);

    const panel = doc.createElement('div');
    panel.id = ids.panel;
    panel.className = 'chatter-reader-panel';

    const header = doc.createElement('div');
    header.className = 'chatter-reader-header';

    const heading = doc.createElement('span');
    heading.id = ids.heading;
    heading.className = 'chatter-reader-heading';
    heading.textContent = t('server.chatter_reader.heading');
    header.appendChild(heading);

    closeBtn = doc.createElement('button');
    closeBtn.type = 'button';
    closeBtn.id = ids.close;
    closeBtn.className = 'chatter-reader-close';
    closeBtn.setAttribute('data-control', ids.close);
    closeBtn.setAttribute('aria-label', t('server.chatter_reader.close'));
    closeBtn.textContent = '×';
    header.appendChild(closeBtn);
    panel.appendChild(header);

    sender = doc.createElement('div');
    sender.id = ids.sender;
    sender.className = 'chatter-reader-sender';
    panel.appendChild(sender);

    title = doc.createElement('div');
    title.id = ids.title;
    title.className = 'chatter-reader-title';
    panel.appendChild(title);

    body = doc.createElement('div');
    body.id = ids.body;
    body.className = 'chatter-reader-body';
    panel.appendChild(body);

    overlay.appendChild(panel);
    overlay.hidden = true;
    overlay.setAttribute('aria-hidden', 'true');
    doc.body.appendChild(overlay);
  } else {
    sender = doc.getElementById(ids.sender);
    title = doc.getElementById(ids.title);
    body = doc.getElementById(ids.body);
    closeBtn = doc.getElementById(ids.close);
  }

  const focusTrap = createFocusTrap(overlay, { doc, onEscape: () => close() });

  function isOpen() {
    return overlay.hidden === false;
  }

  function open(content) {
    const c = content || {};
    if (sender) sender.textContent = c.sender || c.from || '';
    if (title) title.textContent = c.title || '';
    if (body) body.textContent = c.body || '';
    overlay.hidden = false;
    overlay.setAttribute('aria-hidden', 'false');
    overlay.classList.add('open');
    focusTrap.activate();
  }

  function close() {
    if (!isOpen()) return;
    focusTrap.release();
    overlay.hidden = true;
    overlay.setAttribute('aria-hidden', 'true');
    overlay.classList.remove('open');
  }

  if (closeBtn) {
    closeBtn.addEventListener('click', (e) => {
      if (e && typeof e.preventDefault === 'function') e.preventDefault();
      close();
    });
  }
  overlay.addEventListener('click', (e) => {
    if (e && e.target === overlay) close();
  });

  return { open, close, isOpen, overlay };
}

// Expose for server.html's classic (non-module) chatter script, matching the
// window.* convention gui/focus-trap.js and gui/coordination-popup.js follow.
if (typeof window !== 'undefined') {
  window.mountPhoneChatterReader = mountPhoneChatterReader;
}
