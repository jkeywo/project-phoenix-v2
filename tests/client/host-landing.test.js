// tests/client/host-landing.test.js — issue #1360: the landing screen's pure
// view model (gui/host-landing-view.js).
//
// Two claims are under test, and both are load-bearing for the six sibling
// slices that build on this one:
//
//   1. THE ENTRIES ARE DATA. Nothing in the module reads an id by name, so a
//      caller can hand it a table it has never seen and get a menu. The cases
//      below prove that by driving custom tables through it — if adding an
//      entry ever needs a branch in the module, one of these fails.
//
//   2. THE TOGGLE IS PURE. "Clicking New Game again closes it" is
//      `nextOpenEntry`, not a click handler, so it is a test rather than a
//      claim about the DOM.
//
// No DOM here at all: that is the whole reason the decision was lifted out.

import { describe, it, expect } from 'vitest';
import {
  LANDING_ENTRIES,
  CONFIRM_CANCEL_ID,
  landingEntries,
  nextOpenEntry,
  landingViewModel,
} from '../../gui/host-landing-view.js';

/** A table this suite owns, so a shipped-menu edit cannot silently retune it. */
const TABLE = [
  { id: 'alpha', labelId: 'x.alpha', descId: 'x.alpha_desc', stage: 'stage-a', platforms: ['web', 'native'] },
  { id: 'beta', labelId: 'x.beta', descId: 'x.beta_desc', stage: null, platforms: ['web', 'native'] },
  { id: 'gamma', labelId: 'x.gamma', descId: 'x.gamma_desc', stage: 'stage-c', platforms: ['native'] },
];

describe('the shipped entry table', () => {
  it('offers the six entries drawn so far, in order', () => {
    expect(LANDING_ENTRIES.map((e) => e.id)).toEqual([
      'new_game', 'load_game', 'join_peer', 'connect_host', 'load_mod_pack',
      'exit_desktop',
    ]);
  });

  it('gives New Game and Exit to Desktop a stage — the rest are inert', () => {
    // The honest shape of a tracer: the ones without a stage each have a
    // sibling issue that gives them one, and until then they must not pretend.
    const staged = LANDING_ENTRIES.filter((e) => e.stage).map((e) => e.id);
    expect(staged).toEqual(['new_game', 'exit_desktop']);
  });

  it('offers Exit to Desktop on native only, because a tab cannot quit an app', () => {
    // Issue #1365, and the whole of how it is kept off the web: a field on a
    // row. There is no build check in the view model, in the renderer, or in
    // either document.
    const exit = LANDING_ENTRIES.find((e) => e.id === 'exit_desktop');
    expect(exit.platforms).toEqual(['native']);
  });

  it('makes Exit to Desktop ask instead of act, and says so on the row', () => {
    // The one irreversible route on the menu, so the press opens a stage that
    // states what happens and asks once. Everything that stage says is on the
    // row, which is what lets a second confirming entry cost a row.
    const exit = LANDING_ENTRIES.find((e) => e.id === 'exit_desktop');
    expect(exit.stage).toBe('exit-confirm');
    expect(exit.confirm.tone).toBe('danger');
    expect(exit.confirm.action).toBe('exit_desktop');
    for (const key of ['titleId', 'eyebrowId', 'leadId', 'noteId', 'ctaId']) {
      expect(typeof exit.confirm[key]).toBe('string');
      expect(exit.confirm[key].length).toBeGreaterThan(0);
    }
  });

  it('gives no other row a confirmation, so New Game still opens on one press', () => {
    expect(LANDING_ENTRIES.filter((e) => e.confirm).map((e) => e.id))
      .toEqual(['exit_desktop']);
  });

  it('gives every row a label and a description id, so the renderer never guesses', () => {
    for (const entry of LANDING_ENTRIES) {
      expect(typeof entry.labelId).toBe('string');
      expect(typeof entry.descId).toBe('string');
      expect(entry.labelId.length).toBeGreaterThan(0);
      expect(entry.descId.length).toBeGreaterThan(0);
    }
  });
});

describe('landingEntries', () => {
  it('filters by platform, which is how a native-only row arrives without a build check', () => {
    expect(landingEntries('web', TABLE).map((e) => e.id)).toEqual(['alpha', 'beta']);
    expect(landingEntries('native', TABLE).map((e) => e.id)).toEqual(['alpha', 'beta', 'gamma']);
  });

  it('offers a row with no platforms field everywhere', () => {
    const anywhere = [{ id: 'any', labelId: 'a', descId: 'b', stage: null }];
    expect(landingEntries('web', anywhere).map((e) => e.id)).toEqual(['any']);
    expect(landingEntries('native', anywhere).map((e) => e.id)).toEqual(['any']);
  });

  it('drops a malformed row rather than emitting an entry with no id', () => {
    const messy = [null, { labelId: 'no id' }, TABLE[0]];
    expect(landingEntries('web', messy).map((e) => e.id)).toEqual(['alpha']);
  });
});

describe('nextOpenEntry', () => {
  it('opens an entry that has a stage', () => {
    expect(nextOpenEntry(null, 'alpha', TABLE)).toBe('alpha');
  });

  it('closes the entry that is already open — the second click on New Game', () => {
    expect(nextOpenEntry('alpha', 'alpha', TABLE)).toBe(null);
  });

  it('replaces one open stage with another', () => {
    expect(nextOpenEntry('alpha', 'gamma', TABLE)).toBe('gamma');
  });

  it('changes nothing for an inert entry, open or closed', () => {
    expect(nextOpenEntry(null, 'beta', TABLE)).toBe(null);
    expect(nextOpenEntry('alpha', 'beta', TABLE)).toBe('alpha');
  });

  it('changes nothing for an id the table does not hold', () => {
    expect(nextOpenEntry('alpha', 'nonesuch', TABLE)).toBe('alpha');
    expect(nextOpenEntry(null, 'nonesuch', TABLE)).toBe(null);
  });

  it('reads New Game off the shipped table when given no table at all', () => {
    expect(nextOpenEntry(null, 'new_game')).toBe('new_game');
    expect(nextOpenEntry('new_game', 'new_game')).toBe(null);
    // ...and the entries that have no stage yet stay inert through the default.
    expect(nextOpenEntry(null, 'load_game')).toBe(null);
  });
});

describe('landingViewModel', () => {
  it('is idle with nothing open', () => {
    const vm = landingViewModel({ entries: TABLE });
    expect(vm.stage).toBe('idle');
    expect(vm.openEntryId).toBe(null);
    expect(vm.depth).toBe(0);
    expect(vm.rootClass).toBe('is-idle');
    expect(vm.status.sessionId).toBe('server.landing.status_no_session');
  });

  it('reports the open entry as its stage, one step deep', () => {
    const vm = landingViewModel({ entries: TABLE, openEntryId: 'alpha' });
    expect(vm.stage).toBe('stage-a');
    expect(vm.openEntryId).toBe('alpha');
    expect(vm.depth).toBe(1);
    expect(vm.rootClass).toBe('is-open');
    expect(vm.status.sessionId).toBe('server.landing.status_hosting');
  });

  it('marks exactly the open entry selected, and every stageless entry inert', () => {
    const vm = landingViewModel({ entries: TABLE, platform: 'native', openEntryId: 'gamma' });
    expect(vm.entries.map((e) => [e.id, e.selected, e.inert])).toEqual([
      ['alpha', false, false],
      ['beta', false, true],
      ['gamma', true, false],
    ]);
  });

  it('numbers the entries as the design draws them, two digits from one', () => {
    const vm = landingViewModel({ entries: TABLE, platform: 'native' });
    expect(vm.entries.map((e) => e.ordinal)).toEqual(['01', '02', '03']);
  });

  it('reads as closed when the open id is an entry this platform does not offer', () => {
    // A caller's memory can outlive a menu change: the web and native hosts
    // share this module and not their rows.
    const vm = landingViewModel({ entries: TABLE, platform: 'web', openEntryId: 'gamma' });
    expect(vm.stage).toBe('idle');
    expect(vm.openEntryId).toBe(null);
  });

  it('reads as closed when the open id names an entry with no stage', () => {
    const vm = landingViewModel({ entries: TABLE, openEntryId: 'beta' });
    expect(vm.stage).toBe('idle');
    expect(vm.entries.every((e) => !e.selected)).toBe(true);
  });

  it('names which build it is, on both platforms', () => {
    expect(landingViewModel({ entries: TABLE }).identity.platformLabelId)
      .toBe('server.landing.platform_web');
    expect(landingViewModel({ entries: TABLE, platform: 'native' }).identity.platformLabelId)
      .toBe('server.landing.platform_native');
  });

  it('carries the build id as params on a {id, params} pair, not as prose', () => {
    const vm = landingViewModel({ entries: TABLE, build: '2026.09.04-abc1234' });
    expect(vm.status.build).toEqual({
      id: 'server.landing.build',
      params: { build: '2026.09.04-abc1234' },
    });
  });

  it('says "dev" when nothing told it which build this is', () => {
    expect(landingViewModel({ entries: TABLE }).status.build.params.build).toBe('dev');
  });

  it('takes no arguments at all and still returns a renderable menu', () => {
    // The very first paint on the host page calls it before anything has
    // happened, so "no input" has to be a state and not a crash.
    const vm = landingViewModel();
    expect(vm.rootClass).toBe('is-idle');
    // No platform given means the WEB one, so this is the shipped table as
    // that host is offered it — which is no longer the whole table, now that a
    // row is native-only (issue #1365).
    expect(vm.entries.map((e) => e.id))
      .toEqual(landingEntries('web', LANDING_ENTRIES).map((e) => e.id));
  });

  it('opens New Game off the shipped table, the one working action on the web', () => {
    const vm = landingViewModel({ openEntryId: 'new_game' });
    expect(vm.stage).toBe('world-picker');
    expect(vm.entries.find((e) => e.id === 'new_game').selected).toBe(true);
    expect(vm.entries.filter((e) => e.inert).map((e) => e.id))
      .toEqual(['load_game', 'join_peer', 'connect_host', 'load_mod_pack']);
  });
});

describe('landingViewModel — a route that asks first (issue #1365)', () => {
  // The claim that keeps a confirmation DATA: the model republishes the OPEN
  // ROW's block, so a renderer draws "this route's confirmation" and never
  // "the exit confirmation". Driven through a table this suite owns wherever
  // the mechanism is the point, and through the shipped one where the shipped
  // row is.

  const ASKS = [
    { id: 'plain', labelId: 'x.p', descId: 'x.p_desc', stage: 'plain-stage' },
    {
      id: 'grave',
      labelId: 'x.g',
      descId: 'x.g_desc',
      stage: 'grave-stage',
      confirm: {
        titleId: 'x.g.title',
        leadId: 'x.g.lead',
        noteId: 'x.g.note',
        ctaId: 'x.g.cta',
        tone: 'danger',
        action: 'do_the_grave_thing',
      },
    },
  ];

  it('is null for a route that simply opens something', () => {
    expect(landingViewModel({ entries: ASKS, openEntryId: 'plain' }).confirm).toBe(null);
    expect(landingViewModel({ entries: ASKS }).confirm).toBe(null);
  });

  it('republishes the open row block, verb and tone included', () => {
    const vm = landingViewModel({ entries: ASKS, openEntryId: 'grave' });
    expect(vm.stage).toBe('grave-stage');
    expect(vm.confirm.titleId).toBe('x.g.title');
    expect(vm.confirm.leadId).toBe('x.g.lead');
    expect(vm.confirm.noteId).toBe('x.g.note');
    expect(vm.confirm.ctaId).toBe('x.g.cta');
    expect(vm.confirm.tone).toBe('danger');
    expect(vm.confirm.action).toBe('do_the_grave_thing');
  });

  it('fills the way back from the shared default, so a row need not repeat it', () => {
    const vm = landingViewModel({ entries: ASKS, openEntryId: 'grave' });
    expect(vm.confirm.cancelId).toBe(CONFIRM_CANCEL_ID);
    // ...and a row that genuinely wants other words still wins.
    const own = ASKS.map((e) => (e.id === 'grave'
      ? { ...e, confirm: { ...e.confirm, cancelId: 'x.g.back' } }
      : e));
    expect(landingViewModel({ entries: own, openEntryId: 'grave' }).confirm.cancelId)
      .toBe('x.g.back');
  });

  it('calls an unnamed tone ordinary rather than leaving it undefined', () => {
    // The renderer paints from this, so "no tone" has to be a value.
    const quiet = [{
      id: 'q', labelId: 'x.q', descId: 'x.q_desc', stage: 's',
      confirm: { titleId: 'x.q.t', ctaId: 'x.q.c', action: 'q' },
    }];
    expect(landingViewModel({ entries: quiet, openEntryId: 'q' }).confirm.tone)
      .toBe('normal');
  });

  it('drops it the moment the route closes, and while the landing is dismissed', () => {
    expect(landingViewModel({ entries: ASKS, openEntryId: null }).confirm).toBe(null);
    expect(landingViewModel({ entries: ASKS, openEntryId: 'grave', dismissed: true }).confirm)
      .toBe(null);
  });

  it('carries the shipped Exit to Desktop block, on native and only there', () => {
    const native = landingViewModel({ platform: 'native', openEntryId: 'exit_desktop' });
    expect(native.stage).toBe('exit-confirm');
    expect(native.confirm.action).toBe('exit_desktop');
    expect(native.confirm.tone).toBe('danger');
    expect(native.confirm.ctaId).toBe('server.landing.exit_confirm_cta');
    // The web host does not offer the row, so a remembered id reads as closed —
    // which is exactly the guard that stops a shared memory opening a stage
    // this platform has nothing behind.
    const web = landingViewModel({ platform: 'web', openEntryId: 'exit_desktop' });
    expect(web.stage).toBe('idle');
    expect(web.confirm).toBe(null);
  });
});

describe('the entries a native host is offered (issue #1361)', () => {
  // The doctrine, said in the one place it can be said once: a control exists
  // exactly when something behind it can answer it. The native surface renders
  // from this same table through the same renderer, so an entry it cannot
  // answer is kept off it by a FIELD ON A ROW and not by a build check in the
  // renderer or an edit to the native document's markup.

  it('does not offer Connect to Host, because a native host has no join leg', () => {
    // A native host is always a host. Issue #1364 settles the web half —
    // Connect to Host navigates to the client page with the code — and there is
    // no such page and no such leg here.
    const native = landingEntries('native', LANDING_ENTRIES).map((e) => e.id);
    expect(native).not.toContain('connect_host');
    expect(native).toEqual([
      'new_game', 'load_game', 'join_peer', 'load_mod_pack', 'exit_desktop',
    ]);
  });

  it('still offers it on the web, so this is a curated menu and not a lost row', () => {
    expect(landingEntries('web', LANDING_ENTRIES).map((e) => e.id)).toContain('connect_host');
  });

  it('offers New Game on both, because both reach a world-load path', () => {
    // On the web it slides the picker into the middle column; on native the
    // same renderer moves the same `#scenario-panel` node, whose picks reach
    // the scenario arbiter and the native world load down the path they always
    // took. One entry, one stage, two hosts.
    for (const platform of ['web', 'native']) {
      const vm = landingViewModel({ platform, openEntryId: 'new_game' });
      expect(vm.stage).toBe('world-picker');
      expect(vm.entries.find((e) => e.id === 'new_game').selected).toBe(true);
    }
  });

  it('numbers a curated menu from one, so native has no gap where a row was', () => {
    const vm = landingViewModel({ platform: 'native' });
    expect(vm.entries.map((e) => e.ordinal)).toEqual(['01', '02', '03', '04', '05']);
  });

  it('offers Exit to Desktop on native and never on the web', () => {
    // The mirror of Connect to Host, and settled the same way: a browser tab
    // cannot quit an application, so nothing is behind that control there.
    expect(landingEntries('native', LANDING_ENTRIES).map((e) => e.id))
      .toContain('exit_desktop');
    expect(landingEntries('web', LANDING_ENTRIES).map((e) => e.id))
      .not.toContain('exit_desktop');
  });
});

describe('landingViewModel — a dismissed landing', () => {
  // The native surface has no page lifecycle: its host is the only thing that
  // knows a World has been committed, so "the landing is past" is an input
  // here rather than a `display` set by page script. It is the exact sibling
  // of `scenarioCatalogView`'s `locked`.

  it('collapses to a stage of its own that means "not on screen"', () => {
    const vm = landingViewModel({ entries: TABLE, dismissed: true });
    expect(vm.stage).toBe('dismissed');
    expect(vm.dismissed).toBe(true);
    expect(vm.depth).toBe(0);
    expect(vm.rootClass).toBe('is-idle');
  });

  it('outranks a remembered open entry rather than reopening under it', () => {
    // A landing brought back later — a Game Over returning a host to selection
    // — opens on its front door, not on a stage nobody asked for.
    const vm = landingViewModel({ entries: TABLE, openEntryId: 'alpha', dismissed: true });
    expect(vm.stage).toBe('dismissed');
    expect(vm.openEntryId).toBe(null);
    expect(vm.entries.every((e) => !e.selected)).toBe(true);
  });

  it('is false by default, so the host page is unchanged by its existence', () => {
    expect(landingViewModel({ entries: TABLE }).dismissed).toBe(false);
    expect(landingViewModel().dismissed).toBe(false);
    expect(landingViewModel().stage).toBe('idle');
  });

  it('still lists the menu, so a landing shown again needs no second decision', () => {
    const vm = landingViewModel({ entries: TABLE, platform: 'native', dismissed: true });
    expect(vm.entries.map((e) => e.id)).toEqual(['alpha', 'beta', 'gamma']);
  });
});
