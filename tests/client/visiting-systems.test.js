// @vitest-environment jsdom
/**
 * tests/client/visiting-systems.test.js — the buttons the human seek puts on a
 * console's hero bar (issue #984, pasm decision
 * `console-complexity-human-seeking-systems`).
 *
 * The whole feature the player sees is "Comms appears on my console when
 * nobody is at Tactical, and goes away again when someone sits down there", so
 * these tests drive the real destroyer markup through the real renderer and
 * assert on the buttons rather than on the payload that produced them.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { JSDOM } from 'jsdom';
import { renderVisitingSystems } from '../../gui/visiting-systems.js';
import { initConsoleOverlays, toggleConsoleOverlay } from '../../gui/console-overlays.js';

const CONSOLES = ['captain', 'helm', 'tactical', 'engineering'];

/**
 * The destroyer console's own HTML, in a live DOM. Reading the shipped file is
 * the point: a console that spells the ids differently is a console the seek
 * cannot reach, and nothing else in the suite would notice.
 */
function consoleDoc(name) {
  const file = path.join(process.cwd(), 'gui', 'destroyer', `${name}.html`);
  const html = fs.readFileSync(file, 'utf8');
  // Scripts stay off: the console modules import web components and
  // console-core, none of which this module needs to be exercised.
  const dom = new JSDOM(html, { runScripts: 'outside-only' });
  return dom.window.document;
}

/** A payload holding `ids`, shaped like withVisitingSystems' output. */
function payload(ids, systems = {}) {
  return { hosted_systems: ids, systems };
}

const COMMS_VIEW = {
  contacts: [{ uuid: 'u1', name: 'Kestrel' }],
  messages: [{ id: 'm1', is_read: false, text: 'Hail' }],
};
const NAV_VIEW = { blips: [], regions: [], radar_range: 4000, civilians: [] };

describe('every destroyer console carries the seek markup', () => {
  it.each(CONSOLES)('%s has both toggles and both panels, hidden at rest', (name) => {
    const doc = consoleDoc(name);
    for (const id of ['nav-toggle', 'comms-toggle']) {
      const btn = doc.getElementById(id);
      expect(btn, `${name} must have #${id}`).not.toBeNull();
      expect(btn.hasAttribute('hidden'), `${name} #${id} starts hidden`).toBe(true);
    }
    for (const id of ['nav-overlay', 'comms-overlay']) {
      const panel = doc.getElementById(id);
      expect(panel, `${name} must have #${id}`).not.toBeNull();
      expect(panel.classList.contains('open')).toBe(false);
    }
    // The toggle names its panel — the convention gui/console-overlays.js
    // wires, and the only thing connecting the two.
    expect(doc.getElementById('nav-toggle').dataset.overlay).toBe('nav-overlay');
    expect(doc.getElementById('comms-toggle').dataset.overlay).toBe('comms-overlay');
  });
});

describe('renderVisitingSystems — appearance and disappearance', () => {
  it.each(CONSOLES)('%s shows Comms while it hosts it and hides it when the seek moves on', (name) => {
    const doc = consoleDoc(name);
    const btn = doc.getElementById('comms-toggle');

    renderVisitingSystems(payload(['comms'], { comms: COMMS_VIEW }), doc);
    expect(btn.hidden).toBe(false);

    // The seek moves to another station: the view may still be in the payload
    // (Tactical's Intel panel reads it), but this console no longer holds it.
    renderVisitingSystems(payload([], { comms: COMMS_VIEW }), doc);
    expect(btn.hidden).toBe(true);
  });

  it('shows each system independently', () => {
    const doc = consoleDoc('engineering');
    renderVisitingSystems(payload(['navigation'], { navigation: NAV_VIEW }), doc);
    expect(doc.getElementById('nav-toggle').hidden).toBe(false);
    expect(doc.getElementById('comms-toggle').hidden).toBe(true);
  });

  it('stays hidden when the station is named as host but the view has not arrived', () => {
    const doc = consoleDoc('engineering');
    renderVisitingSystems(payload(['comms'], {}), doc);
    expect(doc.getElementById('comms-toggle').hidden).toBe(true);
  });

  // Backwards compatibility with a payload built before the field existed.
  it('renders on the view alone when no hosted list is present', () => {
    const doc = consoleDoc('tactical');
    renderVisitingSystems({ systems: { comms: COMMS_VIEW } }, doc);
    expect(doc.getElementById('comms-toggle').hidden).toBe(false);
  });
});

describe('renderVisitingSystems — an open panel does not outlive the seek', () => {
  it('closes the panel and unlights the toggle when the system leaves', () => {
    const doc = consoleDoc('engineering');
    initConsoleOverlays(doc);

    renderVisitingSystems(payload(['comms'], { comms: COMMS_VIEW }), doc);
    doc.getElementById('comms-toggle').click();
    expect(doc.getElementById('comms-overlay').classList.contains('open')).toBe(true);

    // The panel covers the console. Leaving it up would strand the operator on
    // a surface they no longer hold, with the button that closes it gone.
    renderVisitingSystems(payload([], {}), doc);
    expect(doc.getElementById('comms-overlay').classList.contains('open')).toBe(false);
    expect(doc.getElementById('comms-toggle').dataset.active).toBe('false');
    expect(doc.getElementById('comms-toggle').classList.contains('active')).toBe(false);
  });
});

describe('renderVisitingSystems — what the panels are fed', () => {
  let doc;
  beforeEach(() => {
    doc = consoleDoc('helm');
  });

  it('feeds the comms panels from the visiting view', () => {
    renderVisitingSystems(payload(['comms'], { comms: COMMS_VIEW }), doc);
    expect(doc.getElementById('comms-contact-list').state).toEqual({
      contacts: COMMS_VIEW.contacts,
    });
    expect(doc.getElementById('comms-current-message').state.thread).toEqual(
      COMMS_VIEW.messages[0],
    );
    // The unread flash: the same badge Tactical has always shown.
    expect(doc.getElementById('comms-unread').classList.contains('show')).toBe(true);
  });

  it('keeps the last exchange on screen when nothing is unread', () => {
    const read = { messages: [{ id: 'm1', is_read: true }, { id: 'm2', is_read: true }] };
    renderVisitingSystems(payload(['comms'], { comms: read }), doc);
    expect(doc.getElementById('comms-current-message').state.thread.id).toBe('m2');
    expect(doc.getElementById('comms-unread').classList.contains('show')).toBe(false);
  });

  it('feeds the navigation panels from the visiting view', () => {
    const nav = { ...NAV_VIEW, ship_x: 12, ship_z: -4, ship_heading: 1.5, civilians: [{ uuid: 'c1' }] };
    renderVisitingSystems(payload(['navigation'], { navigation: nav }), doc);
    expect(doc.getElementById('navigation-map').state.ship_pos).toEqual({ x: 12, z: -4 });
    expect(doc.getElementById('navigation-map').state.range).toBe(4000);
    expect(doc.getElementById('civilian-traffic').state).toEqual({ civilians: nav.civilians });
  });

  it('touches nothing when the console holds neither system', () => {
    renderVisitingSystems(payload([], {}), doc);
    expect(doc.getElementById('comms-contact-list').state).toBeUndefined();
    expect(doc.getElementById('navigation-map').state).toBeUndefined();
  });
});

describe('console overlays — one panel at a time', () => {
  it('opening one closes the others', () => {
    const doc = consoleDoc('tactical');
    initConsoleOverlays(doc);
    renderVisitingSystems(payload(['navigation', 'comms'], {
      navigation: NAV_VIEW,
      comms: COMMS_VIEW,
    }), doc);

    doc.getElementById('nav-toggle').click();
    expect(doc.getElementById('nav-overlay').classList.contains('open')).toBe(true);

    doc.getElementById('comms-toggle').click();
    expect(doc.getElementById('nav-overlay').classList.contains('open')).toBe(false);
    expect(doc.getElementById('comms-overlay').classList.contains('open')).toBe(true);
    expect(doc.getElementById('nav-toggle').classList.contains('active')).toBe(false);
    expect(doc.getElementById('comms-toggle').classList.contains('active')).toBe(true);
  });

  it('a second press on the open panel closes it', () => {
    const doc = consoleDoc('tactical');
    initConsoleOverlays(doc);
    doc.getElementById('intel-toggle').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(true);
    doc.getElementById('intel-toggle').click();
    expect(doc.getElementById('intel-overlay').classList.contains('open')).toBe(false);
  });

  it('the back button inside a panel closes it', () => {
    const doc = consoleDoc('captain');
    initConsoleOverlays(doc);
    toggleConsoleOverlay('comms-overlay', doc);
    expect(doc.getElementById('comms-overlay').classList.contains('open')).toBe(true);

    doc.querySelector('#comms-overlay [data-overlay-back]').click();
    expect(doc.getElementById('comms-overlay').classList.contains('open')).toBe(false);
  });
});
