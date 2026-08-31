/** Parent/iframe routing helpers for a composite Station's semantic families. */

const SUBCONTEXT_ATTRIBUTE = 'data-semantic-action-context';

function normalizedContext(value) {
  return typeof value === 'string' ? value.trim().toLowerCase() : '';
}

/**
 * Keep a composite document's keyboard/gamepad family explicit.
 *
 * Each interactive region names its family with
 * `data-semantic-action-context`. Pointer interaction or keyboard focus moves
 * the active family; until either happens the authored default remains active.
 * The controller deliberately remembers the last selected region when focus
 * moves to shared chrome, so an unrelated footer control cannot silently
 * redirect input.
 */
export function createSemanticSubcontextController(root, options = {}) {
  if (!root || typeof root.querySelectorAll !== 'function'
      || typeof root.addEventListener !== 'function') {
    throw new TypeError('semantic subcontext controller requires an event root');
  }
  const defaultContext = normalizedContext(options.defaultContext);
  if (!defaultContext) throw new TypeError('semantic subcontext default is required');

  const selector = `[${SUBCONTEXT_ATTRIBUTE}]`;
  const regions = Array.from(root.querySelectorAll(selector));
  const contexts = new Set(regions
    .map((region) => normalizedContext(region.getAttribute(SUBCONTEXT_ATTRIBUTE)))
    .filter(Boolean));
  contexts.add(defaultContext);
  let activeContext = defaultContext;

  function select(context) {
    const next = normalizedContext(context);
    if (!contexts.has(next)) return false;
    activeContext = next;
    for (const region of regions) {
      const selected = normalizedContext(region.getAttribute(SUBCONTEXT_ATTRIBUTE)) === next;
      region.toggleAttribute('data-semantic-action-active', selected);
    }
    return true;
  }

  function regionFromEvent(event) {
    const target = event && event.target;
    if (!target || typeof target.closest !== 'function') return null;
    const region = target.closest(selector);
    if (!region || !regions.includes(region)) return null;
    return region;
  }

  function onInteraction(event) {
    const region = regionFromEvent(event);
    if (region) select(region.getAttribute(SUBCONTEXT_ATTRIBUTE));
  }

  root.addEventListener('focusin', onInteraction);
  root.addEventListener('pointerdown', onInteraction);
  select(defaultContext);

  return Object.freeze({
    getContext: () => activeContext,
    select,
    dispose() {
      root.removeEventListener('focusin', onInteraction);
      root.removeEventListener('pointerdown', onInteraction);
    },
  });
}

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
    createSemanticSubcontextController,
    semanticContextForIframe,
    semanticActionsForIframe,
    iframeForSemanticContext,
  });
}
