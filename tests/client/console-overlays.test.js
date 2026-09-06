// @vitest-environment jsdom
/**
 * tests/client/console-overlays.test.js — the toggle/full-frame-panel pair a
 * console uses to show a surface it does not have room for inline
 * (gui/console-overlays.js).
 *
 * Comms and Navigation used to be the module's main customers (issue #984),
 * routed through the now-retired gui/visiting-systems.js. Both are complete
 * hero-bar Stations now (issues #1097, #1098) and no longer use this pattern;
 * the destroyer Tactical console's Intel and Security panels (issues #1030,
 * #1346) are what ships today, so the PANEL half of these tests drives the
 * real shipped markup the way the retired suite did for Comms/Navigation.
 *
 * The TOGGLE half drives a fixture instead, and that is a statement about the
 * fleet rather than a shortcut: since issue #1374 no console this module
 * drives authors a `data-overlay` toggle — the shell's Station Bar selects a
 * panel through `setConsoleOverlay`, and the panel's Back button is the way
 * out. The toggle convention is still supported for a surface the bar has no
 * business offering, so it is still tested; there is simply no shipped markup
 * left to test it against. `toggleFixtureDoc()` is that markup, written to
 * the convention gui/console-overlays.js documents.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { JSDOM } from 'jsdom';
import {
  closeConsoleOverlays, initConsoleOverlays, openConsoleOverlayId,
  setConsoleOverlay, toggleConsoleOverlay,
} from '../../gui/console-overlays.js';

/** One shipped console's own HTML, in a live DOM. */
function consoleDoc(...relative) {
  const file = path.join(process.cwd(), 'gui', ...relative);
  const html = fs.readFileSync(file, 'utf8');
  // Scripts stay off: the console module imports web components and
  // console-core, none of which this module needs to be exercised.
  const dom = new JSDOM(html, { runScripts: 'outside-only' });
  return dom.window.document;
}

/** The destroyer Tactical console's own HTML, in a live DOM. */
const tacticalDoc = () => consoleDoc('destroyer', 'tactical.html');

/** The cruiser Tactical console's own HTML, in a live DOM (issue #1389). */
const cruiserTacticalDoc = () => consoleDoc('cruiser', 'tactical.html');

/**
 * A console written to the toggle convention, for the half of the module no
 * shipped document exercises any more (issue #1374). Deliberately the shape
 * gui/console-overlays.js's own header documents, so a change to that
 * convention breaks this rather than passing against a private variant.
 */
function toggleFixtureDoc() {
  const dom = new JSDOM(`<!DOCTYPE html><body>
    <button class="overlay-toggle" id="intel-toggle" data-overlay="intel-overlay" data-active="false">Intel</button>
    <button class="overlay-toggle" id="security-toggle" data-overlay="security-overlay" data-active="false">Security</button>
    <div class="overlay-panel" id="intel-overlay">
      <button class="overlay-back" data-overlay-back>Back</button>
    </div>
    <div class="overlay-panel" id="security-overlay">
      <button class="overlay-back" data-overlay-back>Back</button>
    </div>
  </body>`, { runScripts: 'outside-only' });
  return dom.window.document;
}

describe('the destroyer Tactical console carries the Intel overlay markup', () => {
  it('has both panels, closed, and no in-console toggle of its own', () => {
    const doc = tacticalDoc();
    for (const id of ['intel-overlay', 'security-overlay']) {
      const panel = doc.getElementById(id);
      expect(panel, `#${id} must be present`).not.toBeNull();
      expect(panel.classList.contains('open')).toBe(false);
    }
    // Issue #1374: the bar owns the selection, so the console authors no
    // toggle. Two affordances for one choice is how a tab and a button end up
    // disagreeing about which panel is open.
    expect(doc.querySelectorAll('.overlay-toggle').length).toBe(0);
    expect(doc.getElementById('intel-toggle')).toBeNull();
    expect(doc.getElementById('security-toggle')).toBeNull();
  });

  // Issue #1373: the panel itself is what declares a tab on the shell's
  // Station Bar. Both attributes hold strings.csv ids — the bar resolves them
  // — so this reads the shipped markup rather than a fixture, the same way the
  // toggle/panel pairing above does.
  it('declares both overlays as Station Bar tabs, by string id', () => {
    const doc = tacticalDoc();
    const intel = doc.getElementById('intel-overlay');
    const security = doc.getElementById('security-overlay');
    expect(intel.dataset.tabCode).toBe('console.tactical.intel.code');
    expect(intel.dataset.tabName).toBe('console.tactical.intel');
    expect(security.dataset.tabCode).toBe('console.tactical.security.code');
    expect(security.dataset.tabName).toBe('console.tactical.security');
    // The scan console-core runs is exactly this selector.
    expect([...doc.querySelectorAll('.overlay-panel[data-tab-code]')].map(p => p.id))
      .toEqual(['security-overlay', 'intel-overlay']);
  });

  it('no longer carries the retired Nav/Comms overlay ids (issues #1097, #1098)', () => {
    const doc = tacticalDoc();
    for (const id of ['nav-toggle', 'nav-overlay', 'comms-toggle', 'comms-overlay']) {
      expect(doc.getElementById(id), `#${id} must be gone`).toBeNull();
    }
  });
});

describe('the cruiser Tactical console carries the Security overlay markup (#1389)', () => {
  it('has the panel, closed, holding the team surface, with no toggle of its own', () => {
    const doc = cruiserTacticalDoc();
    const panel = doc.getElementById('security-overlay');
    expect(panel, '#security-overlay must be present').not.toBeNull();
    expect(panel.classList.contains('open')).toBe(false);
    expect(panel.querySelector('ph-security-teams#security-teams')).not.toBeNull();
    // Issue #1374: the bar owns the selection, so the console authors no
    // toggle — its Back button is the only in-console way out.
    expect(doc.querySelectorAll('.overlay-toggle').length).toBe(0);
    expect(panel.querySelector('[data-overlay-back]')).not.toBeNull();
  });

  it('declares the overlay as a Station Bar tab, by string id', () => {
    const doc = cruiserTacticalDoc();
    const security = doc.getElementById('security-overlay');
    expect(security.dataset.tabCode).toBe('console.tactical.security.code');
    expect(security.dataset.tabName).toBe('console.tactical.security');
    // The scan console-core runs is exactly this selector. Security is the ONLY
    // tab this hull's Tactical seat declares: the Intel panel is the cruiser
    // Tactical relayout's (issue #1393), not this slice's.
    expect([...doc.querySelectorAll('.overlay-panel[data-tab-code]')].map(p => p.id))
      .toEqual(['security-overlay']);
  });

  it('opens on the bar\'s selection and closes on its own Back button', () => {
    const doc = cruiserTacticalDoc();
    initConsoleOverlays(doc);
    expect(setConsoleOverlay('security-overlay', doc)).toBe('security-overlay');
    expect(openConsoleOverlayId(doc)).toBe('security-overlay');

    doc.querySelector('#security-overlay [data-overlay-back]').click();
    expect(openConsoleOverlayId(doc)).toBeNull();
  });
});

describe('console overlays — one panel at a time', () => {
  it('a press opens the panel and lights the toggle', () => {
    const doc = toggleFixtureDoc();
    initConsoleOverlays(doc);
    doc.getElementById('intel-toggle').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
    expect(doc.getElementById('intel-toggle').classList.contains('active')).toBe(true);
  });

  it('a second press on the open panel closes it', () => {
    const doc = toggleFixtureDoc();
    initConsoleOverlays(doc);
    doc.getElementById('intel-toggle').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
    doc.getElementById('intel-toggle').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(false);
  });

  it('the back button inside a panel closes it', () => {
    const doc = tacticalDoc();
    initConsoleOverlays(doc);
    toggleConsoleOverlay('intel-overlay', doc);
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);

    doc.querySelector('#intel-overlay [data-overlay-back]').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(false);
  });

  it('setConsoleOverlay opens exactly the named panel', () => {
    const doc = tacticalDoc();
    expect(setConsoleOverlay('intel-overlay', doc)).toBe('intel-overlay');
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
  });

  it('lights the matching toggle where a console authors one', () => {
    const doc = toggleFixtureDoc();
    expect(setConsoleOverlay('intel-overlay', doc)).toBe('intel-overlay');
    expect(doc.getElementById('intel-toggle').classList.contains('active')).toBe(true);
    expect(doc.getElementById('security-toggle').classList.contains('active')).toBe(false);
  });

  it('setConsoleOverlay is SET, not toggle: the same id twice leaves it open', () => {
    const doc = tacticalDoc();
    setConsoleOverlay('intel-overlay', doc);
    setConsoleOverlay('intel-overlay', doc);
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
  });

  it('setConsoleOverlay swaps panels without ever leaving two open', () => {
    const doc = tacticalDoc();
    setConsoleOverlay('intel-overlay', doc);
    setConsoleOverlay('security-overlay', doc);
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(false);
    expect(doc.getElementById('security-overlay').classList.contains('open')).toBe(true);
    expect(doc.querySelectorAll('.overlay-panel.open').length).toBe(1);
  });

  it('setConsoleOverlay(null) closes everything', () => {
    const doc = tacticalDoc();
    setConsoleOverlay('intel-overlay', doc);
    expect(setConsoleOverlay(null, doc)).toBeNull();
    expect(doc.querySelectorAll('.overlay-panel.open').length).toBe(0);
  });

  it('setConsoleOverlay(null) unlights a toggle too', () => {
    const doc = toggleFixtureDoc();
    setConsoleOverlay('intel-overlay', doc);
    expect(setConsoleOverlay(null, doc)).toBeNull();
    expect(doc.getElementById('intel-toggle').dataset.active).toBe('false');
  });

  it('setConsoleOverlay reports null for an id this document has no panel for', () => {
    const doc = tacticalDoc();
    setConsoleOverlay('intel-overlay', doc);
    expect(setConsoleOverlay('nav-overlay', doc)).toBeNull();
    expect(doc.querySelectorAll('.overlay-panel.open').length).toBe(0);
  });

  it('openConsoleOverlayId names whatever is covering the console', () => {
    const doc = tacticalDoc();
    expect(openConsoleOverlayId(doc)).toBeNull();
    setConsoleOverlay('security-overlay', doc);
    expect(openConsoleOverlayId(doc)).toBe('security-overlay');
    closeConsoleOverlays(doc);
    expect(openConsoleOverlayId(doc)).toBeNull();
  });

  it('closeConsoleOverlays unlights every toggle and closes every panel', () => {
    const doc = toggleFixtureDoc();
    initConsoleOverlays(doc);
    doc.getElementById('intel-toggle').click();
    closeConsoleOverlays(doc);
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(false);
    expect(doc.getElementById('intel-toggle').classList.contains('active')).toBe(false);
    expect(doc.getElementById('intel-toggle').dataset.active).toBe('false');
  });
});
