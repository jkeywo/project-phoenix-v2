/**
 * gui/focus-trap.js — the one modal focus contract, shared (issue #1174).
 *
 * A modal is a layer the crew must finish with before the ship behind it is
 * reachable again, and that promise is three separate behaviours a screen
 * reader and a keyboard both depend on:
 *
 *   1. TRAP. While the modal is open, Tab and Shift+Tab cycle only through the
 *      modal's own controls — Tab off the last wraps to the first, Shift+Tab
 *      off the first wraps to the last — so focus can never walk out onto the
 *      page underneath.
 *   2. ESCAPE. Escape closes the layer, so the keyboard is never stranded
 *      inside a modal it cannot dismiss.
 *   3. RESTORE. Opening moves focus INTO the modal; closing puts it back on the
 *      control that opened it, so a keyboard operator resumes exactly where they
 *      left off rather than at the top of the document.
 *
 * The reason this is one helper and not a handler copied into each modal is the
 * same reason roving-tabindex (issue #1170) is one helper: the settings cog on
 * the phone and the settings cog on the host page are the same contract, and a
 * future confirmation dialog is the same contract again. Each modal supplies
 * only its element and its `close` — everything else is here.
 *
 * The background is made `inert` (and `aria-hidden`) for the duration, which is
 * what actually stops a pointer or an assistive cursor reaching the page behind
 * the modal; the Tab wrap above is the keyboard half of the same fence. Both
 * are lifted again on release, restoring whatever the background declared before
 * (the host page's debug dock, say, manages its own `aria-hidden`, so its prior
 * value is remembered and put back rather than blindly cleared).
 *
 * Pure DOM, no framework: the modals are plain overlays built in JS, and this
 * binds straight to them. It is deliberately defensive about missing methods so
 * the same code runs under a browser, under jsdom, and under the hand-rolled
 * DOM stub settings-panel.test.js drives it with.
 */

/**
 * The selector for a control that Tab would land on. `[tabindex]` is filtered
 * below so a roving group's `tabindex="-1"` members are excluded — a composite
 * that is one Tab stop (issue #1170) must not become many again inside a modal.
 */
const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]',
].join(',');

/** Whether `el` is a control focus may actually rest on right now. */
function isFocusable(el) {
  if (!el) return false;
  if (el.disabled) return false;
  if (typeof el.hasAttribute === 'function' && el.hasAttribute('hidden')) return false;
  if (typeof el.getAttribute === 'function') {
    if (el.getAttribute('aria-hidden') === 'true') return false;
    const ti = el.getAttribute('tabindex');
    if (ti !== null && ti !== undefined && Number(ti) < 0) return false;
  }
  // Anything inside an inert subtree cannot be focused, so a control the
  // background-inert put to sleep is not a tab stop either.
  if (typeof el.closest === 'function' && el.closest('[inert]')) return false;
  return true;
}

/**
 * The modal's focusable controls, in document order.
 *
 * @param {Element} root
 * @returns {Element[]}
 */
export function focusableWithin(root) {
  if (!root || typeof root.querySelectorAll !== 'function') return [];
  return Array.from(root.querySelectorAll(FOCUSABLE_SELECTOR)).filter(isFocusable);
}

/**
 * The element focus really rests on, descending through shadow roots so a
 * control inside a web component resolves to the control, not its host.
 *
 * @param {Document} doc
 * @returns {Element|null}
 */
export function activeElementOf(doc) {
  if (!doc) return null;
  let el = doc.activeElement || null;
  while (el && el.shadowRoot && el.shadowRoot.activeElement) {
    el = el.shadowRoot.activeElement;
  }
  return el;
}

function callFocus(el) {
  if (el && typeof el.focus === 'function') {
    try { el.focus(); } catch (_e) { /* some stubs throw; a failed focus is harmless */ }
  }
}

/**
 * Wire the modal focus contract to a modal element.
 *
 * The trap is created ONCE per modal (the overlay persists, `hidden` toggles),
 * and the modal's own `open`/`close` drive it: `activate()` on open, `release()`
 * on close. `release()` is idempotent, so a modal that closes by Escape (which
 * routes through `onEscape` → the modal's `close` → `release`) and one that
 * closes by backdrop click both restore focus exactly once.
 *
 * @param {Element} modal   the dialog element (the overlay that carries
 *   role="dialog"); its focusable descendants are the trap's ring
 * @param {{
 *   doc?: Document,
 *   onEscape?: () => void,
 *   initialFocus?: Element|string,
 * }} [options]
 *   `onEscape` is what Escape invokes — the modal's own `close`, never a second
 *   teardown path. `initialFocus` overrides the default first-control focus (an
 *   element or a selector resolved within the modal).
 * @returns {{ activate: () => void, release: () => void, isActive: () => boolean }}
 */
export function createFocusTrap(modal, options = {}) {
  const opts = options || {};
  const doc = opts.doc
    || (modal && modal.ownerDocument)
    || (typeof document !== 'undefined' ? document : null);
  const onEscape = typeof opts.onEscape === 'function' ? opts.onEscape : null;

  let active = false;
  let opener = null;
  // What the background looked like before the trap put it to sleep, so release
  // restores exactly that rather than assuming it was clear.
  const suspended = [];

  /**
   * The page behind the modal: the modal's siblings, minus the OPENER. The
   * opener (the cog that toggles the panel) stays live so a second click on it
   * still closes the panel — it is the one background control the operator must
   * keep reaching.
   */
  function backgroundRoots() {
    const parent = modal && modal.parentNode;
    if (!parent || !parent.children) return [];
    return Array.from(parent.children).filter((el) => el !== modal && el !== opener);
  }

  function suspendBackground() {
    for (const el of backgroundRoots()) {
      const hadInert = typeof el.hasAttribute === 'function' ? el.hasAttribute('inert') : false;
      const prevHidden = typeof el.getAttribute === 'function' ? el.getAttribute('aria-hidden') : null;
      suspended.push({ el, hadInert, prevHidden });
      if (typeof el.setAttribute === 'function') {
        el.setAttribute('inert', '');
        el.setAttribute('aria-hidden', 'true');
      }
    }
  }

  function restoreBackground() {
    for (const rec of suspended) {
      const { el, hadInert, prevHidden } = rec;
      if (!hadInert && typeof el.removeAttribute === 'function') el.removeAttribute('inert');
      if (prevHidden === null || prevHidden === undefined) {
        if (typeof el.removeAttribute === 'function') el.removeAttribute('aria-hidden');
      } else if (typeof el.setAttribute === 'function') {
        el.setAttribute('aria-hidden', prevHidden);
      }
    }
    suspended.length = 0;
  }

  function moveFocusIn() {
    const items = focusableWithin(modal);
    let target = null;
    if (opts.initialFocus) {
      target = typeof opts.initialFocus === 'string'
        ? (modal && typeof modal.querySelector === 'function' ? modal.querySelector(opts.initialFocus) : null)
        : opts.initialFocus;
    }
    if (!target) target = items.length ? items[0] : modal;
    callFocus(target);
  }

  function onKeyDown(event) {
    if (!active || !event) return;
    if (event.key === 'Escape' || event.key === 'Esc') {
      if (typeof event.preventDefault === 'function') event.preventDefault();
      if (onEscape) onEscape();
      return;
    }
    if (event.key !== 'Tab') return;
    const items = focusableWithin(modal);
    if (items.length === 0) {
      // Nothing to land on: keep Tab from walking onto the page behind.
      if (typeof event.preventDefault === 'function') event.preventDefault();
      callFocus(modal);
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const current = activeElementOf(doc);
    const inModal = !!(current && modal && typeof modal.contains === 'function' && modal.contains(current));
    if (event.shiftKey) {
      // Shift+Tab off the first control (or from anywhere focus has escaped to)
      // lands on the last.
      if (!inModal || current === first) {
        if (typeof event.preventDefault === 'function') event.preventDefault();
        callFocus(last);
      }
    } else if (!inModal || current === last) {
      // Tab off the last control wraps back to the first.
      if (typeof event.preventDefault === 'function') event.preventDefault();
      callFocus(first);
    }
  }

  function activate() {
    if (active) return;
    active = true;
    // Remember the trigger BEFORE the background goes inert (which would blur
    // it), so close can hand focus back to exactly where it came from.
    opener = activeElementOf(doc);
    suspendBackground();
    // Listen on the document, not the modal: Tab pressed while focus has
    // slipped onto the page behind never bubbles up through the modal, so a
    // modal-only listener could not pull it back. The background is inert, so in
    // a real browser focus cannot escape in the first place — this is the belt
    // to inert's braces. Removed again on release, so nothing global lingers.
    if (doc && typeof doc.addEventListener === 'function') {
      doc.addEventListener('keydown', onKeyDown);
    }
    moveFocusIn();
  }

  function release() {
    if (!active) return;
    active = false;
    if (doc && typeof doc.removeEventListener === 'function') {
      doc.removeEventListener('keydown', onKeyDown);
    }
    restoreBackground();
    const back = opener;
    opener = null;
    callFocus(back);
  }

  return { activate, release, isActive: () => active };
}

// Expose for any non-module inline script, matching the window.* convention the
// other shared gui modules (roving-tabindex.js, hero-bar.js) follow.
if (typeof window !== 'undefined') {
  window.createFocusTrap = createFocusTrap;
  window.focusableWithin = focusableWithin;
}
