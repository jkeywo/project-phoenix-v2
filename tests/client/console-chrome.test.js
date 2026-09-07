// @vitest-environment jsdom
/**
 * tests/client/console-chrome.test.js — one header everywhere (issue #1374).
 *
 * Before this slice every console document drew its own chrome: a `.hero`
 * title row saying the name of the seat, a `.console-footer` strip saying it
 * again next to a station-damage bar, and — through `gui/console.css` — a 44px
 * left gutter held clear for a settings cog that has been a child of the
 * shell's Station Bar since issue #1372. On a phone in landscape that is three
 * bands of a five-band screen spent on chrome, and the bar above the iframe
 * was already saying all of it.
 *
 * These are the sweep that keeps it gone. Structural, over every console
 * document found on disk rather than a list to forget to add to, because the
 * failure mode is one console quietly growing its title row back and nobody
 * noticing until they open that console on that device.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { JSDOM } from 'jsdom';
import { parse as parseToml } from 'smol-toml';
import { mountConsoles, stationIdForSource } from '../../gui/console-mount.js';
import { GUI, REPO_ROOT, consoleDocuments, readStripped, rel } from './css-scan.js';

/**
 * Every console document. `consoleDocuments()` walks the hull directories;
 * `gui/command-console.html` sits at the top of `gui/` because Command is not
 * a hull's own station, and it wore exactly the same chrome, so it is swept
 * with the rest.
 */
function allConsoleDocuments() {
  return [...consoleDocuments(), path.join(GUI, 'command-console.html')].sort();
}

const CONSOLE_CSS = fs.readFileSync(path.join(GUI, 'console.css'), 'utf8');

/** A console document parsed, with its scripts inert. */
function docFor(file) {
  const html = fs.readFileSync(file, 'utf8');
  return new JSDOM(html, { runScripts: 'outside-only' }).window.document;
}

describe('no console draws its own chrome', () => {
  it('finds the whole fleet of console documents', () => {
    // A silent zero would let every assertion below pass over nothing.
    expect(allConsoleDocuments().length).toBe(22);
  });

  for (const file of allConsoleDocuments()) {
    const name = rel(file);
    it(`${name} has no title row, no footer and no own-station damage bar`, () => {
      const doc = docFor(file);
      // The title row, in either spelling the fleet used — `.hero` on twenty of
      // them, a bare `<h1>` on gui/courier/tactical.html.
      expect(doc.querySelectorAll('.hero').length, '.hero row').toBe(0);
      expect(doc.querySelectorAll('.hero-controls').length, '.hero-controls').toBe(0);
      expect(doc.body.querySelectorAll('h1').length, 'a title heading').toBe(0);
      // The footer, in all three spellings: `.console-footer` on eighteen,
      // `.footer` on gui/courier/captain.html, and a classless `<footer>` on
      // gui/courier/tactical.html.
      expect(doc.querySelectorAll('footer').length, 'a <footer>').toBe(0);
      expect(doc.querySelectorAll('.console-footer, .footer').length, 'a footer strip').toBe(0);
      // The bar the footer carried. `id="core-damage"` — the SHIP's core hull,
      // on Repair and Engineering — is a different readout and stays; what goes
      // is the console's own-Station one, which the bar's popup replaced.
      expect(doc.getElementById('station-damage'), '#station-damage').toBeNull();
      expect(doc.getElementById('damage'), '#damage').toBeNull();
    });
  }

  it('gui/console.css keeps no rules for the chrome it no longer has', () => {
    const css = readStripped(path.join(GUI, 'console.css'));
    // Dead rules are how a class comes back: the next console to want a title
    // row would find `.hero` still styled and simply use it.
    expect(/(^|[\s,}])\.hero\s*[,{]/.test(css), '.hero rule').toBe(false);
    expect(/(^|[\s,}])\.console-footer\s*[,{]/.test(css), '.console-footer rule').toBe(false);
    expect(/(^|[\s,}])\.console-header\s*[,{]/.test(css), '.console-header rule').toBe(false);
    expect(/--hero-bar/.test(css), '--hero-bar offset').toBe(false);
  });

  it('gui/console.css declares .readout exactly once', () => {
    // The fifteen relocated footer readouts all wear `.readout`. A SECOND rule
    // of that name later in the same file silently wins every property the two
    // share — no error, no warning, just different pixels. Not hypothetical:
    // the stylesheet carried a dead "Readout pill" block from an earlier
    // design, and while it stood those fifteen lines rendered as a padded,
    // bordered, gradient-backed band across the top of the console body —
    // exactly the strip this slice deleted, back under another name.
    const css = readStripped(path.join(GUI, 'console.css'));
    const declarations = css.match(/(^|[\s,}])\.readout\s*\{/g) || [];
    expect(declarations.length, '.readout is declared more than once').toBe(1);
  });
});

describe('the readouts the footer carried moved into the console body', () => {
  for (const file of allConsoleDocuments()) {
    const name = rel(file);
    const doc = docFor(file);
    const badges = [...doc.querySelectorAll('[id$="-auto-badge"]')];
    const target = doc.getElementById('footer-target');
    if (badges.length === 0 && !target) continue;

    it(`${name} keeps them inside the group that owns them`, () => {
      // "The component group that owns the automation" — the control column on
      // Helm, the weapons column on Tactical, the Power column on Engineering.
      // Structurally: inside the console's content, never loose in the frame.
      const body = doc.querySelector('.console-body, .body');
      expect(body, `${name} has no console body`).not.toBeNull();
      for (const badge of badges) {
        expect(body.contains(badge), `#${badge.id} is outside the console body`).toBe(true);
      }
      if (target) {
        expect(body.contains(target), '#footer-target is outside the console body').toBe(true);
      }
    });
  }

  it('every target-bearing console retains a readout or target card', () => {
    // The renderers null-guard `ids.footer`, so a dropped one is silent — which
    // is fine where a dedicated target card owns those facts (Cruiser Tactical)
    // and a bug where removing the readout loses the console's only target.
    const withTarget = allConsoleDocuments()
      .filter((file) => docFor(file).querySelector('#footer-target, ph-target-lock-card'))
      .map(rel);
    expect(withTarget).toEqual([
      'gui/battleship/captain.html',
      'gui/battleship/comms.html',
      'gui/battleship/helm.html',
      'gui/battleship/sensors.html',
      'gui/battleship/tactical.html',
      'gui/cruiser/captain.html',
      'gui/cruiser/comms.html',
      'gui/cruiser/helm.html',
      'gui/cruiser/science.html',
      'gui/cruiser/tactical.html',
      'gui/destroyer/captain.html',
      'gui/destroyer/helm.html',
      'gui/destroyer/tactical.html',
    ]);
  });
});

describe('the shared segment control (issue #1374)', () => {
  // The shipped Captain TARGET | MISSION column and Engineering system
  // picker share this primitive and its roving keyboard contract.

  it('declares .seg and .seg-btn in the shared console stylesheet', () => {
    expect(CONSOLE_CSS).toMatch(/(^|\n)\.seg\s*\{/);
    expect(CONSOLE_CSS).toMatch(/(^|\n)\.seg-btn\s*\{/);
  });

  it('sizes a segment for a thumb', () => {
    const rule = CONSOLE_CSS.match(/(^|\n)\.seg-btn\s*\{([^}]*)\}/);
    expect(rule, 'no .seg-btn rule').not.toBeNull();
    expect(rule[2]).toMatch(/min-height:\s*var\(--control-hit-min\)/);
  });

  it('marks the chosen segment with aria-selected, not a class', () => {
    // One source of truth, and the one assistive technology reads. A `.active`
    // class beside it is the state that goes stale.
    expect(CONSOLE_CSS).toMatch(/\.seg-btn\[aria-selected="true"\]/);
    expect(CONSOLE_CSS).not.toMatch(/\.seg-btn\.active/);
  });

  it('documents the tablist contract the shell bar already uses', () => {
    // The role/aria shape is the whole reason gui/roving-tabindex.js can drive
    // arrow keys through a segment control with no code of its own.
    const comment = CONSOLE_CSS.match(/\/\*[^*]*Segment control[\s\S]*?\*\//);
    expect(comment, 'no .seg documentation block').not.toBeNull();
    expect(comment[0]).toMatch(/role="tablist"/);
    expect(comment[0]).toMatch(/role="tab"/);
    expect(comment[0]).toMatch(/aria-selected/);
  });
});

describe('the shell owns the damage popup the footers used to', () => {
  const CLIENT_HTML = fs.readFileSync(path.join(REPO_ROOT, 'client.html'), 'utf8');

  /**
   * The body of the "tapped the tab I am already on" branch of `onActivate`.
   * The inline script is not importable, so the contract is read off the
   * source; taking the whole branch rather than matching its first statement
   * means an assertion about one line cannot be satisfied by a different one.
   */
  function alreadySelectedBranch() {
    const onActivate = CLIENT_HTML.match(/onActivate:\s*\(tabId, kind\)\s*=>\s*\{[\s\S]*?\n        \},/);
    expect(onActivate, 'no onActivate handler').not.toBeNull();
    const branch = onActivate[0].match(
      /if \(tabId === activeConsole && activeConsoleOverlay\(\) === null\) \{\n([\s\S]*?)\n {10}\}/,
    );
    expect(branch, 'no already-selected-tab branch').not.toBeNull();
    return branch[1];
  }

  it('mounts the popup and the row-drawing component once, in the shell', () => {
    expect(CLIENT_HTML).toMatch(/id="station-damage-popup"/);
    expect(CLIENT_HTML).toMatch(/<ph-damage-detail id="station-damage-popup-detail">/);
    expect(CLIENT_HTML).toMatch(/src="gui\/station-damage-popup\.js"/);
    expect(CLIENT_HTML).toMatch(/src="gui\/components\/ph-damage-detail\.js"/);
  });

  it('opens it from a tap on the tab that is ALREADY selected', () => {
    // The decision itself: this seat's own tab, with no overlay covering it
    // (an open overlay IS the bar's selection, and that same tap means "come
    // back to the console").
    expect(alreadySelectedBranch()).toMatch(/showStationDamagePopup\(\);/);
  });

  it('still reports the visit on that tap, so the unread cue can clear', () => {
    // `unread` is edge-triggered host-side (src/station_importance.rs) on
    // whichever Station an objective ends on — including the one this seat is
    // already sitting at — and gui/hero-bar.js draws the cue on every tab.
    // Tapping your own tab is the only gesture that reports a visit for it, so
    // the popup branch has to send it too; without this the badge sticks until
    // the player switches Station and comes back.
    const branch = alreadySelectedBranch();
    expect(branch).toMatch(/send\('StationVisited', \{ station: tabId \}\);/);
    expect(branch).toMatch(/scheduleRender\(\);/);
  });

  it('closes it when the seat changes, so it cannot name a Station you left', () => {
    const setActive = CLIENT_HTML.match(/function setActiveConsole\(name\)\s*\{[\s\S]*?\n    \}/);
    expect(setActive, 'no setActiveConsole').not.toBeNull();
    expect(setActive[0]).toMatch(/dismissStationDamagePopup\(\)/);
  });

  it('keys the rows and the tabs by the SENDING FRAME, not by the claimed name', () => {
    // A console document says its own name and nothing else, and that name is
    // not a Station (see the cruiser case pinned below). Reading
    // `event.data.console` straight into the stores is the regression: it
    // collapses two seats sharing a document into one slot.
    const listener = CLIENT_HTML.match(
      /window\.addEventListener\('message', \(event\) => \{[\s\S]*?\n      if \(event\.data\.type !== 'console_action'\) return;/,
    );
    expect(listener, 'no message listener').not.toBeNull();
    for (const type of ['console_tabs', 'console_hull']) {
      const branch = listener[0].match(
        new RegExp(`if \\(event\\.data\\.type === '${type}'\\) \\{\\n([\\s\\S]*?)\\n        return;`),
      );
      expect(branch, `no ${type} branch`).not.toBeNull();
      expect(branch[1], `${type} keyed by the claim`)
        .not.toMatch(/const name = event\.data\.console/);
      expect(branch[1], `${type} does not resolve its sender`)
        .toMatch(/const name = senderStationId\(event\);/);
    }
  });
});

describe('two Stations sharing one console document (issue #1374)', () => {
  /**
   * The cruiser's `comms` and `navigation` Stations are both
   * `gui/cruiser/comms.html`, and that document calls itself 'comms'. Read the
   * pairs out of the ship rather than hand-writing them, so the day a hull
   * stops sharing a document this stops pretending to cover the case.
   */
  function cruiserStations() {
    const toml = parseToml(
      fs.readFileSync(path.join(REPO_ROOT, 'assets/entities/alliance_cruiser.toml'), 'utf8'),
    );
    return (toml.station || []).map(st => ({ id: st.id, console: st.console }));
  }

  it('the cruiser really does mount one document at two Stations', () => {
    // A guard on the fixture: without a genuine collision the test below would
    // pass over nothing.
    const shared = cruiserStations().filter(st => st.console === 'gui/cruiser/comms.html');
    expect(shared.map(st => st.id).sort()).toEqual(['comms', 'navigation']);
  });

  it('resolves each mounted iframe to its OWN Station, so they cannot share a key', () => {
    // The real mount seam and the real resolver — this is what client.html
    // calls. Both frames of gui/cruiser/comms.html would post
    // `{console:'comms'}`; the shell has to answer 'comms' for one and
    // 'navigation' for the other, or the Navigation seat's `own_hull` rows
    // overwrite the Comms seat's and the NAV tab's popup is never populated.
    const dom = new JSDOM('<div id="console-container"></div>', { url: 'https://phoenix.test/' });
    const doc = dom.window.document;
    const shipStations = { stations: cruiserStations() };
    mountConsoles(doc, doc.getElementById('console-container'), shipStations);

    const commsWindow = doc.getElementById('comms-iframe').contentWindow;
    const navWindow = doc.getElementById('navigation-iframe').contentWindow;
    expect(commsWindow).not.toBe(navWindow);
    expect(stationIdForSource(doc, shipStations, commsWindow, 'comms')).toBe('comms');
    expect(stationIdForSource(doc, shipStations, navWindow, 'comms')).toBe('navigation');
  });

  it('falls back to the claimed name only when no mounted frame sent it', () => {
    // The paths with no iframe behind them (the native host, BroadcastChannel).
    const dom = new JSDOM('<div id="console-container"></div>', { url: 'https://phoenix.test/' });
    const doc = dom.window.document;
    const shipStations = { stations: cruiserStations() };
    mountConsoles(doc, doc.getElementById('console-container'), shipStations);
    expect(stationIdForSource(doc, shipStations, null, 'helm')).toBe('helm');
    expect(stationIdForSource(doc, shipStations, dom.window, 'helm')).toBe('helm');
    expect(stationIdForSource(doc, shipStations, dom.window, undefined)).toBe('');
  });
});
