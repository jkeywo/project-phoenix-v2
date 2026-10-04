/**
 * Parent-side Console lifetime: mounted Station identity, load/reload seeding,
 * snapshot publication and the declarations consumed by the Station Bar.
 * createConsoleMounts is the client shell's interface. The low-level DOM
 * helpers remain available to standalone mount/visibility consumers.
 */

import { iframeIdFor, planMounts } from './mount-plan.js';
import { consoleSections } from './content-switcher.js';
import { push, setOverlay } from './iframe-bridge.js';
import { alwaysPushConsoles, dirtyConsolesFor } from './dirty-consoles.js';
import { PUSH_CAUSE } from './console-latency.js';

/**
 * Mount one persistent section+iframe per plan entry into `container`.
 * Removes any previously-mounted `.console-section` first (leaving siblings
 * like #lobby-ui untouched). This is the sole creator of console iframe nodes.
 *
 * @param {Document} doc          Owning document (for createElement).
 * @param {Element}  container    The #console-container element.
 * @param {object|null} shipStations  Server-supplied ship_stations.
 * @param {(iframe: Element, mount: object) => void} [onIframe]
 *        Optional per-iframe hook for the controller or a standalone consumer.
 */
export function mountConsoles(doc, container, shipStations, onIframe) {
  if (!container) return;

  // Remove all previously-mounted console sections (identified by class),
  // but leave the lobby-ui sibling untouched.
  for (const el of container.querySelectorAll('.console-section')) el.remove();

  for (const mount of planMounts(shipStations)) {
    const section = doc.createElement('section');
    section.id = mount.sectionId;
    section.className = 'console-section';

    const iframe = doc.createElement('iframe');
    iframe.id = mount.iframeId;
    iframe.src = mount.url;
    iframe.title = mount.title;
    iframe.allowFullscreen = true;

    section.appendChild(iframe);
    container.appendChild(section);

    if (typeof onIframe === 'function') onIframe(iframe, mount);
  }
}

/**
 * Apply the active-console visibility to already-mounted sections: at most one
 * gets `.active`. This is a pure class toggle — it never creates, removes, or
 * re-parents any node, so iframe node identity and local state are preserved
 * across every switch.
 *
 * @param {Document} doc           Owning document (for getElementById).
 * @param {string|null} activeConsole  Lowercase station id, or null.
 * @param {boolean} inGame         Whether the game shell is active.
 * @param {string[]} stationIds    The ship's mounted station ids.
 * @returns {Record<string, boolean>} The applied visibility map.
 */
export function applyConsoleVisibility(doc, activeConsole, inGame, stationIds) {
  const sections = consoleSections(activeConsole, inGame, stationIds);
  for (const [sectionId, visible] of Object.entries(sections)) {
    const el = doc.getElementById(sectionId);
    if (el) el.className = visible ? 'console-section active' : 'console-section';
  }
  return sections;
}

/**
 * Which mounted Station sent this message — resolved by FRAME IDENTITY, not by
 * anything the sender said about itself (issue #1374).
 *
 * A console document knows its own name and nothing else. That name is not a
 * Station: `gui/cruiser/comms.html` is the console for the cruiser's `comms`
 * Station AND its `navigation` Station, so both mounted iframes run
 * `initConsole({ name: 'comms' })` and both call themselves 'comms' while
 * carrying different `own_hull` rows and a different chart. Keying the shell's
 * per-console stores on that claim collapses the two seats into one slot: the
 * Navigation seat's rows land under 'comms' and the Comms seat's popup lists
 * the wrong systems, while 'navigation' never gets a row at all.
 *
 * The shell has the answer already — `mountConsoles` above created each iframe
 * against a Station id — so it looks the sender up instead of trusting it.
 * `fallback` is used only when no mounted frame matches, which is every path
 * with no iframe behind it (the native host, BroadcastChannel).
 *
 * @param {Document} doc               Owning document (for getElementById).
 * @param {object|null} shipStations   Server-supplied ship_stations.
 * @param {Window|null} source         `event.source` of the postMessage.
 * @param {string} [fallback]          The name the sender claimed.
 * @returns {string} A Station id, the fallback, or '' when neither is known.
 */
export function stationIdForSource(doc, shipStations, source, fallback) {
  if (doc && source) {
    for (const st of (shipStations && shipStations.stations) || []) {
      const id = st && st.id;
      if (!id) continue;
      const iframe = doc.getElementById(iframeIdFor(id));
      let win = null;
      // A frame that has not created its window yet reads as null; a
      // cross-origin one hands back a proxy that is still safe to compare.
      try { win = iframe && iframe.contentWindow; } catch (_) { win = null; }
      if (win && win === source) return id;
    }
  }
  return (typeof fallback === 'string') ? fallback : '';
}

/**
 * Own the mounted Console documents for one client shell.
 *
 * Readers are live: reconnect, profile import and locale changes never leave a
 * load callback holding an old simulation/profile snapshot. Adapters perform
 * single presentation operations; this module owns their load ordering.
 * Declaration caches retain the last report until a console replaces it, as
 * before. A remount changes frame identity, not that cache policy.
 *
 * onDeclaration receives { type, stationId } after an accepted declaration;
 * the shell retains rendering and decides whether an open popup needs repaint.
 */
export function createConsoleMounts({
  doc,
  container,
  readState = () => null,
  buildState,
  readActiveStation = () => null,
  readBindings = () => null,
  readFeedbackPreferences = () => null,
  installLocale = () => {},
  applyAccessibility = () => {},
  noteSnapshot = () => {},
  afterBindings = () => {},
  onDeclaration = () => {},
}) {
  const mounted = new Map();
  const tabs = new Map();
  const hull = new Map();
  let shipStations = null;
  let selectedOverlay = { console: null, id: null };
  let generation = 0;
  let disposed = false;

  function frame(stationId) {
    return mounted.get(stationId)?.iframe || null;
  }

  function refresh(stationId, cause) {
    if (disposed || !stationId) return;
    const state = readState();
    const json = state && typeof buildState === 'function' ? buildState(stationId, state) : '{}';
    push(frame(stationId), stationId, json);
    // Preserve measurement after the attempted publication, even if its
    // document/hook is unavailable. Cause, not delivery, controls eligibility.
    noteSnapshot(stationId, cause);
  }

  function publishChanges(changes) {
    if (disposed) return;
    const state = readState();
    const stationSystems = state?.stationSystems;
    const families = state?.systemConsoleFamilies;
    const blackboardFamilies = state?.blackboardConsoleFamilies;
    const always = alwaysPushConsoles(stationSystems, families);
    for (const stationId of dirtyConsolesFor(changes, stationSystems, families, blackboardFamilies)) {
      if (always.has(stationId) || readActiveStation() === stationId) {
        refresh(stationId, PUSH_CAUSE.SERVER_MESSAGE);
      }
    }
  }

  function publishBindings(iframe) {
    const bindings = readBindings();
    if (!iframe || !bindings) return;
    try {
      const target = iframe.contentWindow;
      const update = target?.__updateSemanticActionBindings;
      if (typeof update === 'function') update(bindings);
      const updateFeedback = target?.__updateActionFeedbackPreferences;
      if (typeof updateFeedback === 'function') updateFeedback(readFeedbackPreferences());
    } catch (_) { /* unavailable documents are retried on load */ }
    afterBindings();
  }

  function refreshBindings() {
    if (disposed) return;
    for (const { iframe } of mounted.values()) publishBindings(iframe);
  }

  function selectOverlay(stationId, overlayId) {
    if (disposed) return;
    const previous = selectedOverlay;
    selectedOverlay = { console: overlayId ? stationId : null, id: overlayId || null };
    if (previous.console && previous.console !== selectedOverlay.console) {
      setOverlay(frame(previous.console), null);
    }
    if (stationId) setOverlay(frame(stationId), overlayId || null);
  }

  function detachLoads() {
    generation += 1;
    for (const { iframe, onLoad } of mounted.values()) iframe.removeEventListener('load', onLoad);
    mounted.clear();
  }

  function mount(nextShipStations) {
    if (disposed || !container) return;
    detachLoads();
    shipStations = nextShipStations;
    const currentGeneration = generation;
    mountConsoles(doc, container, shipStations, (iframe, plan) => {
      const stationId = plan.stationId;
      const record = { iframe, section: iframe.parentElement, onLoad: null };
      record.onLoad = () => {
        // Removing a listener does not revoke a callback already queued. In
        // particular Welcome may request locale reloads before replacing the
        // frames. An obsolete load must never publish into its replacement.
        if (disposed || generation !== currentGeneration || mounted.get(stationId) !== record) return;
        installLocale(iframe);
        refresh(stationId, PUSH_CAUSE.IFRAME_LOAD);
        if (selectedOverlay.console === stationId) selectOverlay(null, null);
        applyAccessibility(iframe);
        publishBindings(iframe);
      };
      mounted.set(stationId, record);
      iframe.addEventListener('load', record.onLoad);
    });
  }

  function show(activeStation, inGame) {
    if (disposed) return {};
    const stationIds = (shipStations?.stations || []).map(station => station.id).filter(Boolean);
    return applyConsoleVisibility(doc, activeStation, inGame, stationIds);
  }

  function view(stationId) {
    return {
      tabs: tabs.get(stationId) || [],
      hull: hull.get(stationId) || [],
      activeOverlay: selectedOverlay.console === stationId ? selectedOverlay.id : null,
    };
  }

  function handleDeclaration(event) {
    if (disposed) return false;
    const data = event?.data;
    if (data?.type !== 'console_tabs' && data?.type !== 'console_hull') return false;
    // Preserve the non-frame compatibility paths, including an unmatched
    // source's claimed name. This is presentation routing, not Admission.
    let stationId = typeof data.console === 'string' ? data.console : '';
    if (event.source) {
      for (const [id, { iframe }] of mounted) {
        let source = null;
        try { source = iframe.contentWindow; } catch (_) { /* unavailable */ }
        if (source && source === event.source) { stationId = id; break; }
      }
    }
    if (!stationId) return true;
    if (data.type === 'console_tabs') {
      tabs.set(stationId, Array.isArray(data.tabs) ? data.tabs : []);
      const open = data.open || null;
      if (open) selectedOverlay = { console: stationId, id: open };
      else if (selectedOverlay.console === stationId) selectedOverlay = { console: null, id: null };
    } else hull.set(stationId, Array.isArray(data.entries) ? data.entries : []);
    onDeclaration({ type: data.type, stationId });
    return true;
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    for (const { section } of mounted.values()) section.remove();
    detachLoads();
    shipStations = null;
    tabs.clear();
    hull.clear();
    selectedOverlay = { console: null, id: null };
  }

  return { mount, show, frame, publishChanges, refresh, refreshBindings,
    selectOverlay, view, handleDeclaration, dispose };
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.createConsoleMounts = createConsoleMounts;
  window.mountConsolesDom = mountConsoles;
  window.applyConsoleVisibility = applyConsoleVisibility;
  window.consoleStationIdForSource = stationIdForSource;
}
