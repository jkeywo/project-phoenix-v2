/**
 * gui/station-damage-popup.js — the Station Bar's per-system damage popup
 * (issue #1374).
 *
 * Every console used to carry a `ph-station-damage` bar in its footer: a
 * summed hull strip you tapped to see that Station's individual systems. The
 * footers are gone, so the popup moved onto the bar — tapping the ALREADY
 * SELECTED Station tab opens it. One surface for the whole fleet instead of
 * twenty-two copies of the same widget, and the seat spends none of its
 * screen on a readout it looks at once a minute.
 *
 * The rows are not new state: `gui/console-core.js` posts the active console's
 * own `own_hull.entries` to the shell as `console_hull` (issue #1373) and the
 * shell holds the last set per console. This module owns what the popup SAYS
 * and the layer behaviour around it; `ph-damage-detail` draws the rows and
 * `gui/focus-trap.js` is the modal contract (issue #1174) — neither is
 * reimplemented here.
 *
 * The shell hands in a Station DISPLAY NAME and the rows; it never hands in
 * markup. That is what keeps the model below testable with no shell around it.
 */

import { createFocusTrap } from './focus-trap.js';
import { t } from './strings.js';

/** One trap per popup element, built on first open and reused after that. */
const traps = new WeakMap();

/**
 * What the popup shows for one Station.
 *
 * The title reuses the string the retired footer widget already had for the
 * same heading (`component.station_damage.popup_title`) rather than minting a
 * second one that would drift from it.
 *
 * @param {{station?: string, entries?: Array}} [input]
 *   `station` is the Station's display name, already resolved by the caller —
 *   the shell resolves `station.<id>.name` the same way it does for the bar's
 *   own tab labels. `entries` are the `own_hull` rows the console posted.
 * @returns {{title: string, entries: Array, empty: boolean}}
 */
export function stationDamagePopupModel(input) {
  const source = input || {};
  const entries = Array.isArray(source.entries) ? source.entries.filter(Boolean) : [];
  return {
    title: t('component.station_damage.popup_title', { name: source.station || '' }),
    entries,
    // A Station with no damageable Systems is an ordinary state, not an error —
    // a Captain's chair owns no hull of its own — so the popup says so rather
    // than opening onto an empty box.
    empty: entries.length === 0,
  };
}

/**
 * Paint the popup's elements from that model, saying nothing about whether it
 * is showing. That separation is what lets a fresh `console_hull` post repaint
 * an OPEN popup in place — a system dropping under fire is exactly the change
 * the player opened it to watch.
 *
 * Every element is looked up by id and null-guarded: the popup lives in
 * client.html, so a caller that has not mounted it gets `null` rather than a
 * throw.
 *
 * @param {Document} doc
 * @param {{station?: string, entries?: Array}} state
 * @returns {{title: string, entries: Array, empty: boolean}|null}
 */
export function renderStationDamagePopup(doc, state) {
  if (!doc || typeof doc.getElementById !== 'function') return null;
  const titleEl = doc.getElementById('station-damage-popup-title');
  const detailEl = doc.getElementById('station-damage-popup-detail');
  const emptyEl = doc.getElementById('station-damage-popup-empty');
  if (!titleEl || !detailEl || !emptyEl) return null;

  const model = stationDamagePopupModel(state);
  titleEl.textContent = model.title;
  detailEl.state = { entries: model.entries };
  detailEl.hidden = model.empty;
  emptyEl.hidden = !model.empty;
  return model;
}

/** The popup's root, or null where it is not mounted. */
function popupRoot(doc) {
  return (doc && typeof doc.getElementById === 'function')
    ? doc.getElementById('station-damage-popup')
    : null;
}

/** Is the popup currently showing? */
export function isStationDamagePopupOpen(doc) {
  const root = popupRoot(doc);
  return !!root && !root.hidden;
}

/**
 * The trap for this popup, built once.
 *
 * The dialog semantics are set here rather than in the markup on purpose: a
 * surface that declares `role="dialog"` owes the focus contract
 * (tests/client/modal-contract.test.js), and declaring it beside the
 * `createFocusTrap` call is what keeps the two from drifting apart.
 */
function trapFor(doc, root) {
  let trap = traps.get(root);
  if (trap) return trap;
  const card = typeof root.querySelector === 'function'
    ? root.querySelector('.popup-card') : null;
  if (card && typeof card.setAttribute === 'function') {
    card.setAttribute('role', 'dialog');
    card.setAttribute('aria-modal', 'true');
  }
  trap = createFocusTrap(root, {
    doc,
    onEscape: () => closeStationDamagePopup(doc),
    initialFocus: '#station-damage-popup-close',
  });
  traps.set(root, trap);
  // The scrim dismisses; the card does not, so a tap that lands on a system
  // row is not also a dismiss. Bound once with the trap, for the same reason.
  if (typeof root.addEventListener === 'function') {
    root.addEventListener('click', (event) => {
      if (event && event.target === root) closeStationDamagePopup(doc);
    });
  }
  const close = doc.getElementById('station-damage-popup-close');
  if (close && typeof close.addEventListener === 'function') {
    close.addEventListener('click', () => closeStationDamagePopup(doc));
  }
  return trap;
}

/**
 * Show the popup for one Station: paint it, reveal it, and take focus under
 * the shared modal contract. A second call while it is already open repaints
 * without disturbing focus or re-remembering the opener.
 *
 * @param {Document} doc
 * @param {{station?: string, entries?: Array}} state
 * @returns {{title: string, entries: Array, empty: boolean}|null}
 */
export function openStationDamagePopup(doc, state) {
  const root = popupRoot(doc);
  if (!root) return null;
  const model = renderStationDamagePopup(doc, state);
  if (!model) return null;
  if (!root.hidden) return model;
  root.hidden = false;
  trapFor(doc, root).activate();
  return model;
}

/**
 * Hide it again and hand focus back to whatever opened it — the bar tab, for
 * the tap that opened it, so a keyboard operator resumes on the bar rather
 * than at the top of the document. Idempotent.
 *
 * @param {Document} doc
 */
export function closeStationDamagePopup(doc) {
  const root = popupRoot(doc);
  if (!root || root.hidden) return;
  root.hidden = true;
  const trap = traps.get(root);
  if (trap) trap.release();
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.stationDamagePopupModel = stationDamagePopupModel;
  window.renderStationDamagePopup = renderStationDamagePopup;
  window.openStationDamagePopup = openStationDamagePopup;
  window.closeStationDamagePopup = closeStationDamagePopup;
  window.isStationDamagePopupOpen = isStationDamagePopupOpen;
}
