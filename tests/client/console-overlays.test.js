// @vitest-environment jsdom
/**
 * tests/client/console-overlays.test.js — the toggle/full-frame-panel pair a
 * console uses to show a surface it does not have room for inline
 * (gui/console-overlays.js).
 *
 * Comms and Navigation used to be the module's main customers (issue #984),
 * routed through the now-retired gui/visiting-systems.js. Both are complete
 * hero-bar Stations now (issues #1097, #1098) and no longer use this pattern;
 * the destroyer Tactical console's Intel panel (issue #1030) is the one
 * surviving consumer, so these tests drive the real shipped markup through
 * the real module the same way the retired suite did for Comms/Navigation.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { JSDOM } from 'jsdom';
import {
  closeConsoleOverlays, initConsoleOverlays, openConsoleOverlayId,
  setConsoleOverlay, toggleConsoleOverlay,
} from '../../gui/console-overlays.js';

/** The destroyer Tactical console's own HTML, in a live DOM. */
function tacticalDoc() {
  const file = path.join(process.cwd(), 'gui', 'destroyer', 'tactical.html');
  const html = fs.readFileSync(file, 'utf8');
  // Scripts stay off: the console module imports web components and
  // console-core, none of which this module needs to be exercised.
  const dom = new JSDOM(html, { runScripts: 'outside-only' });
  return dom.window.document;
}

describe('the destroyer Tactical console carries the Intel overlay markup', () => {
  it('has the toggle and panel, matched by data-overlay', () => {
    const doc = tacticalDoc();
    const toggle = doc.getElementById('intel-toggle');
    const panel = doc.getElementById('intel-overlay');
    expect(toggle).not.toBeNull();
    expect(panel).not.toBeNull();
    expect(panel.classList.contains('open')).toBe(false);
    expect(toggle.dataset.overlay).toBe('intel-overlay');
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

describe('console overlays — one panel at a time', () => {
  it('a press opens the panel and lights the toggle', () => {
    const doc = tacticalDoc();
    initConsoleOverlays(doc);
    doc.getElementById('intel-toggle').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
    expect(doc.getElementById('intel-toggle').classList.contains('active')).toBe(true);
  });

  it('a second press on the open panel closes it', () => {
    const doc = tacticalDoc();
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

  it('setConsoleOverlay opens exactly the named panel and lights its toggle', () => {
    const doc = tacticalDoc();
    expect(setConsoleOverlay('intel-overlay', doc)).toBe('intel-overlay');
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
    expect(doc.getElementById('intel-toggle').classList.contains('active')).toBe(true);
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
    const doc = tacticalDoc();
    initConsoleOverlays(doc);
    doc.getElementById('intel-toggle').click();
    closeConsoleOverlays(doc);
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(false);
    expect(doc.getElementById('intel-toggle').classList.contains('active')).toBe(false);
    expect(doc.getElementById('intel-toggle').dataset.active).toBe('false');
  });
});
