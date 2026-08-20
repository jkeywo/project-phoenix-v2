import { JSDOM } from 'jsdom';
import { describe, it, expect } from 'vitest';
import { mountConsoles, applyConsoleVisibility } from '../../gui/console-mount.js';

// Issue #1099 AC1: switching away from a Station tab and back preserves its
// session-local interface context. The client mounts ONE persistent iframe per
// ship station once (mountConsoles, fired only on the Welcome `mount-consoles`
// side effect); a tab switch is a pure `.active` CSS toggle
// (applyConsoleVisibility), and a SimState hosting change never re-mounts. So
// the iframe DOM node — and any interface state living inside it — must survive
// an activeConsole switch away and back untouched.
//
// This drives the REAL production seams from gui/console-mount.js — the same
// functions client.html calls (window.mountConsolesDom /
// window.applyConsoleVisibility) — so node identity and local-state survival
// are asserted of production code, not of a copy. If a regression made the
// mount rebuild iframe nodes, or made the toggle re-parent/re-create them, the
// node-identity assertions below would fail. The "Welcome-only remount" half is
// pinned at the router seam in client-router.test.js.

const SHIP = {
  stations: [
    { id: 'helm', name: 'Helm', console: 'gui/battleship/helm.html' },
    { id: 'navigation', name: 'Navigation', console: 'gui/battleship/navigation.html' },
    { id: 'comms', name: 'Comms', console: 'gui/battleship/comms.html' },
  ],
};

/** Drive the real tab-switch toggle (inGame === true) for the active console. */
function applyActive(doc, activeConsole, stationIds) {
  applyConsoleVisibility(doc, activeConsole, true, stationIds);
}

function harness() {
  const dom = new JSDOM('<div id="console-container"></div>', { url: 'https://phoenix.test/' });
  const doc = dom.window.document;
  const container = doc.getElementById('console-container');
  mountConsoles(doc, container, SHIP);
  const ids = SHIP.stations.map(st => st.id);
  return { doc, container, ids };
}

describe('console iframe persistence across tab switches (issue #1099 AC1)', () => {
  it('keeps the same iframe node and its session-local state when switching away and back', () => {
    const { doc, ids } = harness();
    applyActive(doc, 'helm', ids);

    const helmIframe = doc.getElementById('helm-iframe');
    // Stamp session-local interface context onto the live iframe node — the
    // stand-in for scroll position, a half-typed field, a selected contact.
    helmIframe.dataset.localContext = 'half-typed-hail';
    helmIframe.__sessionScroll = 512;

    // Switch away to a visiting Station tab, then back to helm.
    applyActive(doc, 'navigation', ids);
    applyActive(doc, 'helm', ids);

    const afterReturn = doc.getElementById('helm-iframe');
    expect(afterReturn).toBe(helmIframe); // same DOM node, never re-created
    expect(afterReturn.dataset.localContext).toBe('half-typed-hail');
    expect(afterReturn.__sessionScroll).toBe(512);
    // The section is shown again by class alone.
    expect(afterReturn.parentElement.className).toBe('console-section active');
  });

  it('an inactive tab is only hidden, never unmounted', () => {
    const { doc, ids } = harness();
    applyActive(doc, 'helm', ids);
    const navIframe = doc.getElementById('navigation-iframe');
    navIframe.dataset.localContext = 'course-plotted';

    // helm active → navigation is inactive but must still exist with its state.
    expect(navIframe.parentElement.className).toBe('console-section');
    expect(doc.getElementById('navigation-iframe')).toBe(navIframe);
    expect(navIframe.dataset.localContext).toBe('course-plotted');
  });

  it('a hosting change (re-applying visibility) does not disturb iframe nodes', () => {
    const { doc, ids } = harness();
    applyActive(doc, 'comms', ids);
    const before = ids.map(id => doc.getElementById(`${id}-iframe`));

    // A SimState station_hosts change re-runs the visibility pass but never
    // re-mounts: apply it repeatedly and every iframe node is identity-stable.
    applyActive(doc, 'comms', ids);
    applyActive(doc, 'helm', ids);
    applyActive(doc, 'comms', ids);

    const after = ids.map(id => doc.getElementById(`${id}-iframe`));
    for (let i = 0; i < before.length; i++) expect(after[i]).toBe(before[i]);
  });
});
