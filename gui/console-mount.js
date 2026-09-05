/**
 * gui/console-mount.js — The DOM mount + `.active` visibility toggle for ship
 * consoles (extracted from client.html for issue #1099 AC1).
 *
 * These two functions are the production seams that make a Station tab's
 * session-local interface context survive a tab switch:
 *
 *  - mountConsoles()          creates exactly one persistent <section>+<iframe>
 *                             per station (from gui/mount-plan.js planMounts),
 *                             fired only on the Welcome `mount-consoles` side
 *                             effect. It is the ONLY place iframe nodes are
 *                             created, so nodes it mounts are stable for the
 *                             session.
 *  - applyConsoleVisibility() is the tab switch: a pure `.active` CSS toggle
 *                             over the already-mounted sections (visibility map
 *                             from gui/content-switcher.js consoleSections). It
 *                             never creates or removes nodes, so switching away
 *                             and back leaves the same iframe node — and any
 *                             interface state living on/in it — untouched.
 *  - stationIdForSource()     reads the mount backwards: given the window a
 *                             postMessage came FROM, which Station is that?
 *                             The shell mounted the frame, so the shell is the
 *                             one that knows (issue #1374).
 *
 * client.html's inline shell calls these via window.mountConsolesDom /
 * window.applyConsoleVisibility / window.consoleStationIdForSource; tests
 * import them directly.
 */

import { iframeIdFor, planMounts } from './mount-plan.js';
import { consoleSections } from './content-switcher.js';

/**
 * Mount one persistent section+iframe per plan entry into `container`.
 * Removes any previously-mounted `.console-section` first (leaving siblings
 * like #lobby-ui untouched). This is the sole creator of console iframe nodes.
 *
 * @param {Document} doc          Owning document (for createElement).
 * @param {Element}  container    The #console-container element.
 * @param {object|null} shipStations  Server-supplied ship_stations.
 * @param {(iframe: Element, mount: object) => void} [onIframe]
 *        Optional per-iframe hook (client.html attaches its load listener).
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

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.mountConsolesDom = mountConsoles;
  window.applyConsoleVisibility = applyConsoleVisibility;
  window.consoleStationIdForSource = stationIdForSource;
}
