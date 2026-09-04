// @vitest-environment jsdom
//
// gui/host-landing-render.js — the landing screen's shared renderer
// (issue #1360).
//
// The claim under test is the one that makes "one landing" true rather than
// aspirational: the renderer takes `doc` first, so a SECOND surface (#1361's
// native viewscreen) can hand it a different document and get the same
// landing. Two things follow, and both are exercised here:
//
//   - the markup is server.html's OWN, read off disk, for the reason
//     tests/client/host-scenario-render.test.js reads it: the renderer writes
//     into element ids, and ids asserted against a hand-built stub prove
//     nothing about the document either surface actually shows;
//
//   - a DELIBERATELY INCOMPLETE document must not throw. That is the whole
//     two-document contract — the native lobby document carries a trimmed
//     subset of this markup, and a branch whose element is absent has to do
//     nothing rather than abandon the rest of the render half-written.
//
// The view models are driven through `landingViewModel` wherever the stage is
// the point, rather than through hand-written objects that could drift from
// what either caller passes.

import { describe, it, expect, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  renderHostLanding,
  undockPicker,
  LANDING_ENTRY_CLASS,
  LANDING_ENTRY_SELECTOR,
  LANDING_ENTRY_ATTR,
  PICKER_DOCKED_CLASS,
} from '../../gui/host-landing-render.js';
import { landingViewModel } from '../../gui/host-landing-view.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = fs.readFileSync(path.join(HERE, '../../server.html'), 'utf-8');

/** The string resolver both surfaces inject; here it just echoes the id. */
const t = (id, params) => (params ? `${id}:${JSON.stringify(params)}` : id);

/**
 * server.html's real landing and picker markup, in a fresh document.
 *
 * Both, because the middle column's whole job in this slice is to hold the
 * live `#scenario-panel` node — a landing without the picker beside it could
 * not test the one action that works.
 */
function landingDoc() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const landing = parsed.getElementById('landing-panel');
  const picker = parsed.getElementById('scenario-panel');
  if (!landing) throw new Error('#landing-panel not found in server.html');
  if (!picker) throw new Error('#scenario-panel not found in server.html');
  const doc = document.implementation.createHTMLDocument('');
  doc.body.appendChild(doc.importNode(landing, true));
  doc.body.appendChild(doc.importNode(picker, true));
  return doc;
}

/** A recording hook set — what each surface supplies in its own way. */
function hooks() {
  const calls = { pick: [], fullscreen: 0 };
  return [
    {
      pick: (id) => calls.pick.push(id),
      toggleFullscreen: () => { calls.fullscreen += 1; },
    },
    calls,
  ];
}

/**
 * The same markup, installed in the WINDOW's own document.
 *
 * Focus is the one claim `landingDoc()` cannot express: jsdom only tracks
 * `activeElement` for a document with a browsing context, and
 * `document.implementation.createHTMLDocument()` has none — `focus()` there is
 * a silent no-op and `activeElement` never leaves `<body>`. The focus cases
 * therefore drive the ambient document, which is also the shape both real
 * surfaces hand the renderer.
 */
function landingInWindow() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  document.body.innerHTML = '';
  document.body.appendChild(document.importNode(parsed.getElementById('landing-panel'), true));
  document.body.appendChild(document.importNode(parsed.getElementById('scenario-panel'), true));
  return document;
}

const entries = (doc) => Array.from(doc.querySelectorAll(LANDING_ENTRY_SELECTOR));
const text = (doc, id) => doc.getElementById(id).textContent;

describe('renderHostLanding — the idle landing', () => {
  it('writes the identity, the platform and the build into the document', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ build: 'abc1234' }), t);
    expect(text(doc, 'landing-title')).toBe('server.landing.title');
    expect(text(doc, 'landing-tagline')).toBe('server.landing.tagline');
    expect(text(doc, 'landing-platform')).toBe('server.landing.platform_web');
    expect(text(doc, 'landing-status-build')).toBe('server.landing.build:{"build":"abc1234"}');
    expect(doc.getElementById('landing-logo').getAttribute('aria-label'))
      .toBe('server.landing.logo_alt');
  });

  it('marks the root idle and hands the stylesheet a depth of zero', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    const root = doc.getElementById('landing-panel');
    expect(root.classList.contains('is-idle')).toBe(true);
    expect(root.classList.contains('is-open')).toBe(false);
    expect(root.dataset.landingStage).toBe('idle');
    expect(root.style.getPropertyValue('--landing-depth')).toBe('0');
  });

  it('builds one button per row, numbered, labelled and described', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    const list = entries(doc);
    expect(list.map((el) => el.getAttribute(LANDING_ENTRY_ATTR))).toEqual([
      'new_game', 'load_game', 'join_peer', 'connect_host', 'load_mod_pack',
    ]);
    expect(list[0].querySelector('.landing-mi-ix').textContent).toBe('01');
    expect(list[0].querySelector('.landing-mi-label').textContent).toBe('server.landing.new_game');
    expect(list[0].querySelector('.landing-mi-desc').textContent).toBe('server.landing.new_game_desc');
    for (const el of list) expect(el.classList.contains(LANDING_ENTRY_CLASS)).toBe(true);
  });

  it('marks the four entries with no stage aria-disabled rather than disabled', () => {
    // Focusable and readable: "here, but not yet" is more honest than a control
    // a screen reader cannot reach at all.
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    const list = entries(doc);
    expect(list.filter((el) => el.getAttribute('aria-disabled') === 'true')
      .map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .toEqual(['load_game', 'join_peer', 'connect_host', 'load_mod_pack']);
    expect(list.every((el) => el.disabled === false)).toBe(true);
  });

  it('rebuilds its own entries and nothing else, however often it runs', () => {
    const doc = landingDoc();
    const menu = doc.getElementById('landing-menu');
    // Something this renderer does not own, in the container it rebuilds.
    const foreign = doc.createElement('span');
    foreign.id = 'not-ours';
    menu.appendChild(foreign);

    renderHostLanding(doc, landingViewModel(), t);
    renderHostLanding(doc, landingViewModel(), t);
    renderHostLanding(doc, landingViewModel(), t);

    expect(entries(doc)).toHaveLength(5);
    expect(doc.getElementById('not-ours')).not.toBe(null);
  });

  it('reports every click, including the inert entries, and judges none itself', () => {
    // Whether an entry does anything is nextOpenEntry's decision. A second
    // judgement here would be a place for the two to disagree.
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, landingViewModel(), t, h);
    entries(doc).forEach((el) => el.click());
    expect(calls.pick).toEqual([
      'new_game', 'load_game', 'join_peer', 'connect_host', 'load_mod_pack',
    ]);
  });

  it('binds the fullscreen control once per render, not once more each time', () => {
    // The button is static markup and this function runs on every menu click,
    // so an accumulated listener would toggle fullscreen N times on click N.
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, landingViewModel(), t, h);
    renderHostLanding(doc, landingViewModel(), t, h);
    renderHostLanding(doc, landingViewModel(), t, h);
    doc.getElementById('landing-fullscreen-btn').click();
    expect(calls.fullscreen).toBe(1);
  });
});

describe('renderHostLanding — New Game', () => {
  const openVm = () => landingViewModel({ openEntryId: 'new_game' });

  it('marks the root open, one step deep, on the world-picker stage', () => {
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t);
    const root = doc.getElementById('landing-panel');
    expect(root.classList.contains('is-open')).toBe(true);
    expect(root.classList.contains('is-idle')).toBe(false);
    expect(root.dataset.landingStage).toBe('world-picker');
    expect(root.style.getPropertyValue('--landing-depth')).toBe('1');
  });

  it('marks New Game selected and expanded, and nothing else', () => {
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t);
    const on = entries(doc).filter((el) => el.classList.contains('on'));
    expect(on.map((el) => el.getAttribute(LANDING_ENTRY_ATTR))).toEqual(['new_game']);
    expect(on[0].getAttribute('aria-expanded')).toBe('true');
  });

  it('reveals the EXISTING picker by moving it into the middle column', () => {
    // Not a re-render of the world list: the live node moves, so a pick still
    // reaches driveWorldLoad() down the path it always did, and the mod-pack
    // upload and save importer inside it travel with it.
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t);
    const picker = doc.getElementById('scenario-panel');
    expect(picker.parentElement).toBe(doc.getElementById('landing-mid'));
    expect(picker.classList.contains(PICKER_DOCKED_CLASS)).toBe(true);
    // The whole picker came, not a copy of part of it.
    expect(picker.querySelector('#world-list')).not.toBe(null);
    expect(picker.querySelector('#mod-pack-btn')).not.toBe(null);
  });

  it('puts the picker back on the body when the menu closes again', () => {
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t);
    renderHostLanding(doc, landingViewModel(), t);
    const picker = doc.getElementById('scenario-panel');
    expect(picker.parentElement).toBe(doc.body);
    expect(picker.classList.contains(PICKER_DOCKED_CLASS)).toBe(false);
  });

  it('is idempotent — re-rendering the open stage does not re-move the node', () => {
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t);
    const picker = doc.getElementById('scenario-panel');
    renderHostLanding(doc, openVm(), t);
    expect(doc.getElementById('scenario-panel')).toBe(picker);
    expect(doc.getElementById('landing-mid').children).toHaveLength(1);
  });

  it('leaves the picker alone when the surface says it owns it elsewhere', () => {
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t, null, { dockPicker: false });
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
  });
});

describe('undockPicker', () => {
  it('returns the picker to the body whatever the landing is doing', () => {
    // The page-lifecycle escape hatch: driveWorldLoad hides the landing
    // outright, and a `display` set on a node inside a hidden ancestor shows
    // nothing at all when round two re-shows the picker.
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t);
    undockPicker(doc);
    const picker = doc.getElementById('scenario-panel');
    expect(picker.parentElement).toBe(doc.body);
    expect(picker.classList.contains(PICKER_DOCKED_CLASS)).toBe(false);
  });

  it('does nothing at all on a document with no picker', () => {
    const doc = document.implementation.createHTMLDocument('');
    expect(() => undockPicker(doc)).not.toThrow();
  });
});

describe('renderHostLanding — keyboard focus across the rebuild', () => {
  // The menu is rebuilt from scratch on every call and the page re-renders on
  // every click, so activating an entry deletes the node the operator is
  // standing on. On a pointer that is invisible; on a keyboard or a gamepad it
  // is the front door dropping focus to `<body>` and restarting the tab order
  // at the top of the document — on the one surface a host is most likely to
  // drive entirely by keys, before any pointer-friendly stage is open.
  afterEach(() => { document.body.innerHTML = ''; });

  it('keeps focus on the entry that was activated', () => {
    const doc = landingInWindow();
    renderHostLanding(doc, landingViewModel(), t, {});
    doc.querySelector(`[${LANDING_ENTRY_ATTR}="new_game"]`).focus();
    expect(doc.activeElement.getAttribute(LANDING_ENTRY_ATTR)).toBe('new_game');

    // What a click does: the caller re-renders on the new stage, which is the
    // render that deletes the button under the operator.
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});

    expect(doc.activeElement.getAttribute(LANDING_ENTRY_ATTR)).toBe('new_game');
    // ...and it is the entry the rebuild CREATED, not a detached node that
    // happens to still answer to `focus()`.
    expect(doc.getElementById('landing-menu').contains(doc.activeElement)).toBe(true);
  });

  it('keeps it on an inert entry too, when a caller re-renders anyway', () => {
    // server.html skips the render for an entry that changes nothing, but that
    // is the page's economy and not this renderer's contract: a surface that
    // re-renders unconditionally must not cost its operator their place.
    const doc = landingInWindow();
    renderHostLanding(doc, landingViewModel(), t, {});
    doc.querySelector(`[${LANDING_ENTRY_ATTR}="load_game"]`).focus();
    renderHostLanding(doc, landingViewModel(), t, {});
    expect(doc.activeElement.getAttribute(LANDING_ENTRY_ATTR)).toBe('load_game');
  });

  it('does not pull focus back from wherever the operator has moved on to', () => {
    // The restore is conditioned on focus having fallen to `<body>`, which is
    // the rebuild's own signature. A render that lands while the operator is
    // elsewhere — the docked picker, the corner control, the settings overlay
    // — must leave them there.
    const doc = landingInWindow();
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});
    const elsewhere = doc.getElementById('mod-pack-btn');
    elsewhere.focus();
    expect(doc.activeElement).toBe(elsewhere);
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});
    expect(doc.activeElement).toBe(elsewhere);
  });

  it('focuses nothing when nobody was on the menu to begin with', () => {
    const doc = landingInWindow();
    renderHostLanding(doc, landingViewModel(), t, {});
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});
    expect(doc.activeElement).toBe(doc.body);
  });
});

describe('a deliberately incomplete document', () => {
  /**
   * Everything the renderer touches, so each case can remove exactly one.
   *
   * The list is spelled out rather than derived, because it is the CONTRACT:
   * a surface may carry any subset of these, and the day the renderer starts
   * writing an id that is not here, this suite still covers the old ones and
   * the new one earns its own line.
   */
  const TOUCHED = [
    'landing-panel', 'landing-title', 'landing-tagline', 'landing-platform',
    'landing-logo', 'landing-status-platform', 'landing-status-session',
    'landing-status-build', 'landing-fullscreen-btn', 'landing-menu',
    'landing-mid', 'scenario-panel',
  ];

  for (const id of TOUCHED) {
    it(`survives a document with no #${id}`, () => {
      const doc = landingDoc();
      doc.getElementById(id).remove();
      expect(() => {
        renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});
        renderHostLanding(doc, landingViewModel(), t, {});
      }).not.toThrow();
    });
  }

  it('renders into an EMPTY document without throwing, and writes nothing', () => {
    // The extreme of the same contract, and the shape a caller reaches when a
    // surface has not built its markup yet.
    const doc = document.implementation.createHTMLDocument('');
    expect(() => renderHostLanding(doc, landingViewModel(), t, {})).not.toThrow();
    expect(doc.body.children).toHaveLength(0);
  });

  it('still writes everything it CAN when one element is missing', () => {
    // The failure this guards is the worse one: not a throw, but a render that
    // stops at the first hole and leaves the rest of the surface stale.
    const doc = landingDoc();
    doc.getElementById('landing-title').remove();
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});
    expect(entries(doc)).toHaveLength(5);
    expect(doc.getElementById('scenario-panel').parentElement)
      .toBe(doc.getElementById('landing-mid'));
    expect(text(doc, 'landing-status-session')).toBe('server.landing.status_hosting');
  });

  it('renders with no hooks at all — the controls simply do nothing', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    expect(() => {
      entries(doc).forEach((el) => el.click());
      doc.getElementById('landing-fullscreen-btn').click();
    }).not.toThrow();
  });
});
