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
  CONFIRM_DANGER_CLASS,
} from '../../gui/host-landing-render.js';
import { CONFIRM_CANCEL_ID, landingViewModel } from '../../gui/host-landing-view.js';

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
  const calls = { pick: [], confirm: [], fullscreen: 0 };
  return [
    {
      pick: (id) => calls.pick.push(id),
      confirm: (action) => calls.confirm.push(action),
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
    // One picker in the column, however often the render runs. Counted as
    // pickers rather than as children because the column has a second, static
    // tenant now — the confirmation stage (issue #1365) — which lives there
    // whether or not it is on screen.
    expect(doc.getElementById('landing-mid').querySelectorAll('#scenario-panel'))
      .toHaveLength(1);
  });

  it('leaves the picker alone when the surface says it owns it elsewhere', () => {
    const doc = landingDoc();
    renderHostLanding(doc, openVm(), t, null, { dockPicker: false });
    expect(doc.getElementById('scenario-panel').parentElement).toBe(doc.body);
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
    expect(ids).toEqual([
      'new_game', 'load_game', 'join_peer', 'load_mod_pack', 'exit_desktop',
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
    expect(picker.classList.contains(PICKER_DOCKED_CLASS)).toBe(true);
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
    // model's decision, never a second judgement made in the renderer.
    doc.querySelector(`[${LANDING_ENTRY_ATTR}="load_game"]`).click();
    expect(calls.pick).toEqual(['new_game', 'load_game']);
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
