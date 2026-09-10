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
  CONFIRM_DANGER_CLASS,
} from '../../gui/host-landing-render.js';
import { CONFIRM_CANCEL_ID, landingViewModel } from '../../gui/host-landing-view.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = fs.readFileSync(path.join(HERE, '../../server.html'), 'utf-8');

/** The string resolver both surfaces inject; here it just echoes the id. */
const t = (id, params) => (params ? `${id}:${JSON.stringify(params)}` : id);

/**
 * server.html's real landing, picker, save-catalogue and join markup, in a
 * fresh document.
 *
 * All four, because the middle column's whole job is to hold ONE of the live
 * panels the menu's staged rows borrow — `#scenario-panel` for New Game (issue
 * #1362), `#save-slots-panel` for Load Game (issue #1363) and
 * `#landing-join-panel` for both join routes (issue #1364). A landing without
 * them beside it could not test that opening one puts the others back.
 */
function landingDoc() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const doc = document.implementation.createHTMLDocument('');
  for (const id of ['landing-panel', 'scenario-panel', 'save-slots-panel', 'landing-join-panel']) {
    const node = parsed.getElementById(id);
    if (!node) throw new Error(`#${id} not found in server.html`);
    doc.body.appendChild(doc.importNode(node, true));
  }
  return doc;
}

/** A recording hook set — what each surface supplies in its own way. */
function hooks() {
  const calls = { pick: [], confirm: [], fullscreen: 0, join: [] };
  return [
    {
      pick: (id) => calls.pick.push(id),
      confirm: (action) => calls.confirm.push(action),
      toggleFullscreen: () => { calls.fullscreen += 1; },
      submitJoin: (join, typed) => calls.join.push([join.action, typed]),
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

/**
 * The panels BORROWED into the middle column — which is not the same thing as
 * that column's children.
 *
 * The column has tenants of its own since issues #1365 and #1366:
 * `#landing-confirm` and `#landing-packs` are markup that LIVES there and is
 * shown or hidden in place, never moved. So "one open stage at a time" is a
 * claim about the panels this renderer MOVES, and it is asked the way
 * `undockLandingPanels` asks it — by the docked class — rather than by counting
 * everything parented in the column.
 */
const docked = (doc) => Array.from(
  doc.getElementById('landing-mid').querySelectorAll(`.${LANDING_DOCKED_CLASS}`),
);

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
    // The WEB menu: five rows. Exit to Desktop is native-only (a tab cannot
    // quit an app) and so is Load mod pack (its stage is a scanned folder, and
    // this host's mod-pack door is the live upload control inside the picker).
    expect(list.map((el) => el.getAttribute(LANDING_ENTRY_ATTR))).toEqual([
      'new_game', 'load_game', 'host_gm', 'join_peer', 'connect_host',
    ]);
    expect(list[0].querySelector('.landing-mi-ix').textContent).toBe('01');
    expect(list[0].querySelector('.landing-mi-label').textContent).toBe('server.landing.new_game');
    expect(list[0].querySelector('.landing-mi-desc').textContent).toBe('server.landing.new_game_desc');
    for (const el of list) expect(el.classList.contains(LANDING_ENTRY_CLASS)).toBe(true);
  });

  it('marks an inert entry aria-disabled rather than disabled', () => {
    // Focusable and readable: "here, but not yet" is more honest than a control
    // a screen reader cannot reach at all.
    //
    // Nothing on the pre-boot WEB menu is inert any more — every row this host
    // offers is a row it can open — so the claim is made where an inert row
    // actually lives: the native menu, whose Load Game and Join as Peer stages
    // are unbuilt and whose Load mod pack needs a folder this run has none of.
    const web = landingDoc();
    renderHostLanding(web, landingViewModel(), t);
    expect(entries(web).filter((el) => el.getAttribute('aria-disabled') === 'true'))
      .toEqual([]);

    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ platform: 'native' }), t);
    const list = entries(doc);
    expect(list.filter((el) => el.getAttribute('aria-disabled') === 'true')
      .map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .toEqual(['load_game', 'host_gm', 'join_peer', 'load_mod_pack']);
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
      'new_game', 'load_game', 'host_gm', 'join_peer', 'connect_host',
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
    // One picker in the column, however often the render runs. Counted as
    // pickers rather than as children because the column has a second, static
    // tenant now — the confirmation stage (issue #1365) — which lives there
    // whether or not it is on screen.
    expect(doc.getElementById('landing-mid').querySelectorAll('#scenario-panel'))
      .toHaveLength(1);
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

describe('renderHostLanding — the confirmation stage (issue #1365)', () => {
  // Exit to Desktop is the one route on the menu an operator cannot take back,
  // so pressing the entry opens a panel that says what is about to happen and
  // asks once. Everything below is written from `vm.confirm` — the OPEN ROW's
  // own block — so this renderer draws confirmations and knows nothing about
  // quitting.

  const NATIVE = { platform: 'native', build: '0.1.0' };
  const askingVm = () => landingViewModel({ ...NATIVE, openEntryId: 'exit_desktop' });
  const el = (doc, id) => doc.getElementById(id);

  it('is off screen and wordless until the route is open', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    expect(el(doc, 'landing-confirm').style.display).toBe('none');
    expect(text(doc, 'landing-confirm-title')).toBe('');
    expect(text(doc, 'landing-confirm-cta')).toBe('');
  });

  it('writes the open row own words when it opens', () => {
    const doc = landingDoc();
    renderHostLanding(doc, askingVm(), t);
    expect(el(doc, 'landing-confirm').style.display).toBe('');
    expect(text(doc, 'landing-confirm-title')).toBe('server.landing.exit_confirm_title');
    expect(text(doc, 'landing-confirm-eyebrow')).toBe('server.landing.exit_confirm_eyebrow');
    expect(text(doc, 'landing-confirm-lead')).toBe('server.landing.exit_confirm_lead');
    expect(text(doc, 'landing-confirm-note')).toBe('server.landing.exit_confirm_note');
    expect(text(doc, 'landing-confirm-cta')).toBe('server.landing.exit_confirm_cta');
    expect(text(doc, 'landing-confirm-cancel')).toBe(CONFIRM_CANCEL_ID);
  });

  it('marks a destructive confirmation from the row tone, not from its id', () => {
    const doc = landingDoc();
    renderHostLanding(doc, askingVm(), t);
    expect(el(doc, 'landing-confirm').classList.contains(CONFIRM_DANGER_CLASS)).toBe(true);
    expect(el(doc, 'landing-confirm-cta').classList.contains(CONFIRM_DANGER_CLASS)).toBe(true);

    // A confirmation that named no tone is an ordinary one, and is drawn as
    // one — proving the class follows `tone` rather than "there is a confirm".
    const quiet = [{
      id: 'q',
      labelId: 'x.q',
      descId: 'x.q_desc',
      stage: 'q-stage',
      confirm: { titleId: 'x.q.t', ctaId: 'x.q.c', action: 'q' },
    }];
    renderHostLanding(doc, landingViewModel({ entries: quiet, openEntryId: 'q' }), t);
    expect(el(doc, 'landing-confirm').classList.contains(CONFIRM_DANGER_CLASS)).toBe(false);
  });

  it('clears itself when the route closes, rather than keeping last words', () => {
    // A hidden panel still holding the previous route's sentence shows it for
    // a frame the next time it opens.
    const doc = landingDoc();
    renderHostLanding(doc, askingVm(), t);
    renderHostLanding(doc, landingViewModel(NATIVE), t);
    expect(el(doc, 'landing-confirm').style.display).toBe('none');
    expect(text(doc, 'landing-confirm-title')).toBe('');
    expect(text(doc, 'landing-confirm-note')).toBe('');
    expect(el(doc, 'landing-confirm').classList.contains(CONFIRM_DANGER_CLASS)).toBe(false);
  });

  it('is not drawn by the stage that opens the World picker', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t);
    expect(el(doc, 'landing-confirm').style.display).toBe('none');
    expect(doc.getElementById('scenario-panel').parentElement)
      .toBe(doc.getElementById('landing-mid'));
  });

  it('carries the row verb through the hook, and judges nothing itself', () => {
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, askingVm(), t, h);
    el(doc, 'landing-confirm-cta').click();
    expect(calls.confirm).toEqual(['exit_desktop']);
    // Never through `pick`: a confirmed verb is not a menu press.
    expect(calls.pick).toEqual([]);
  });

  it('binds the confirm control once per render, not once more each time', () => {
    // The same failure the fullscreen control has: these are STATIC markup and
    // this function runs on every click, so an accumulated listener would send
    // one quit per render.
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, askingVm(), t, h);
    renderHostLanding(doc, askingVm(), t, h);
    renderHostLanding(doc, askingVm(), t, h);
    el(doc, 'landing-confirm-cta').click();
    expect(calls.confirm).toEqual(['exit_desktop']);
  });

  it('closes through the entry own toggle rather than a second way out', () => {
    // Cancel reports as a press on the open entry, which is exactly what a
    // second click on the menu row is — so `nextOpenEntry` closes it, and
    // there is one rule for "this stage is shut" instead of two.
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, askingVm(), t, h);
    el(doc, 'landing-confirm-cancel').click();
    expect(calls.pick).toEqual(['exit_desktop']);
    expect(calls.confirm).toEqual([]);
  });

  it('leaves both controls inert while nothing is open', () => {
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, landingViewModel(), t, h);
    el(doc, 'landing-confirm-cta').click();
    el(doc, 'landing-confirm-cancel').click();
    expect(calls.confirm).toEqual([]);
    expect(calls.pick).toEqual([]);
  });

  it('renders the asking stage with no hooks at all, and does nothing', () => {
    // The host PAGE supplies no `confirm` hook, deliberately: a browser tab
    // cannot quit an application. The control must render and be inert rather
    // than throw out of a click handler.
    const doc = landingDoc();
    renderHostLanding(doc, askingVm(), t);
    expect(() => {
      el(doc, 'landing-confirm-cta').click();
      el(doc, 'landing-confirm-cancel').click();
    }).not.toThrow();
  });
});

describe('renderHostLanding — the mod-pack shelf (issue #1366)', () => {
  // The third tenant of the middle column, and the same claim the confirmation
  // stage makes: everything drawn comes off `vm.packs`, which the view model
  // composes from the OPEN ROW — so this renderer knows there is such a thing
  // as a shelf and knows nothing about Load mod pack.
  const el = (doc, id) => doc.getElementById(id);
  const SHELF = {
    dir: 'mods',
    offered: [
      { file: 'thin-margin.zip', label: 'thin-margin' },
      { file: 'borrowed-sun.zip', label: 'borrowed-sun' },
    ],
    installed: [],
    attempted: null,
    accepted: false,
    findings: [],
    conflicts: [],
  };
  /** What `host_lobby_link.js` passes on a native host given --mod-pack-dir. */
  const shelfVm = (extra) => landingViewModel(Object.assign({
    platform: 'native',
    build: '0.1.0',
    provides: ['packs'],
    openEntryId: 'load_mod_pack',
    packs: SHELF,
  }, extra || {}));
  const rows = (doc) => Array.from(doc.querySelectorAll('.landing-pack'));
  const notes = (doc) => Array.from(doc.querySelectorAll('.landing-note'))
    .map((n) => n.textContent);

  it('is off screen and wordless until the route is open', () => {
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel(), t);
    expect(el(doc, 'landing-packs').style.display).toBe('none');
    expect(text(doc, 'landing-packs-title')).toBe('');
    expect(text(doc, 'landing-packs-cta')).toBe('');
    expect(rows(doc)).toEqual([]);
  });

  it('lists the scanned archives and names the folder they came from', () => {
    const doc = landingDoc();
    renderHostLanding(doc, shelfVm(), t);
    expect(el(doc, 'landing-packs').style.display).toBe('');
    expect(text(doc, 'landing-packs-title')).toBe('server.landing.packs.title');
    expect(text(doc, 'landing-packs-folder'))
      .toBe('server.landing.packs.folder:{"dir":"mods"}');
    expect(rows(doc).map((r) => r.getAttribute('data-landing-pack')))
      .toEqual(['thin-margin.zip', 'borrowed-sun.zip']);
    expect(rows(doc)[0].querySelector('.landing-pack-label').textContent)
      .toBe('thin-margin');
    // The file name is shown as well as the label: a folder is the operator's
    // own filing, and two packs whose labels read alike are told apart by the
    // bytes on disk.
    expect(rows(doc)[0].querySelector('.landing-pack-file').textContent)
      .toBe('thin-margin.zip');
  });

  it('says which emptiness an empty shelf is', () => {
    const doc = landingDoc();
    renderHostLanding(doc, shelfVm({
      packs: { ...SHELF, offered: [] },
    }), t);
    expect(text(doc, 'landing-packs-empty')).toBe('server.landing.packs.empty');
    renderHostLanding(doc, shelfVm({
      packs: { ...SHELF, offered: [], scan_error: 'mods: not found' },
    }), t);
    // The id says which emptiness; the host's own sentence names the path.
    expect(text(doc, 'landing-packs-empty'))
      .toBe('server.landing.packs.scan_failed mods: not found');
    // …and it goes away entirely the moment there is something to list.
    renderHostLanding(doc, shelfVm(), t);
    expect(el(doc, 'landing-packs-empty').style.display).toBe('none');
  });

  it('highlights through the hook and installs through the other one', () => {
    // Two hooks because they cost different things: a highlight is this
    // surface's own memory, an install reads a disk and changes the catalogue
    // every phone in the room is looking at.
    const doc = landingDoc();
    const calls = { pickPack: [], installPack: [], pick: [] };
    const h = {
      pick: (id) => calls.pick.push(id),
      pickPack: (file) => calls.pickPack.push(file),
      installPack: (action, file) => calls.installPack.push([action, file]),
    };
    renderHostLanding(doc, shelfVm(), t, h);
    doc.querySelector('[data-landing-pack="borrowed-sun.zip"]').click();
    expect(calls.pickPack).toEqual(['borrowed-sun.zip']);
    // Nothing is chosen yet as far as this render is concerned, so the install
    // control is dead — that decision is the view model's, not this file's.
    el(doc, 'landing-packs-cta').click();
    expect(calls.installPack).toEqual([]);
    expect(el(doc, 'landing-packs-cta').getAttribute('aria-disabled')).toBe('true');

    renderHostLanding(doc, shelfVm({ chosenPack: 'borrowed-sun.zip' }), t, h);
    expect(el(doc, 'landing-packs-cta').getAttribute('aria-disabled')).toBe('false');
    expect(rows(doc)[1].classList.contains('on')).toBe(true);
    expect(rows(doc)[1].getAttribute('aria-pressed')).toBe('true');
    el(doc, 'landing-packs-cta').click();
    // The ROW's verb, carried verbatim: this renderer never learns its name.
    expect(calls.installPack).toEqual([['install_mod_pack', 'borrowed-sun.zip']]);
  });

  it('binds the install control once per render, not once more each time', () => {
    // Static markup, re-rendered on every press: an accumulated listener would
    // install one pack per render.
    const doc = landingDoc();
    const calls = [];
    const h = { installPack: (action, file) => calls.push(file) };
    const vm = () => shelfVm({ chosenPack: 'thin-margin.zip' });
    renderHostLanding(doc, vm(), t, h);
    renderHostLanding(doc, vm(), t, h);
    renderHostLanding(doc, vm(), t, h);
    el(doc, 'landing-packs-cta').click();
    expect(calls).toEqual(['thin-margin.zip']);
  });

  it('closes through the entry own toggle rather than a second way out', () => {
    const doc = landingDoc();
    const calls = [];
    renderHostLanding(doc, shelfVm(), t, { pick: (id) => calls.push(id) });
    el(doc, 'landing-packs-cancel').click();
    expect(calls).toEqual(['load_mod_pack']);
  });

  it('draws what a refusal said, so a failed pack says what is wrong', () => {
    const doc = landingDoc();
    renderHostLanding(doc, shelfVm({
      packs: {
        ...SHELF,
        attempted: 'broken.zip',
        accepted: false,
        findings: [{
          severity: 'error',
          category: 'missing-manifest',
          message: 'mod pack is missing its required scenarios.toml manifest',
          file: 'scenarios.toml',
        }],
      },
    }), t);
    const drawn = notes(doc);
    expect(drawn[0]).toBe('server.landing.packs.refused:{"pack":"broken.zip"}');
    expect(drawn[1]).toContain('server.landing.packs.severity_error');
    expect(drawn[1]).toContain('scenarios.toml');
    expect(drawn[1]).toContain('missing its required scenarios.toml manifest');
    expect(doc.querySelector('.landing-note-bad')).not.toBe(null);
  });

  it('names which pack won a path two of them carry', () => {
    const doc = landingDoc();
    renderHostLanding(doc, shelfVm({
      packs: {
        ...SHELF,
        installed: [
          { id: 'thin-margin', name: 'Thin Margin', version: '1.2' },
          { id: 'borrowed-sun', name: 'Borrowed Sun', version: '0.9' },
        ],
        conflicts: [{
          path: 'assets/entities/alliance_destroyer.toml',
          winner: 'borrowed-sun',
          losers: ['thin-margin'],
        }],
      },
    }), t);
    const drawn = notes(doc);
    expect(drawn).toContain('server.landing.packs.installed_heading');
    expect(drawn).toContain('server.landing.packs.conflict_heading');
    expect(drawn.some((n) => n.includes('"winner":"borrowed-sun"')
      && n.includes('"losers":"thin-margin"'))).toBe(true);
  });

  it('clears itself when the route closes, rather than keeping last words', () => {
    const doc = landingDoc();
    renderHostLanding(doc, shelfVm({
      packs: { ...SHELF, attempted: 'broken.zip', accepted: false },
    }), t);
    expect(notes(doc).length).toBe(1);
    renderHostLanding(doc, landingViewModel({ platform: 'native' }), t);
    expect(el(doc, 'landing-packs').style.display).toBe('none');
    expect(text(doc, 'landing-packs-title')).toBe('');
    expect(rows(doc)).toEqual([]);
    expect(notes(doc)).toEqual([]);
  });

  it('renders the shelf with no hooks at all, and does nothing', () => {
    const doc = landingDoc();
    renderHostLanding(doc, shelfVm({ chosenPack: 'thin-margin.zip' }), t);
    expect(() => {
      el(doc, 'landing-packs-cta').click();
      el(doc, 'landing-packs-cancel').click();
      rows(doc)[0].click();
    }).not.toThrow();
  });

  it('survives a document that carries no shelf markup at all', () => {
    // The two-document contract: the shelf's panel is one more branch whose
    // element may be absent, and it must do nothing rather than abandon the
    // rest of the render half-written.
    const doc = landingDoc();
    doc.getElementById('landing-packs').remove();
    expect(() => renderHostLanding(doc, shelfVm(), t)).not.toThrow();
    // …and the writes AFTER it still happened.
    expect(text(doc, 'landing-title')).toBe('server.landing.title');
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
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
    expect(docked(doc)).toHaveLength(1);
    expect(doc.getElementById('save-slots-panel').parentElement).toBe(mid);
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
    expect(doc.getElementById('scenario-panel').classList.contains(LANDING_DOCKED_CLASS))
      .toBe(false);

    renderHostLanding(doc, newVm(), t);
    expect(docked(doc)).toHaveLength(1);
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
    expect(docked(doc)).toHaveLength(1);
  });

  it('does nothing on a document that carries no catalogue at all', () => {
    // The two-document contract: a surface may hand this a trimmed subset of
    // the markup, and a stage whose panel is absent must leave the rest of the
    // render intact rather than throw half-way through it.
    const doc = landingDoc();
    doc.getElementById('save-slots-panel').remove();
    expect(() => renderHostLanding(doc, loadVm(), t)).not.toThrow();
    expect(doc.getElementById('landing-panel').dataset.landingStage).toBe('save-catalogue');
    expect(docked(doc)).toHaveLength(0);
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
    // The confirmation stage (issue #1365), which the renderer writes into by
    // id like everything above it and must therefore survive the absence of.
    'landing-confirm', 'landing-confirm-title', 'landing-confirm-eyebrow',
    'landing-confirm-lead', 'landing-confirm-note', 'landing-confirm-cancel',
    'landing-confirm-cta',
  ];

  for (const id of TOUCHED) {
    it(`survives a document with no #${id}`, () => {
      const doc = landingDoc();
      doc.getElementById(id).remove();
      expect(() => {
        renderHostLanding(doc, landingViewModel({ openEntryId: 'new_game' }), t, {});
        renderHostLanding(doc, landingViewModel(), t, {});
        // …and on the stage that writes into the ids this case removes.
        renderHostLanding(
          doc,
          landingViewModel({ platform: 'native', openEntryId: 'exit_desktop' }),
          t,
          {},
        );
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
   * page's own markup minus the staged hull column it cannot reach.
   *
   * Built by DELETING from server.html's markup rather than by hand, so this
   * suite cannot drift into testing a landing neither surface shows. The two
   * deletions are the two `document.rs` makes, and each has its own case below.
   *
   * `#landing-fullscreen-btn` is NOT one of them any more — #1367 put a host
   * verb behind that control and the document kept it — so it stays here.
   */
  function nativeLandingDoc() {
    const doc = landingDoc();
    // The staged hull column (issue #1362), which `LANDING_HULL_COLUMN_MARKER`
    // strips from the real document. See the case below for why.
    doc.getElementById('landing-ship').remove();
    doc.getElementById('landing-panel').style.display = 'none';
    return doc;
  }

  /** What `host_lobby_link.js` passes: platform, build, and who owns the panel. */
  const nativeVm = (extra) => landingViewModel(Object.assign(
    { platform: 'native', build: '0.1.0' },
    extra,
  ));

  it('draws the same landing into a document with the hull column gone', () => {
    const doc = nativeLandingDoc();
    expect(() => {
      renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    }).not.toThrow();
    expect(text(doc, 'landing-title')).toBe('server.landing.title');
    expect(text(doc, 'landing-platform')).toBe('server.landing.platform_native');
    expect(text(doc, 'landing-status-build')).toBe('server.landing.build:{"build":"0.1.0"}');
  });

  it('carries no hull column, because it never enters the rung that reveals one', () => {
    // Issue #1362's staged rung, and the two halves of one decision that have
    // to stay together.
    //
    // `host_lobby_link.js` passes no `deepStage` — the way OUT of a deep stage
    // is the hull column's Back control, which needs a `backToWorlds` hook this
    // surface has no host verb behind — so the root never becomes `is-deep`,
    // and `gui/host-landing.css` keeps `.landing-col-ship` at
    // `opacity: 0; visibility: hidden` at every breakpoint until it does. So
    // `native_host::host_lobby::document` removes the column outright
    // (`LANDING_HULL_COLUMN_MARKER`), which is what makes
    // `gui/host-scenario-render.js` take its single-column branch and draw the
    // hulls in the World column, as this surface always did.
    //
    // Carried WITHOUT a `deepStage` — which is how it arrived, as a child of
    // the whole-panel extraction — the hull picker would mount into a column
    // nothing can ever show, and a multi-hull World would be unpickable here
    // with a clean log.
    const doc = nativeLandingDoc();
    renderHostLanding(
      doc, nativeVm({ openEntryId: 'new_game' }), t, {}, { ownPanelVisibility: true },
    );
    const root = doc.getElementById('landing-panel');
    expect(root.classList.contains('is-open')).toBe(true);
    expect(root.classList.contains('is-deep')).toBe(false);
    expect(root.style.getPropertyValue('--landing-depth')).toBe('1');
    expect(doc.getElementById('ship-list')).toBe(null);
    // …and the menu is still reachable, which is the other thing the deep rung
    // takes away: the renderer writes `inert` on it from the depth, and a
    // surface that went deep with no Back would have closed its own front door.
    expect(doc.getElementById('landing-menu').hasAttribute('inert')).toBe(false);

    // The WEB surface does go deep, so this is a surface that opted out of a
    // rung and not a rung nobody has.
    const web = landingDoc();
    renderHostLanding(
      web, landingViewModel({ openEntryId: 'new_game', deepStage: 'ship-picker' }), t,
    );
    const webRoot = web.getElementById('landing-panel');
    expect(webRoot.classList.contains('is-deep')).toBe(true);
    expect(webRoot.style.getPropertyValue('--landing-depth')).toBe('2');
    expect(web.getElementById('ship-list')).not.toBe(null);
    expect(web.getElementById('landing-menu').hasAttribute('inert')).toBe(true);
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
    // AC5 is the second of those. See the row, and the test below. Exit to
    // Desktop is the mirror image and native-only, so this menu is longer than
    // the web one by exactly that row (issue #1365).
    expect(ids).toEqual([
      'new_game', 'load_game', 'host_gm', 'join_peer', 'load_mod_pack',
      'exit_desktop',
    ]);
    // …and the same document on the web surface still offers it, so this is a
    // curated menu and not a lost row.
    const web = landingDoc();
    renderHostLanding(web, landingViewModel(), t, {});
    expect(entries(web).map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .toContain('connect_host');
  });

  it('offers Exit to Desktop here and nowhere else (issue #1365)', () => {
    // The mirror image of the case above, on the same mechanism: a browser tab
    // cannot quit an application, so the row is `platforms: ['native']` and no
    // build check appears in this renderer or in either document.
    const doc = nativeLandingDoc();
    renderHostLanding(doc, nativeVm(), t, {}, { ownPanelVisibility: true });
    expect(entries(doc).map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .toContain('exit_desktop');
    const web = landingDoc();
    renderHostLanding(web, landingViewModel(), t, {});
    expect(entries(web).map((el) => el.getAttribute(LANDING_ENTRY_ATTR)))
      .not.toContain('exit_desktop');
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
    // The bug `undockLandingPanels` exists for, reached the other way: a `display` set
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
    expect(disabled).toEqual(['load_game', 'host_gm', 'join_peer', 'load_mod_pack']);
    // ...and pressing it opens no middle column, which is the failure the row
    // being absent was avoiding in the first place.
    renderHostLanding(
      doc, nativeVm({ openEntryId: 'load_game' }), t, {}, { ownPanelVisibility: true },
    );
    expect(docked(doc)).toHaveLength(0);
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

describe('renderHostLanding — the join-code stage (issue #1364)', () => {
  const peerVm = (joinErrorId) => landingViewModel({ openEntryId: 'join_peer', joinErrorId });
  const clientVm = () => landingViewModel({ openEntryId: 'connect_host' });
  const newVm = () => landingViewModel({ openEntryId: 'new_game' });

  it('reveals the EXISTING join panel by moving it into the middle column', () => {
    const doc = landingDoc();
    renderHostLanding(doc, peerVm(), t);
    const panel = doc.getElementById('landing-join-panel');
    expect(panel.parentElement).toBe(doc.getElementById('landing-mid'));
    expect(panel.classList.contains(LANDING_DOCKED_CLASS)).toBe(true);
    expect(doc.getElementById('landing-panel').dataset.landingStage).toBe('join-code');
  });

  it('writes what THIS route joins as, and rewrites it for the other one', () => {
    // The load-bearing claim: one panel, two routes, and the difference is the
    // row's `join` descriptor rather than a second set of markup. The failure
    // this catches is the sticky one — a panel that still says "Game master
    // peer" over a field that now wants a crew code.
    const doc = landingDoc();
    renderHostLanding(doc, peerVm(), t);
    expect(text(doc, 'landing-join-role')).toBe('server.landing.join_peer_role');
    expect(text(doc, 'landing-join-blurb')).toBe('server.landing.join_peer_blurb');
    expect(text(doc, 'landing-join-submit')).toBe('server.landing.join_peer_submit');

    renderHostLanding(doc, clientVm(), t);
    expect(text(doc, 'landing-join-role')).toBe('server.landing.connect_host_role');
    expect(text(doc, 'landing-join-blurb')).toBe('server.landing.connect_host_blurb');
    expect(text(doc, 'landing-join-submit')).toBe('server.landing.connect_host_submit');
  });

  it('writes the field`s own label, placeholder and hint from the view model', () => {
    const doc = landingDoc();
    renderHostLanding(doc, peerVm(), t);
    expect(text(doc, 'landing-join-label')).toBe('server.landing.join_code_label');
    expect(text(doc, 'landing-join-hint')).toBe('server.landing.join_code_hint');
    const field = doc.getElementById('landing-join-code');
    expect(field.placeholder).toBe('server.landing.join_code_placeholder');
    expect(field.getAttribute('aria-label')).toBe('server.landing.join_code_label');
  });

  it('shows the refusal the view model carries, and clears it again', () => {
    // AC4. The sentence is a `strings.csv` id chosen by `landingJoinAttempt`;
    // this only puts it on screen, and takes it off when the attempt it
    // described is no longer the last one.
    const doc = landingDoc();
    renderHostLanding(doc, peerVm('client.join.error_length'), t);
    expect(text(doc, 'landing-join-error')).toBe('client.join.error_length');
    renderHostLanding(doc, peerVm(), t);
    expect(text(doc, 'landing-join-error')).toBe('');
  });

  it('carries the typed code back with the ROW`s descriptor, judging nothing', () => {
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, peerVm(), t, h);
    doc.getElementById('landing-join-code').value = 'quarking';
    doc.getElementById('landing-join-submit').click();
    expect(calls.join).toEqual([['boot-game-master', 'quarking']]);

    // The other route, same field, same button: what differs is the descriptor
    // the hook is handed, which is what the caller dispatches on.
    renderHostLanding(doc, clientVm(), t, h);
    doc.getElementById('landing-join-submit').click();
    expect(calls.join[1]).toEqual(['open-client-page', 'quarking']);
  });

  it('reports nonsense too, because judging a code is not this module`s job', () => {
    // The exact sibling of "reports every click, including the inert entries":
    // whether eight letters are a code is `landingJoinAttempt`'s answer, and a
    // second judgement here would be a place for the two to disagree.
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, peerVm(), t, h);
    doc.getElementById('landing-join-code').value = '???';
    doc.getElementById('landing-join-submit').click();
    expect(calls.join).toEqual([['boot-game-master', '???']]);
  });

  it('submits on Return as well, so the field can be driven entirely by keys', () => {
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, peerVm(), t, h);
    const field = doc.getElementById('landing-join-code');
    field.value = 'QUARKING';
    field.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(calls.join).toEqual([['boot-game-master', 'QUARKING']]);
    // ...and only on Return: every other key is somebody typing.
    field.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'A', bubbles: true }));
    expect(calls.join).toHaveLength(1);
  });

  it('binds the submit once per render, not once more each time', () => {
    // The static-markup hazard the fullscreen control has: this function runs
    // again on every menu click and on every refusal, so an accumulated
    // listener would join three times on the third attempt.
    const doc = landingDoc();
    const [h, calls] = hooks();
    renderHostLanding(doc, peerVm(), t, h);
    renderHostLanding(doc, peerVm('client.join.error_length'), t, h);
    renderHostLanding(doc, peerVm(), t, h);
    doc.getElementById('landing-join-submit').click();
    expect(calls.join).toHaveLength(1);
  });

  it('never writes the field`s value, so a refusal does not delete what was typed', () => {
    // The one thing this renderer deliberately does not own. A render that
    // cleared the value would take away the eight letters at the exact moment
    // the operator needs to see what is wrong with them.
    const doc = landingDoc();
    const field = doc.getElementById('landing-join-code');
    renderHostLanding(doc, peerVm(), t);
    field.value = 'QUARKING';
    renderHostLanding(doc, peerVm('client.join.error_denied'), t);
    expect(field.value).toBe('QUARKING');
    renderHostLanding(doc, clientVm(), t);
    expect(field.value).toBe('QUARKING');
  });

  it('empties every join sentence when the stage closes', () => {
    // A panel undocked with last week's role still written in it would say the
    // wrong thing the instant something else showed it.
    const doc = landingDoc();
    renderHostLanding(doc, peerVm('client.join.error_empty'), t);
    renderHostLanding(doc, landingViewModel(), t);
    for (const id of ['landing-join-role', 'landing-join-blurb', 'landing-join-label',
      'landing-join-hint', 'landing-join-error', 'landing-join-submit']) {
      expect(text(doc, id), id).toBe('');
    }
  });

  it('swaps the join panel against the other borrowed panels rather than stacking', () => {
    const doc = landingDoc();
    const mid = doc.getElementById('landing-mid');
    renderHostLanding(doc, newVm(), t);
    renderHostLanding(doc, peerVm(), t);
    expect(docked(doc)).toHaveLength(1);
    expect(doc.getElementById('landing-join-panel').parentElement).toBe(mid);
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);

    renderHostLanding(doc, newVm(), t);
    expect(docked(doc)).toHaveLength(1);
    expect(doc.getElementById('scenario-panel').parentElement).toBe(mid);
    expect(doc.getElementById('landing-join-panel').parentElement).toBe(doc.body);
  });

  it('KEEPS the one panel docked across a swap between the two join routes', () => {
    // Both rows name the same node, so moving from Join as Peer to Connect to
    // Host must not undock and re-dock it — that would drop what was typed with
    // the element, and it is the case a per-id undock would have got wrong.
    const doc = landingDoc();
    const mid = doc.getElementById('landing-mid');
    renderHostLanding(doc, peerVm(), t);
    const panel = doc.getElementById('landing-join-panel');
    renderHostLanding(doc, clientVm(), t);
    expect(doc.getElementById('landing-join-panel')).toBe(panel);
    expect(panel.parentElement).toBe(mid);
    expect(docked(doc)).toHaveLength(1);
  });

  it('returns the join panel to the body on undockLandingPanels, unasked', () => {
    const doc = landingDoc();
    renderHostLanding(doc, peerVm(), t);
    undockLandingPanels(doc);
    const panel = doc.getElementById('landing-join-panel');
    expect(panel.parentElement).toBe(doc.body);
    expect(panel.classList.contains(LANDING_DOCKED_CLASS)).toBe(false);
  });

  it('does nothing on a document that carries no join panel at all', () => {
    // The two-document contract: the native lobby document has no
    // `#landing-join-panel`, because the Game Master profile is a browser
    // profile. The render must leave the rest of the landing intact.
    const doc = landingDoc();
    doc.getElementById('landing-join-panel').remove();
    expect(() => renderHostLanding(doc, peerVm(), t, hooks()[0])).not.toThrow();
    expect(doc.getElementById('landing-panel').dataset.landingStage).toBe('join-code');
    expect(docked(doc)).toHaveLength(0);
  });

  it('draws nothing at all for a surface whose menu has no join route', () => {
    // The native landing: the row is offered, its stage is taken away by
    // `stagePlatforms`, so `vm.join` is null and every sentence stays empty.
    const doc = landingDoc();
    renderHostLanding(doc, landingViewModel({ platform: 'native', openEntryId: 'join_peer' }), t);
    expect(text(doc, 'landing-join-role')).toBe('');
    expect(doc.getElementById('landing-join-panel').parentElement).toBe(doc.body);
  });
});
