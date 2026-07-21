/**
 * gui/key-relay.js — forward host-page key events into the active console iframe.
 *
 * Console UIs run in iframes, and key events are delivered only to the document
 * that holds focus; they do not cross the frame boundary. So a console's
 * keybinds (helm's WASD/arrows, Ctrl impulse, Shift boost) worked only while
 * focus happened to sit inside that iframe — clicking the tab bar, the
 * viewscreen, or any host chrome silently killed them until the operator
 * clicked back into the console. This relays the host's key events to the
 * active console so the bindings work no matter what was last clicked.
 *
 * This is the keyboard relay injected into console iframes (the swipe relay was deleted in #827), which
 * exists because touch events inside an iframe don't reach the host either.
 *
 * There is no double-delivery risk: when focus IS inside the iframe the host
 * never sees the event, so nothing is relayed and the console handles it
 * natively.
 */

/** Key event types worth relaying. Excludes `keypress` (deprecated). */
const RELAYED_TYPES = ['keydown', 'keyup'];

/** True when `el` is a field the operator could be typing into. */
function isTypingTarget(el) {
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable === true;
}

/**
 * Relay `keydown`/`keyup` seen by `hostDoc` into the document returned by
 * `getTargetDoc` (the active console iframe's document, or null when no
 * console is active).
 *
 * Skips relaying while the operator is typing into a host-page field, so host
 * chrome keeps first claim on the keyboard.
 *
 * @param {Document} hostDoc
 * @param {() => (Document|null|undefined)} getTargetDoc
 * @returns {() => void} uninstall function
 */
export function installKeyRelay(hostDoc, getTargetDoc) {
  if (!hostDoc || typeof getTargetDoc !== 'function') return () => {};

  const onKey = (e) => {
    // A relayed event should never be relayed again.
    if (e.__relayedKey) return;
    if (isTypingTarget(e.target) || isTypingTarget(hostDoc.activeElement)) return;

    let doc = null;
    try { doc = getTargetDoc(); } catch (_) { return; }
    // Guard `doc === hostDoc`: relaying a document to itself would recurse.
    if (!doc || doc === hostDoc) return;

    const view = doc.defaultView;
    if (!view || typeof view.KeyboardEvent !== 'function') return;

    // Build the copy with the *target frame's* KeyboardEvent constructor so it
    // is a genuine KeyboardEvent inside that realm.
    const copy = new view.KeyboardEvent(e.type, {
      code: e.code,
      key: e.key,
      location: e.location,
      repeat: e.repeat,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      metaKey: e.metaKey,
      bubbles: true,
      cancelable: true,
    });
    copy.__relayedKey = true;

    const notCancelled = doc.dispatchEvent(copy);
    // Mirror the console's preventDefault back onto the host event, so keys the
    // console claims (arrows, Ctrl) don't also scroll or trigger host chrome.
    if (!notCancelled && e.cancelable) e.preventDefault();
  };

  for (const type of RELAYED_TYPES) hostDoc.addEventListener(type, onKey);
  return () => {
    for (const type of RELAYED_TYPES) hostDoc.removeEventListener(type, onKey);
  };
}

// Expose for the non-module inline script in client.html, matching the
// window.iframeBridgePush convention.
if (typeof window !== 'undefined') {
  window.installKeyRelay = installKeyRelay;
}
