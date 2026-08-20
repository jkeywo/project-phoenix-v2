/**
 * tests/client/accessibility-presentation.test.js — the observable presentation
 * effect (issue #1102 AC3).
 *
 * Drives the REAL production seams under JSDOM: gui/console-mount.js mounts the
 * persistent console iframes, then gui/accessibility-profile.js applies a
 * text-scale setting. The assertion is the whole acceptance criterion — the
 * --a11y-text-scale var lands on BOTH the shell documentElement AND each
 * same-origin console iframe :root, so the console root font-size (which
 * multiplies by it, see gui/console.css) scales every string at once. This is
 * the shell-push path client.html wires at boot / on iframe load.
 */
import { JSDOM } from 'jsdom';
import { describe, it, expect } from 'vitest';
import { mountConsoles, applyConsoleVisibility } from '../../gui/console-mount.js';
import {
  applyAccessibilityProfile,
  profileWithPresentation,
  emptyAccessibilityProfile,
  TEXT_SCALE_VAR,
  TEXT_SCALE_DEFAULT,
} from '../../gui/accessibility-profile.js';

const SHIP = {
  stations: [
    { id: 'helm', name: 'Helm', console: 'gui/battleship/helm.html' },
    { id: 'navigation', name: 'Navigation', console: 'gui/battleship/navigation.html' },
    { id: 'comms', name: 'Comms', console: 'gui/battleship/comms.html' },
  ],
};

function harness() {
  const dom = new JSDOM('<div id="console-container"></div>', { url: 'https://phoenix.test/' });
  const doc = dom.window.document;
  mountConsoles(doc, doc.getElementById('console-container'), SHIP);
  const ids = SHIP.stations.map((st) => st.id);
  // Make helm the active console, exactly as the tab switch does in production.
  applyConsoleVisibility(doc, 'helm', true, ids);
  // JSDOM does not LOAD an iframe's src, so a mounted console's contentDocument
  // has no :root to write onto. A real browser gives every SAME-ORIGIN iframe a
  // live document; navigate each to about:blank so JSDOM materialises that same
  // :root, standing in for it. The mount + `.console-section iframe` discovery
  // seam under test is still the real one — only the document JSDOM refuses to
  // load is supplied here.
  for (const st of SHIP.stations) {
    doc.getElementById(`${st.id}-iframe`).setAttribute('src', 'about:blank');
  }
  return { dom, doc, ids };
}

/** The text-scale var written on one iframe's contentDocument :root, if reachable. */
function iframeVar(doc, iframeId) {
  const iframe = doc.getElementById(iframeId);
  const idoc = iframe && iframe.contentDocument;
  const root = idoc && idoc.documentElement;
  return root ? root.style.getPropertyValue(TEXT_SCALE_VAR) : null;
}

describe('text scale applies across the console surfaces (issue #1102 AC3)', () => {
  it('lands the --a11y-text-scale var on the shell root and every mounted console root', () => {
    const { dom, doc } = harness();
    const profile = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.3);

    const effects = applyAccessibilityProfile(profile, { doc, win: dom.window });

    expect(effects.textScale).toBeCloseTo(1.3);
    // Shell documentElement.
    expect(doc.documentElement.style.getPropertyValue(TEXT_SCALE_VAR)).toBe('1.3');
    // Each same-origin console iframe :root.
    expect(iframeVar(doc, 'helm-iframe')).toBe('1.3');
    expect(iframeVar(doc, 'navigation-iframe')).toBe('1.3');
    expect(iframeVar(doc, 'comms-iframe')).toBe('1.3');
  });

  it('a change re-applies to every root, and reset returns to the identity', () => {
    const { dom, doc } = harness();
    applyAccessibilityProfile(
      profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.5),
      { doc, win: dom.window },
    );
    expect(iframeVar(doc, 'helm-iframe')).toBe('1.5');

    // Setting text scale back to default resolves to the identity everywhere.
    const reset = profileWithPresentation(
      profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.5),
      'textScale', 'default',
    );
    applyAccessibilityProfile(reset, { doc, win: dom.window });
    expect(doc.documentElement.style.getPropertyValue(TEXT_SCALE_VAR)).toBe(String(TEXT_SCALE_DEFAULT));
    expect(iframeVar(doc, 'helm-iframe')).toBe(String(TEXT_SCALE_DEFAULT));
  });

  it('discovers the console iframes itself when none are passed explicitly', () => {
    // The production shell calls applyAccessibilityProfile() with no `iframes`,
    // so the module must find the `.console-section iframe` nodes under `doc`.
    const { dom, doc } = harness();
    applyAccessibilityProfile(
      profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.2),
      { doc, win: dom.window },
    );
    expect(iframeVar(doc, 'comms-iframe')).toBe('1.2');
  });
});
