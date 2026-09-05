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
  landingEntries,
  nextOpenEntry,
  landingViewModel,
} from '../../gui/host-landing-view.js';

/** A table this suite owns, so a shipped-menu edit cannot silently retune it. */
const TABLE = [
  {
    id: 'alpha',
    labelId: 'x.alpha',
    descId: 'x.alpha_desc',
    stage: 'stage-a',
    // A TWO-rung ladder, so "how deep" is exercised past the one rung New Game
    // actually ships with — the depth has to be the position in the list and
    // not a boolean somebody wrote as one (issue #1362).
    deeper: ['stage-a2', 'stage-a3'],
    docks: 'panel-a',
    platforms: ['web', 'native'],
  },
  { id: 'beta', labelId: 'x.beta', descId: 'x.beta_desc', stage: null, platforms: ['web', 'native'] },
  { id: 'gamma', labelId: 'x.gamma', descId: 'x.gamma_desc', stage: 'stage-c', platforms: ['native'] },
];

describe('the shipped entry table', () => {
  it('offers the five entries this slice draws, in order', () => {
    expect(LANDING_ENTRIES.map((e) => e.id)).toEqual([
      'new_game', 'load_game', 'join_peer', 'connect_host', 'load_mod_pack',
    ]);
  });

  it('gives New Game and Load Game a stage — the rest render and are inert', () => {
    // The honest shape of a tracer: the three without a stage each have a
    // sibling issue that gives them one, and until then they must not pretend.
    // Load Game joined in issue #1363, and joined by growing a stage on its
    // own row rather than by anything below it learning its name.
    const staged = LANDING_ENTRIES.filter((e) => e.stage).map((e) => e.id);
    expect(staged).toEqual(['new_game', 'load_game']);
  });

  it('names the panel each staged row borrows, as an element id on the row', () => {
    // Two borrowers now, which is why the field is an id rather than the
    // boolean it was while only the picker docked: the renderer reads the row
    // and moves the node it names, and a third borrower needs no new branch.
    expect(LANDING_ENTRIES.filter((e) => e.docks).map((e) => [e.id, e.docks])).toEqual([
      ['new_game', 'scenario-panel'],
      ['load_game', 'save-slots-panel'],
    ]);
  });

  it('records Load Game\'s native gap as an unserved STAGE, not an unoffered row', () => {
    // #1363's AC5 is unbuilt, and this is where that is written down. The two
    // fields say different things: `platforms` is what a surface does not
    // offer (Connect to Host, for ever), `stagePlatforms` is what it cannot
    // open yet (Load Game, until the native save-catalogue channel lands).
    const load = LANDING_ENTRIES.find((e) => e.id === 'load_game');
    expect(load.platforms).toEqual(['web', 'native']);
    expect(load.stagePlatforms).toEqual(['web']);
    const connect = LANDING_ENTRIES.find((e) => e.id === 'connect_host');
    expect(connect.platforms).toEqual(['web']);
    expect(connect.stagePlatforms).toBeUndefined();
    // Nothing else in the shipped table is gated this way; a second one would
    // be a second unfinished AC and should arrive with its own test.
    expect(LANDING_ENTRIES.filter((e) => e.stagePlatforms).map((e) => e.id))
      .toEqual(['load_game']);
  });

  it('does not offer Exit to Desktop, which is a native-only row for #1365', () => {
    expect(LANDING_ENTRIES.some((e) => e.id === 'exit')).toBe(false);
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

describe('landingEntries — a stage this surface cannot serve yet', () => {
  // `stagePlatforms` is the field that keeps `platforms` honest: one says what
  // a surface does not OFFER, the other what it cannot yet OPEN, and only the
  // first is doctrine. Collapsing them would have deleted a row to record a
  // gap (issue #1363's AC5).
  const GATED = [
    { id: 'alpha', labelId: 'x.a', descId: 'x.a_desc', stage: 'stage-a', docks: 'panel-a', deeper: ['stage-a2'], stagePlatforms: ['web'] },
    { id: 'beta', labelId: 'x.b', descId: 'x.b_desc', stage: 'stage-b' },
  ];

  it('keeps the row and takes its stage away, on the surface that lacks it', () => {
    const native = landingEntries('native', GATED);
    expect(native.map((e) => e.id)).toEqual(['alpha', 'beta']);
    const alpha = native.find((e) => e.id === 'alpha');
    expect(alpha.stage).toBe(null);
    // A stage nothing can open has no panel to borrow and no ladder to climb.
    expect(alpha.docks).toBe(null);
    expect(alpha.deeper).toBe(null);
    // ...and the table itself is untouched: the copy is the surface's view of
    // the row, never an edit to the shipped row every surface shares.
    expect(GATED[0].stage).toBe('stage-a');
    expect(GATED[0].docks).toBe('panel-a');
  });

  it('leaves the row alone on the surface that lists it, and passes it through', () => {
    const web = landingEntries('web', GATED);
    expect(web.find((e) => e.id === 'alpha')).toBe(GATED[0]);
    expect(web.find((e) => e.id === 'beta')).toBe(GATED[1]);
  });

  it('opens everywhere when the field is absent — the permissive default', () => {
    expect(landingEntries('native', GATED).find((e) => e.id === 'beta').stage).toBe('stage-b');
  });

  it('refuses the press too, so the two memories cannot disagree', () => {
    // `nextOpenEntry` over the surface's OWN list, which is what both callers
    // pass: a click judged against the full table would leave the page holding
    // an entry as open that its view model renders as closed.
    expect(nextOpenEntry(null, 'alpha', landingEntries('native', GATED))).toBe(null);
    expect(nextOpenEntry(null, 'alpha', landingEntries('web', GATED))).toBe('alpha');
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

  it('reads the shipped table when given no table at all', () => {
    expect(nextOpenEntry(null, 'new_game')).toBe('new_game');
    expect(nextOpenEntry('new_game', 'new_game')).toBe(null);
    // Load Game replaces it rather than sitting beside it: one stage is open
    // at a time, because there is one middle column (issue #1363).
    expect(nextOpenEntry('new_game', 'load_game')).toBe('load_game');
    expect(nextOpenEntry('load_game', 'load_game')).toBe(null);
    // ...and the entries that have no stage yet stay inert through the default.
    expect(nextOpenEntry(null, 'join_peer')).toBe(null);
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
    expect(vm.entries.map((e) => e.id)).toEqual(LANDING_ENTRIES.map((e) => e.id));
  });

  it('opens New Game off the shipped table', () => {
    const vm = landingViewModel({ openEntryId: 'new_game' });
    expect(vm.stage).toBe('world-picker');
    expect(vm.entries.find((e) => e.id === 'new_game').selected).toBe(true);
    expect(vm.entries.filter((e) => e.inert).map((e) => e.id))
      .toEqual(['join_peer', 'connect_host', 'load_mod_pack']);
  });

  it('opens Load Game onto the save catalogue, borrowing its panel (issue #1363)', () => {
    const vm = landingViewModel({ openEntryId: 'load_game' });
    expect(vm.stage).toBe('save-catalogue');
    expect(vm.docks).toBe('save-slots-panel');
    expect(vm.depth).toBe(1);
    expect(vm.rootClass).toBe('is-open');
    expect(vm.entries.find((e) => e.id === 'load_game').selected).toBe(true);
    // One column, one open stage: New Game is not selected beside it, and the
    // picker's panel is not the one being borrowed.
    expect(vm.entries.find((e) => e.id === 'new_game').selected).toBe(false);
  });

  it('offers Load Game on the native host but cannot open it yet (#1363 AC5)', () => {
    // The recorded gap, not doctrine. The native lobby document carries no
    // `#save-slots-panel`, `HostLobbyBridge` has no channel to fill one from,
    // and native resume is startup-only by design — so the row carries
    // `stagePlatforms: ['web']` until a slice buys those, and NOT
    // `platforms: ['web']`, which would have said native hosts do not load
    // games at all. It renders, it is inert, and pressing it opens nothing.
    const idle = landingViewModel({ platform: 'native' });
    const row = idle.entries.find((e) => e.id === 'load_game');
    expect(row).toBeDefined();
    expect(row.inert).toBe(true);
    expect(row.stage).toBe(null);

    const vm = landingViewModel({ platform: 'native', openEntryId: 'load_game' });
    expect(vm.stage).toBe('idle');
    expect(vm.docks).toBe(null);
    expect(vm.openEntryId).toBe(null);
    // Connect to Host is the contrast, and the reason the two fields are two:
    // that row is not on this menu at all, because a native host is always a
    // host and there is nothing behind it to build.
    expect(idle.entries.map((e) => e.id)).not.toContain('connect_host');
  });
});

describe('landingViewModel — an open entry with a ladder (issue #1362)', () => {
  // New Game asks two questions: choose a World, then choose a hull. The
  // second is not a second menu ENTRY, it is a rung inside the one already
  // open — so the depth, the classes and the stage all come from the row's own
  // `deeper` list, and nothing in the module knows what any of those names
  // mean.

  const open = (deepStage) => landingViewModel({ entries: TABLE, openEntryId: 'alpha', deepStage });

  it('stays at its first rung when the caller names no deeper stage', () => {
    const vm = open(null);
    expect(vm.stage).toBe('stage-a');
    expect(vm.deepStage).toBe(null);
    expect(vm.depth).toBe(1);
    expect(vm.rootClass).toBe('is-open');
  });

  it('reports the deeper stage, one more column along, and says it is deep', () => {
    const vm = open('stage-a2');
    expect(vm.stage).toBe('stage-a2');
    expect(vm.deepStage).toBe('stage-a2');
    expect(vm.depth).toBe(2);
    // Additive, not a replacement: the middle column has to stay on screen for
    // the operator to step back into, so `is-open` is still true.
    expect(vm.rootClass).toBe('is-open is-deep');
  });

  it('counts the rungs, so a second one is a third column and not the same one', () => {
    const vm = open('stage-a3');
    expect(vm.stage).toBe('stage-a3');
    expect(vm.depth).toBe(3);
    expect(vm.rootClass).toBe('is-open is-deep');
  });

  it('ignores a deeper stage this entry does not list, rather than drawing it', () => {
    // The host hands over the PICKER's own stage unfiltered, which is also
    // 'scenario-list' or 'locked' most of the time. A name the row does not
    // know reads as "not deep" — never as a stage nothing can draw.
    for (const name of ['stage-c', 'scenario-list', 'locked', 'nonesuch']) {
      const vm = open(name);
      expect(vm.stage).toBe('stage-a');
      expect(vm.depth).toBe(1);
      expect(vm.deepStage).toBe(null);
    }
  });

  it('ignores it entirely on an entry with no ladder at all', () => {
    const vm = landingViewModel({ entries: TABLE, platform: 'native', openEntryId: 'gamma', deepStage: 'stage-a2' });
    expect(vm.stage).toBe('stage-c');
    expect(vm.depth).toBe(1);
    expect(vm.deepStage).toBe(null);
  });

  it('is not deep with nothing open, whatever the caller says the picker is doing', () => {
    const vm = landingViewModel({ entries: TABLE, deepStage: 'stage-a2' });
    expect(vm.stage).toBe('idle');
    expect(vm.depth).toBe(0);
    expect(vm.rootClass).toBe('is-idle');
  });

  it('is not deep once the landing is dismissed', () => {
    const vm = landingViewModel({ entries: TABLE, openEntryId: 'alpha', deepStage: 'stage-a2', dismissed: true });
    expect(vm.stage).toBe('dismissed');
    expect(vm.depth).toBe(0);
    expect(vm.deepStage).toBe(null);
    expect(vm.rootClass).toBe('is-idle');
  });

  it('says WHICH panel the open entry borrows, as a fact about the row', () => {
    // The renderer moves the node this names and never a node chosen from a
    // comparison of `stage` to a name — a stage-name test would have undocked
    // the picker the moment the hull rung opened, taking the World list with
    // it. An id rather than a boolean since issue #1363, when a second row
    // started borrowing a second panel.
    expect(open(null).docks).toBe('panel-a');
    expect(open('stage-a2').docks).toBe('panel-a');
    expect(landingViewModel({ entries: TABLE, platform: 'native', openEntryId: 'gamma' }).docks).toBe(null);
    expect(landingViewModel({ entries: TABLE }).docks).toBe(null);
    expect(landingViewModel({ entries: TABLE, openEntryId: 'alpha', dismissed: true }).docks).toBe(null);
  });
});

describe('the shipped New Game ladder (issue #1362)', () => {
  it('descends into the picker`s OWN stage name, so the two models share a value', () => {
    // The host passes `scenarioCatalogView().stage` straight through. If this
    // row ever spelled the rung differently, the landing would sit at depth 1
    // while the picker showed hulls, and nothing would say so.
    const vm = landingViewModel({ openEntryId: 'new_game', deepStage: 'ship-picker' });
    expect(vm.stage).toBe('ship-picker');
    expect(vm.depth).toBe(2);
    expect(vm.rootClass).toBe('is-open is-deep');
    expect(vm.docks).toBe('scenario-panel');
  });

  it('stays on the World picker for every other stage the picker reports', () => {
    for (const stage of ['scenario-list', 'scenario-empty', 'ship-auto', 'locked']) {
      const vm = landingViewModel({ openEntryId: 'new_game', deepStage: stage });
      expect(vm.stage).toBe('world-picker');
      expect(vm.depth).toBe(1);
    }
  });

  it('descends on both platforms, because both reach the same picker', () => {
    for (const platform of ['web', 'native']) {
      const vm = landingViewModel({ platform, openEntryId: 'new_game', deepStage: 'ship-picker' });
      expect(vm.depth).toBe(2);
    }
  });

  it('gives no other shipped entry a ladder yet, which is the honest tracer', () => {
    const withLadders = LANDING_ENTRIES
      .filter((e) => Array.isArray(e.deeper) && e.deeper.length)
      .map((e) => e.id);
    expect(withLadders).toEqual(['new_game']);
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
    expect(native).toEqual(['new_game', 'load_game', 'join_peer', 'load_mod_pack']);
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
    expect(vm.entries.map((e) => e.ordinal)).toEqual(['01', '02', '03', '04']);
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
