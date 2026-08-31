/** Parent/iframe routing helpers for a composite Station's semantic families. */

/** Read the active semantic subcontext exposed by an iframe, or its Station. */
export function semanticContextForIframe(iframe, fallback) {
  try {
    const read = iframe && iframe.contentWindow && iframe.contentWindow.__semanticActionContext;
    const value = typeof read === 'function' ? read() : null;
    return typeof value === 'string' && value ? value : fallback;
  } catch (_) {
    return fallback;
  }
}

/**
 * Keep the parent-owned gamepad catalogue honest for the active document.
 *
 * The parent registry is the union of every shipped Station family so settings
 * can remap a portable profile. A context alone is not enough to decide live
 * availability: Courier Captain installs Power/Repair adapters in its Captain
 * realm, while an ordinary Captain realm does not. The iframe therefore owns
 * the final action-id capability check. Missing/not-yet-loaded seams return
 * `null`, rather than an empty ready catalogue: the gamepad owner uses that
 * distinction to keep its neutral gate closed until the iframe can name every
 * live binding.
 */
export function semanticActionsForIframe(actions, iframe, context) {
  const candidates = Array.isArray(actions) ? actions : [];
  try {
    const supports = iframe && iframe.contentWindow
      && iframe.contentWindow.__supportsSemanticAction;
    if (typeof supports !== 'function') return null;
    return candidates.filter((action) => (
      action && typeof action.id === 'string' && supports(action.id, context) === true
    ));
  } catch (_) {
    return null;
  }
}

/**
 * Route a family-context activation to the active composite iframe when that
 * iframe declares ownership. Otherwise use the ordinary one-context/one-iframe
 * lookup. This applies to both live values and continuous neutral delivery.
 */
export function iframeForSemanticContext(context, activeIframe, lookup) {
  try {
    const supports = activeIframe && activeIframe.contentWindow
      && activeIframe.contentWindow.__supportsSemanticActionContext;
    if (typeof supports === 'function' && supports(context) === true) return activeIframe;
  } catch (_) { /* fall through to the ordinary Station iframe */ }
  return typeof lookup === 'function' ? lookup(context) : null;
}

if (typeof window !== 'undefined') {
  window.CompositeActionRouting = Object.freeze({
    semanticContextForIframe,
    semanticActionsForIframe,
    iframeForSemanticContext,
  });
}
