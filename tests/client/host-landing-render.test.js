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
  undockLandingPanels,
  LANDING_ENTRY_CLASS,
  LANDING_ENTRY_SELECTOR,
  LANDING_ENTRY_ATTR,
  LANDING_DOCKED_CLASS,
} from '../../gui/host-landing-render.js';
import { landingViewModel } from '../../gui/host-landing-view.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = fs.readFileSync(path.join(HERE, '../../server.html'), 'utf-8');

/** The string resolver both surfaces inject; here it just echoes the id. */
const t = (id, params) => (params ? `${id}:${JSON.stringify(params)}` : id);

/**
 * server.html's real landing, picker and save-catalogue markup, in a fresh
 * document.
 *
 * All three, because the middle column's whole job is to hold ONE of the two
 * live panels the menu's staged rows borrow — `#scenario-panel` for New Game
 * (issue #1362) and `#save-slots-panel` for Load Game (issue #1363). A landing
 * without both beside it could not test that opening one puts the other back.
 */
function landingDoc() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const landing = parsed.getElementById('landing-panel');
  const picker = parsed.getElementById('scenario-panel');
  const saves = parsed.getElementById('save-slots-panel');
  if (!landing) throw new Error('#landing-panel not found in server.html');
  if (!picker) throw new Error('#scenario-panel not found in server.html');
  if (!saves) throw new Error('#save-slots-panel not found in server.html');
  const doc = document.implementation.createHTMLDocument('');
  doc.body.appendChild(doc.importNode(landing, true));
  doc.body.appendChild(doc.importNode(picker, true));
  doc.body.appendChild(doc.importNode(saves, true));
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

  it('marks the entries with no stage aria-disabled rather than disabled', () => {
    // Focusable and readable: "here, but not yet" is more honest than a control
    // a screen reader cannot reach at all.
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    const list = entries(doc);
    expect(list.filter((el) => el.getAttribute('aria-disabled') === 'true')
      .map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .toEqual(['join_peer', 'connect_host', 'load_mod_pack']);
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
    expect(picker.classList.contains(LANDING_DOCKED_CLASS)).toBe(true);
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
    expect(picker.classList.contains(LANDING_DOCKED_CLASS)).toBe(false);
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
    renderHostLanding(doc, openVm(), t, null, { dockPanels: false });
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
  });
});

describe('renderHostLanding — one rung deeper (issue #1362)', () => {
  // Choosing a World reveals the hull column BESIDE the World list. This
  // renderer's whole part in that is two writes — a second class and a larger
  // depth — because which columns those reveal, and whether the track slides
  // at all, is gui/host-landing.css's business at each breakpoint.
  const openVm = () => landingViewModel({ openEntryId: 'new_game' });
  const deepVm = () => landingViewModel({ openEntryId: 'new_game', deepStage: 'ship-picker' });

  it('marks the root deep as well as open, two columns along', () => {
    const doc = landingDoc();
    renderHostLanding(doc, deepVm(), t);
    const root = doc.getElementById('landing-panel');
    expect(root.classList.contains('is-open')).toBe(true);
    expect(root.classList.contains('is-deep')).toBe(true);
    expect(root.classList.contains('is-idle')).toBe(false);
    expect(root.dataset.landingStage).toBe('ship-picker');
    expect(root.style.getPropertyValue('--landing-depth')).toBe('2');
  });

  it('takes `is-deep` off again on the way back to the World list', () => {
    // The failure this catches is the sticky one: a class added by a deeper
    // stage and never removed leaves the hull column on screen over a picker
    // that has already gone back to worlds.
    const doc = landingDoc();
    const root = doc.getElementById('landing-panel');
    renderHostLanding(doc, deepVm(), t);
    renderHostLanding(doc, openVm(), t);
    expect(root.classList.contains('is-deep')).toBe(false);
    expect(root.classList.contains('is-open')).toBe(true);
    expect(root.style.getPropertyValue('--landing-depth')).toBe('1');

    renderHostLanding(doc, landingViewModel(), t);
    expect(root.classList.contains('is-deep')).toBe(false);
    expect(root.classList.contains('is-open')).toBe(false);
    expect(root.classList.contains('is-idle')).toBe(true);
  });

  it('KEEPS the picker docked through the deeper rung', () => {
    // The load-bearing one. The dock used to be conditioned on the stage being
    // called 'world-picker', so the hull stage would have undocked
    // `#scenario-panel` — carrying the World list off the screen at the exact
    // moment the operator needs it to step back along. It reads a field on the
    // row instead.
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t);
    renderHostLanding(doc, deepVm(), t);
    const picker = doc.getElementById('scenario-panel');
    expect(picker.parentElement).toBe(doc.getElementById('landing-mid'));
    expect(picker.classList.contains(LANDING_DOCKED_CLASS)).toBe(true);
    expect(picker.querySelector('#world-list')).not.toBe(null);
  });

  it('docks it when the deeper rung is the FIRST thing rendered', () => {
    // A render that arrives with a World already locked — the module island's
    // first call after a pick came in from a phone.
    const doc = landingDoc();
    renderHostLanding(doc, deepVm(), t);
    expect(doc.getElementById('scenario-panel').parentElement)
      .toBe(doc.getElementById('landing-mid'));
  });

  it('makes the receding menu INACTIVE, not merely faint', () => {
    // gui/host-landing.css dims this column to 42% one rung deep. Dimmed LIVE
    // controls would be text far under the contrast floor the same sheet holds
    // these labels to at rest; the floor's one exemption is text in an
    // inactive component, so the column is made genuinely inactive — the sheet
    // drops the pointer, this drops the keyboard. Back is the way out of a
    // deep stage, so nothing an operator needs goes with it.
    const doc = landingDoc();
    const menu = doc.getElementById('landing-menu');
    renderHostLanding(doc, deepVm(), t);
    expect(menu.hasAttribute('inert')).toBe(true);
  });

  it('gives the menu back on the way out of the deep stage', () => {
    // The sticky-attribute failure, the exact sibling of the `is-deep` one
    // above: a menu left inert after a Back is a front door nothing can open.
    const doc = landingDoc();
    const menu = doc.getElementById('landing-menu');
    renderHostLanding(doc, deepVm(), t);
    renderHostLanding(doc, openVm(), t);
    expect(menu.hasAttribute('inert')).toBe(false);
    renderHostLanding(doc, landingViewModel(), t);
    expect(menu.hasAttribute('inert')).toBe(false);
  });

  it('leaves the hull column to the picker`s own renderer', () => {
    // `#landing-ship` is a column this renderer draws nothing into: what goes
    // in it is the ph-ship-picker gui/host-scenario-render.js mounts, so there
    // is one implementation of a hull card and not two.
    const doc = landingDoc();
    renderHostLanding(doc, deepVm(), t);
    expect(doc.getElementById('ship-list').children).toHaveLength(0);
  });
});

describe('undockLandingPanels', () => {
  it('returns the picker to the body whatever the landing is doing', () => {
    // The page-lifecycle escape hatch: driveWorldLoad hides the landing
    // outright, and a `display` set on a node inside a hidden ancestor shows
    // nothing at all when round two re-shows the picker.
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t);
    undockLandingPanels(doc);
    const picker = doc.getElementById('scenario-panel');
    expect(picker.parentElement).toBe(doc.body);
    expect(picker.classList.contains(LANDING_DOCKED_CLASS)).toBe(false);
  });

  it('returns the save catalogue too, without being told it exists', () => {
    // Found by the docked CLASS rather than by id, which is what stops a row
    // that borrows a third panel having to remember this function (#1363).
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ openEntryId: 'load_game' }), t);
    undockLandingPanels(doc);
    const saves = doc.getElementById('save-slots-panel');
    expect(saves.parentElement).toBe(doc.body);
    expect(saves.classList.contains(LANDING_DOCKED_CLASS)).toBe(false);
  });

  it('does nothing at all on a document with no landing column', () => {
    const doc = document.implementation.createHTMLDocument('');
    expect(() => undockLandingPanels(doc)).not.toThrow();
  });
});

describe('renderHostLanding — Load Game (issue #1363)', () => {
  const loadVm = () => landingViewModel({ openEntryId: 'load_game' });
  const newVm = () => landingViewModel({ openEntryId: 'new_game' });

  it('reveals the EXISTING save catalogue by moving it into the middle column', () => {
    // Not a re-render of the rows: the live node moves, so a Start still
    // reaches the resume path down the wire it always did — and the save
    // importer travels with it, because mountSaveSlots put it in this panel's
    // header rather than leaving it a block of the boot panel.
    const doc = landingDoc();
    renderHostLanding(doc, loadVm(), t);
    const saves = doc.getElementById('save-slots-panel');
    expect(saves.parentElement).toBe(doc.getElementById('landing-mid'));
    expect(saves.classList.contains(LANDING_DOCKED_CLASS)).toBe(true);
    const root = doc.getElementById('landing-panel');
    expect(root.dataset.landingStage).toBe('save-catalogue');
    expect(root.classList.contains('is-open')).toBe(true);
  });

  it('swaps the two borrowed panels rather than stacking them', () => {
    // One column, one open stage. The failure this catches is the obvious one:
    // an undock written per-id would have left the picker parented in the
    // column with the catalogue on top of it.
    const doc = landingDoc();
    const mid = doc.getElementById('landing-mid');
    renderHostLanding(doc, newVm(), t);
    renderHostLanding(doc, loadVm(), t);
    expect(mid.children).toHaveLength(1);
    expect(doc.getElementById('save-slots-panel').parentElement).toBe(mid);
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
    expect(doc.getElementById('scenario-panel').classList.contains(LANDING_DOCKED_CLASS))
      .toBe(false);

    renderHostLanding(doc, newVm(), t);
    expect(mid.children).toHaveLength(1);
    expect(doc.getElementById('scenario-panel').parentElement).toBe(mid);
    expect(doc.getElementById('save-slots-panel').parentElement).toBe(doc.body);
  });

  it('puts the catalogue back on the body when the menu closes again', () => {
    const doc = landingDoc();
    renderHostLanding(doc, loadVm(), t);
    renderHostLanding(doc, landingViewModel(), t);
    const saves = doc.getElementById('save-slots-panel');
    expect(saves.parentElement).toBe(doc.body);
    expect(saves.classList.contains(LANDING_DOCKED_CLASS)).toBe(false);
  });

  it('is idempotent — re-rendering the open stage does not re-move the node', () => {
    const doc = landingDoc();
    renderHostLanding(doc, loadVm(), t);
    const saves = doc.getElementById('save-slots-panel');
    renderHostLanding(doc, loadVm(), t);
    expect(doc.getElementById('save-slots-panel')).toBe(saves);
    expect(doc.getElementById('landing-mid').children).toHaveLength(1);
  });

  it('does nothing on a document that carries no catalogue at all', () => {
    // The two-document contract: a surface may hand this a trimmed subset of
    // the markup, and a stage whose panel is absent must leave the rest of the
    // render intact rather than throw half-way through it.
    const doc = landingDoc();
    doc.getElementById('save-slots-panel').remove();
    expect(() => renderHostLanding(doc, loadVm(), t)).not.toThrow();
    expect(doc.getElementById('landing-panel').dataset.landingStage).toBe('save-catalogue');
    expect(doc.getElementById('landing-mid').children).toHaveLength(0);
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

describe('renderHostLanding — the native viewscreen (issue #1361)', () => {
  // The claim #1360 made and this slice cashes: `doc` is the first argument, so
  // a SECOND surface hands it a different document and gets the same landing.
  // What differs between the two surfaces is stated here in full — a trimmed
  // document, a curated entry list, and who owns the panel's visibility — and
  // nothing else, because anything else would be a second landing.

  /**
   * The native lobby document's landing, as `document.rs` assembles it: the
   * page's own markup minus the fullscreen control it cannot answer.
   *
   * Built by DELETING from server.html's markup rather than by hand, so this
   * suite cannot drift into testing a landing neither surface shows.
   */
  function nativeLandingDoc() {
    const doc = landingDoc();
    doc.getElementById('landing-fullscreen-btn').remove();
    doc.getElementById('landing-panel').style.display = 'none';
    return doc;
  }

  /** What `host_lobby_link.js` passes: platform, build, and who owns the panel. */
  const nativeVm = (extra) => landingViewModel(Object.assign(
    { platform: 'native', build: '0.1.0' },
    extra,
  ));

  it('draws the same landing into a document with the fullscreen control gone', () => {
    const doc = nativeLandingDoc();
    expect(() => {
      renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    }).not.toThrow();
    expect(text(doc, 'landing-title')).toBe('server.landing.title');
    expect(text(doc, 'landing-platform')).toBe('server.landing.platform_native');
    expect(text(doc, 'landing-status-build')).toBe('server.landing.build:{"build":"0.1.0"}');
  });

  it('does not offer Connect to Host, which a native host has no leg to answer', () => {
    // The doctrine, and it is enforced by the entry table rather than by
    // anything here: a control exists exactly when something behind it can
    // answer it, and a native host is always a host.
    const doc = nativeLandingDoc();
    renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    const ids = entries(doc).map((el) => el.getAttribute(LANDING_ENTRY_ATTR));
    expect(ids).not.toContain('connect_host');
    // Load Game IS still here, and inert: `platforms` says what a surface does
    // not offer, `stagePlatforms` says what it cannot open yet, and #1363's
    // AC5 is the second of those. See the row, and the test below.
    expect(ids).toEqual(['new_game', 'load_game', 'join_peer', 'load_mod_pack']);
    // …and the same document on the web surface still offers it, so this is a
    // curated menu and not a lost row.
    const web = landingDoc();
    renderHostLanding(web, landingViewModel(), t, {});
    expect(entries(web).map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .toContain('connect_host');
  });

  it('does not offer Exit to Desktop either, which is #1365 and not built here', () => {
    const doc = nativeLandingDoc();
    renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    expect(entries(doc).map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .not.toContain('exit');
  });

  it('shows the panel the first push reveals it with, and hides it when dismissed', () => {
    // The native document assembles #landing-panel at `display: none` so a host
    // given --world never flashes a front door it walked through at the prompt.
    // The first push is what opens it, and the push that says a World has
    // landed is what takes it away — the crew lobby underneath sits at z-index
    // 180, so a landing left up would cover the thing the room is watching.
    const doc = nativeLandingDoc();
    const panel = doc.getElementById('landing-panel');
    expect(panel.style.display).toBe('none');

    renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    expect(panel.style.display).toBe('');

    renderHostLanding(doc, nativeVm({ dismissed: true }), t, {}, { ownPanelVisibility: true });
    expect(panel.style.display).toBe('none');
  });

  it('leaves the panel alone for a surface that owns its own lifecycle', () => {
    // server.html passes nothing and keeps hideLanding() — the option is what
    // stops this renderer becoming a second authority on that page.
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ dismissed: true }), t, {});
    expect(doc.getElementById('landing-panel').style.display).toBe('');
  });

  it('reveals the picker into the middle column on the native document too', () => {
    // New Game moves the LIVE #scenario-panel, on this surface as on the page,
    // so a pick reaches the native world-load path down exactly the wire the
    // picker already used.
    const doc = nativeLandingDoc();
    renderHostLanding(
      doc,
      nativeVm({ openEntryId: 'new_game' }),
      t,
      {},
      { ownPanelVisibility: true },
    );
    const picker = doc.getElementById('scenario-panel');
    expect(picker.parentElement).toBe(doc.getElementById('landing-mid'));
    expect(picker.classList.contains(LANDING_DOCKED_CLASS)).toBe(true);
    expect(picker.querySelector('#world-list')).not.toBe(null);
  });

  it('takes the picker back out of a landing it is about to hide', () => {
    // The bug `undockPicker` exists for, reached the other way: a `display` set
    // on a node parented inside a hidden ancestor shows nothing at all, so a
    // dismissed landing must not still be holding the picker.
    const doc = nativeLandingDoc();
    const opts = { ownPanelVisibility: true };
    renderHostLanding(doc, nativeVm({ openEntryId: 'new_game' }), t, {}, opts);
    renderHostLanding(doc, nativeVm({ openEntryId: 'new_game', dismissed: true }), t, {}, opts);
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
    expect(doc.getElementById('landing-panel').style.display).toBe('none');
  });

  it('carries a menu press back through the hook, and nowhere else', () => {
    const doc = nativeLandingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, nativeVm(), t, h, { ownPanelVisibility: true });
    doc.querySelector(`[${LANDING_ENTRY_ATTR}="new_game"]`).click();
    expect(calls.pick).toEqual(['new_game']);
    // Inert entries report too: whether a press does anything is the view
    // model's decision, never a second judgement made in the renderer. Load
    // Game is the interesting one on this surface — a row with a stage the
    // WEB host opens, which this one cannot yet (#1363's AC5).
    doc.querySelector(`[${LANDING_ENTRY_ATTR}="load_game"]`).click();
    expect(calls.pick).toEqual(['new_game', 'load_game']);
  });

  it('draws Load Game as a not-yet row rather than dropping it (#1363 AC5)', () => {
    // The gap is recorded, not hidden. A menu that dropped the row would have
    // read as "native hosts do not load games", which is not what is true: the
    // route belongs here and the surface cannot serve it yet, so it renders
    // dashed and aria-disabled beside the other rows waiting on a slice.
    const doc = nativeLandingDoc();
    renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    const disabled = entries(doc)
      .filter((el) => el.getAttribute('aria-disabled') === 'true')
      .map((el) => el.getAttribute(LANDING_ENTRY_ATTR));
    expect(disabled).toEqual(['load_game', 'join_peer', 'load_mod_pack']);
    // ...and pressing it opens no middle column, which is the failure the row
    // being absent was avoiding in the first place.
    renderHostLanding(
      doc, nativeVm({ openEntryId: 'load_game' }), t, {}, { ownPanelVisibility: true },
    );
    expect(doc.getElementById('landing-mid').children).toHaveLength(0);
    expect(doc.getElementById('save-slots-panel').parentElement).toBe(doc.body);
    expect(doc.getElementById('landing-panel').dataset.landingStage).toBe('idle');
    // The same row on the web host does open it, so this is a surface saying
    // "not yet" and not a row that was never wired.
    const web = landingDoc();
    renderHostLanding(web, landingViewModel({ openEntryId: 'load_game' }), t);
    expect(web.getElementById('save-slots-panel').parentElement)
      .toBe(web.getElementById('landing-mid'));
  });

  it('survives a document with no landing at all, panel ownership and all', () => {
    // The two-document contract at its extreme: a surface may hand this a
    // document that carries none of the markup, and every write is guarded.
    const doc = document.implementation.createHTMLDocument('');
    expect(() => {
      renderHostLanding(doc, nativeVm({ dismissed: true }), t, {}, { ownPanelVisibility: true });
    }).not.toThrow();
  });
});
