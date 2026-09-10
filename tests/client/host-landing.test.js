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
import { readFileSync } from 'node:fs';
import { buildTable } from '../../gui/strings.js';
import {
  LANDING_ENTRIES,
  CONFIRM_CANCEL_ID,
  landingEntries,
  nextOpenEntry,
  landingViewModel,
  landingJoinAttempt,
} from '../../gui/host-landing-view.js';

/**
 * The authored join-code format, as `landingJoinAttempt` is handed it.
 *
 * A fabricated table rather than the shipped JSON, for the reason this file
 * owns its own entry TABLE: what is under test is the decision, and a suite
 * that read the shipped format would start failing the day a designer renamed
 * a denied word. The two namespace GUIDs are this fixture's own and the
 * suffix rules are the shipped shape — eight letters, `0`→`O`, `1`→`I`.
 */
const JOIN_DATA = {
  format_version: 1,
  namespaces: {
    client: '11111111-1111-4111-8111-111111111111',
    server: '22222222-2222-4222-8222-222222222222',
  },
  version: { guid: '33333333-3333-4333-8333-333333333333' },
  suffix: {
    length: 8,
    alphabet: 'ABCDEFGHIJKMNOPQRSTUVWXYZ',
    strip: ' -_',
    normalise: { 0: 'O', 1: 'I', L: 'I' },
  },
  deny: ['BADWORD'],
};

/** The shipped Join as Peer / Connect to Host descriptors. */
const joinOf = (id) => LANDING_ENTRIES.find((e) => e.id === id).join;

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
  it('offers the seven entries drawn so far, in order', () => {
    expect(LANDING_ENTRIES.map((e) => e.id)).toEqual([
      'new_game', 'load_game', 'host_gm', 'join_peer', 'connect_host',
      'load_mod_pack', 'exit_desktop',
    ]);
  });

  it('puts Host as GM between the host routes and the join routes, where it belongs', () => {
    // It is BOTH: a host that opens its own session, and the game master
    // profile. Sitting next to Join as Peer is what says the two GM routes are
    // the same destination reached two ways — one that IS the session, one that
    // joins somebody else's.
    const ids = LANDING_ENTRIES.map((e) => e.id);
    expect(ids.indexOf('host_gm')).toBeGreaterThan(ids.indexOf('new_game'));
    expect(ids.indexOf('host_gm')).toBeLessThan(ids.indexOf('join_peer'));
  });

  it('gives Host as GM New Game`s own stage, ladder and panel — to the letter', () => {
    // The load-bearing claim: a standalone game master answers the SAME two
    // questions a host answers, in the same columns, through the same
    // `#scenario-panel`. A second picker would be a second answer to "what is
    // this session flying".
    const host = LANDING_ENTRIES.find((e) => e.id === 'new_game');
    const gm = LANDING_ENTRIES.find((e) => e.id === 'host_gm');
    expect(gm.stage).toBe(host.stage);
    expect(gm.docks).toBe(host.docks);
    expect(gm.deeper).toEqual(host.deeper);
    // ...and it carries no `join`: this route opens a session, it does not
    // type a code at one.
    expect(gm.join).toBeUndefined();
    expect(gm.confirm).toBeUndefined();
    expect(gm.statusId).toBe('server.landing.status_host_gm');
  });

  it('gives every row a stage, now that all six slices have landed', () => {
    // The honest shape of a tracer: a row without a stage has a sibling issue
    // that gives it one, and until then it must not pretend. Load Game joined
    // in issue #1363, both join routes in #1364, Exit to Desktop in #1365 and
    // Load mod pack in #1366 — each by growing a stage on its own row rather
    // than by anything below it learning its name, which is what the six
    // landing in one table without a switch was supposed to cost.
    //
    // A row can still be INERT, and three of them are on some surface or some
    // run — but through `needs`, `stagePlatforms` or `stagePreBoot`, each
    // tested below, rather than through a missing stage.
    expect(LANDING_ENTRIES.filter((e) => e.stage).map((e) => e.id)).toEqual([
      'new_game', 'load_game', 'host_gm', 'join_peer', 'connect_host',
      'load_mod_pack', 'exit_desktop',
    ]);
    expect(LANDING_ENTRIES.filter((e) => !e.stage)).toEqual([]);
  });

  it('names the panel each staged row borrows, as an element id on the row', () => {
    // Three borrowers now, which is why the field is an id rather than the
    // boolean it was while only the picker docked: the renderer reads the row
    // and moves the node it names, and a fourth borrower needs no new branch.
    // The two join routes name the SAME node, because they ask one question
    // and differ only in what a good answer means (issue #1364).
    expect(LANDING_ENTRIES.filter((e) => e.docks).map((e) => [e.id, e.docks])).toEqual([
      ['new_game', 'scenario-panel'],
      ['load_game', 'save-slots-panel'],
      ['host_gm', 'scenario-panel'],
      ['join_peer', 'landing-join-panel'],
      ['connect_host', 'landing-join-panel'],
    ]);
  });

  it('gives both join routes one stage name and their own join descriptor', () => {
    // The load-bearing claim of issue #1364: what makes Join as Peer different
    // from Connect to Host is DATA on the row — which typed namespace a bare
    // suffix composes into, which surface words a refusal, and which action a
    // good code runs — and not a branch in the view model, the renderer or the
    // stylesheet, all three of which see one `join-code` stage.
    const peer = LANDING_ENTRIES.find((e) => e.id === 'join_peer');
    const client = LANDING_ENTRIES.find((e) => e.id === 'connect_host');
    expect([peer.stage, client.stage]).toEqual(['join-code', 'join-code']);
    expect(peer.join.namespace).toBe('server');
    expect(peer.join.surface).toBe('server');
    expect(peer.join.action).toBe('boot-game-master');
    expect(client.join.namespace).toBe('client');
    expect(client.join.surface).toBe('client');
    expect(client.join.action).toBe('open-client-page');
    // Two routes, two actions: a shared action would be one of them quietly
    // doing the other's job.
    expect(peer.join.action).not.toBe(client.join.action);
  });

  it('gives every join row the three string ids its panel is written from', () => {
    for (const entry of LANDING_ENTRIES.filter((e) => e.join)) {
      for (const key of ['roleId', 'blurbId', 'submitId']) {
        expect(typeof entry.join[key], `${entry.id}.${key}`).toBe('string');
        expect(entry.join[key].length).toBeGreaterThan(0);
      }
    }
  });

  it('records both native gaps as unserved STAGES, not as unoffered rows', () => {
    // #1363's AC5 and #1364's native half are unbuilt, and this is where that
    // is written down. The two fields say different things: `platforms` is what
    // a surface does not offer (Connect to Host, for ever, because a native
    // host has no join leg), `stagePlatforms` is what it cannot open yet — Load
    // Game until the native save-catalogue channel lands, Join as Peer until
    // something on that surface can answer a Game Master profile at all.
    const load = LANDING_ENTRIES.find((e) => e.id === 'load_game');
    expect(load.platforms).toEqual(['web', 'native']);
    expect(load.stagePlatforms).toEqual(['web']);
    const peer = LANDING_ENTRIES.find((e) => e.id === 'join_peer');
    expect(peer.platforms).toEqual(['web', 'native']);
    expect(peer.stagePlatforms).toEqual(['web']);
    const connect = LANDING_ENTRIES.find((e) => e.id === 'connect_host');
    expect(connect.platforms).toEqual(['web']);
    expect(connect.stagePlatforms).toBeUndefined();
    // Nothing else in the shipped table is gated this way; a third would be a
    // third unfinished AC and should arrive with its own test.
    expect(LANDING_ENTRIES.filter((e) => e.stagePlatforms).map((e) => e.id))
      .toEqual(['load_game', 'host_gm', 'join_peer']);
    // Host as GM is the third, and the same KIND of gap as Join as Peer's: the
    // Game Master profile is a browser profile chosen inside `wasm_init`, and
    // the native host composes its app before any landing row can ask for one.
    const hostGm = LANDING_ENTRIES.find((e) => e.id === 'host_gm');
    expect(hostGm.platforms).toEqual(['web', 'native']);
    expect(hostGm.stagePlatforms).toEqual(['web']);
  });

  it('marks both join routes as pre-boot stages, and nothing else', () => {
    // Issue #1364's blocking constraint, written on the rows that carry it:
    // the Game Master profile is read once inside `wasm_init`, and Connect to
    // Host leaves the page altogether — neither survives a world being loaded,
    // and the landing DOES come back over one (issue #756). New Game and Load
    // Game are the routes round two exists for, so they must not pick this up.
    expect(LANDING_ENTRIES.filter((e) => e.stagePreBoot).map((e) => e.id))
      .toEqual(['host_gm', 'join_peer', 'connect_host']);
    // Every row with a code field is one of them: a third join route arriving
    // without this is a third way to swap a profile that cannot be swapped.
    expect(LANDING_ENTRIES.filter((e) => e.join).map((e) => e.id))
      .toEqual(['join_peer', 'connect_host']);
  });

  it('makes Load mod pack need a SHELF *and* a native host, saying both', () => {
    // TWO availability fields on one row, and neither covering for the other.
    //
    // `platforms: ['native']` is doctrine about the build: this stage is a
    // scanned FOLDER and a browser has none. The web host is not missing a
    // mod-pack door either — `#mod-pack-upload` (issue #760) is a working file
    // picker that rides into the landing's middle column with `#scenario-panel`
    // — so a row rendered for ever dashed there would have been a second door
    // onto one idea with the front one nailed shut.
    //
    // `needs: 'packs'` is the rule `platforms` could not have expressed: the
    // same native binary offers a folder when it was started with
    // --mod-pack-dir and none when it was not, which is not a fact about the
    // build.
    const row = LANDING_ENTRIES.find((e) => e.id === 'load_mod_pack');
    expect(row.stage).toBe('mod-packs');
    expect(row.needs).toBe('packs');
    expect(row.platforms).toEqual(['native']);
    // …and its verb is named once, on the row that owns it — the same
    // arrangement `exit_desktop`'s `confirm.action` makes, and deliberately the
    // same token as the record the native surface sends.
    expect(row.action).toBe('install_mod_pack');
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

  it('marks the one shelf-shaped row as a shelf, on the row rather than by name', () => {
    // The exact sibling of the confirmation case above, and the reason the view
    // model can publish `packs` without comparing a stage to the literal
    // 'mod-packs'. A second folder-shaped route — saved sessions, scenario
    // manifests — is then one more row here and no new branch anywhere.
    expect(LANDING_ENTRIES.filter((e) => e.shelf).map((e) => e.id))
      .toEqual(['load_mod_pack']);
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

describe('landingEntries — a stage this surface can no longer open', () => {
  // `stagePreBoot` is the third member of the family, and the one that is about
  // TIME rather than platform: the landing comes back after a Game Over with a
  // world already loaded (issue #756), and a route that decides a BOOT profile
  // has nothing left to decide by then. Same strip, same vocabulary — a second
  // way of saying "inert" is a second answer for `nextOpenEntry` to disagree
  // with.
  const TIMED = [
    { id: 'alpha', labelId: 'x.a', descId: 'x.a_desc', stage: 'stage-a', docks: 'panel-a', deeper: ['stage-a2'], join: { namespace: 'server' }, stagePreBoot: true },
    { id: 'beta', labelId: 'x.b', descId: 'x.b_desc', stage: 'stage-b' },
  ];

  it('keeps the row and takes its stage away once a world has booted', () => {
    const booted = landingEntries('web', TIMED, { booted: true });
    expect(booted.map((e) => e.id)).toEqual(['alpha', 'beta']);
    const alpha = booted.find((e) => e.id === 'alpha');
    expect(alpha.stage).toBe(null);
    expect(alpha.docks).toBe(null);
    expect(alpha.deeper).toBe(null);
    // The code field goes with the stage: there is nothing to type into.
    expect(alpha.join).toBe(null);
    // ...and the shipped row is untouched, as it is for the platform gap.
    expect(TIMED[0].stage).toBe('stage-a');
  });

  it('leaves every row alone before the boot, and with no options at all', () => {
    expect(landingEntries('web', TIMED, { booted: false }).find((e) => e.id === 'alpha'))
      .toBe(TIMED[0]);
    expect(landingEntries('web', TIMED).find((e) => e.id === 'alpha')).toBe(TIMED[0]);
  });

  it('leaves a row without the field open after the boot — the permissive default', () => {
    expect(landingEntries('web', TIMED, { booted: true }).find((e) => e.id === 'beta').stage)
      .toBe('stage-b');
  });

  it('refuses the press too, so the two memories cannot disagree', () => {
    expect(nextOpenEntry(null, 'alpha', landingEntries('web', TIMED, { booted: true })))
      .toBe(null);
    expect(nextOpenEntry(null, 'alpha', landingEntries('web', TIMED))).toBe('alpha');
  });
});

describe('a booted surface offers no join stage', () => {
  // The shipped claim behind issue #1364's "no runtime Boot Profile swap", and
  // the case the first landing cannot make: `is_browser_gm` is read once,
  // inside `wasm_init`, so on the landing that RETURNS after a Game Over both
  // join routes must be unopenable — one because the profile it sets would
  // never be read, the other because leaving the page would discard the world
  // behind it.
  it('marks both join rows inert, and leaves New Game and Load Game alone', () => {
    const vm = landingViewModel({ platform: 'web', booted: true });
    const inert = Object.fromEntries(vm.entries.map((e) => [e.id, e.inert]));
    expect(inert.join_peer).toBe(true);
    expect(inert.connect_host).toBe(true);
    // Host as GM goes with them, and for the same reason: the profile is read
    // once inside `wasm_init`, so the request this route commits is a dead
    // letter on the landing that comes back over a loaded world.
    expect(inert.host_gm).toBe(true);
    expect(inert.new_game).toBe(false);
    expect(inert.load_game).toBe(false);
    // The menu is not one row shorter: an operator finds the route where they
    // left it, saying it cannot be taken now.
    expect(vm.entries.map((e) => e.id)).toEqual(
      landingViewModel({ platform: 'web' }).entries.map((e) => e.id),
    );
  });

  it('renders no code field even when a join row is remembered as open', () => {
    // The page's open-entry memory outlives a round: `_landingOpenEntry` is
    // this page's, and a stage the surface can no longer open reads as closed
    // rather than as a field that would run the action behind it.
    const vm = landingViewModel({ platform: 'web', openEntryId: 'join_peer', booted: true });
    expect(vm.openEntryId).toBe(null);
    expect(vm.join).toBe(null);
    expect(vm.docks).toBe(null);
    expect(vm.stage).toBe('idle');
  });

  it('opens Host as GM before the boot and never after it', () => {
    expect(landingViewModel({ platform: 'web', openEntryId: 'host_gm' }).stage)
      .toBe('world-picker');
    const after = landingViewModel({ platform: 'web', openEntryId: 'host_gm', booted: true });
    expect(after.openEntryId).toBe(null);
    expect(after.docks).toBe(null);
    expect(after.stage).toBe('idle');
  });

  it('still opens both of them before the boot, which is the whole of the slice', () => {
    for (const id of ['join_peer', 'connect_host']) {
      const vm = landingViewModel({ platform: 'web', openEntryId: id });
      expect(vm.openEntryId).toBe(id);
      expect(vm.stage).toBe('join-code');
      expect(vm.join.action).toBe(joinOf(id).action);
    }
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
    // Both join routes open the same stage, and opening one still CLOSES the
    // other: one middle column, one open stage (issue #1364).
    expect(nextOpenEntry(null, 'join_peer')).toBe('join_peer');
    expect(nextOpenEntry('join_peer', 'connect_host')).toBe('connect_host');
    expect(nextOpenEntry('join_peer', 'join_peer')).toBe(null);
    // ...and the entry that has no stage yet stays inert through the default.
    expect(nextOpenEntry(null, 'load_mod_pack')).toBe(null);
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

  it('opens New Game off the shipped table', () => {
    const vm = landingViewModel({ openEntryId: 'new_game' });
    expect(vm.stage).toBe('world-picker');
    expect(vm.entries.find((e) => e.id === 'new_game').selected).toBe(true);
    // Nothing on the WEB menu is inert on a pre-boot landing: every row this
    // host offers is a row it can open. Load mod pack used to sit here dashed
    // for ever; it is native-only now, because the browser's mod-pack door is
    // the live upload control inside the picker.
    expect(vm.entries.filter((e) => e.inert).map((e) => e.id)).toEqual([]);
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
    // Two now, and they are the same ladder: Host as GM descends into the hull
    // column exactly as New Game does, because a standalone game master picks
    // the hull its own AI crew flies.
    expect(withLadders).toEqual(['new_game', 'host_gm']);
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
      'new_game', 'load_game', 'host_gm', 'join_peer', 'load_mod_pack',
      'exit_desktop',
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
    expect(vm.entries.map((e) => e.ordinal))
      .toEqual(['01', '02', '03', '04', '05', '06']);
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

describe('landingJoinAttempt — judging a typed code (issue #1364)', () => {
  // AC4: "a refused code explains what is wrong with it, reusing the existing
  // join-code error strings". Every case below asserts a `strings.csv` id that
  // already existed before this slice — a new sentence here would be a second
  // wording for a failure the phone already words.
  const peer = () => joinOf('join_peer');
  const client = () => joinOf('connect_host');

  it('accepts a bare suffix, composing it into the ROW’s namespace', () => {
    const attempt = landingJoinAttempt(peer(), 'quarking', JOIN_DATA);
    expect(attempt.ok).toBe(true);
    expect(attempt.namespace).toBe('server');
    expect(attempt.action).toBe('boot-game-master');
    // The structured code, not the letters: both legs downstream want the whole
    // identifier, and composing it once here is what stops each of them doing
    // it differently.
    expect(attempt.code).toBe(
      '22222222-2222-4222-8222-222222222222_33333333-3333-4333-8333-333333333333_QUARKING',
    );
  });

  it('composes the SAME letters into the client namespace on the other route', () => {
    // One field, two meanings, and the meaning is the row's. This is the whole
    // reason the namespace is data rather than a constant in the page.
    const attempt = landingJoinAttempt(client(), 'quarking', JOIN_DATA);
    expect(attempt.ok).toBe(true);
    expect(attempt.namespace).toBe('client');
    expect(attempt.action).toBe('open-client-page');
    expect(attempt.code.startsWith('11111111-')).toBe(true);
  });

  it('canonicalises what a player typed, exactly as the phone’s field does', () => {
    // Lower case, the spacing somebody adds reading a code aloud, and the
    // confusable digits. Not re-implemented here — this is gui/join-code.js
    // answering, which is the point of importing it rather than parsing again.
    const attempt = landingJoinAttempt(peer(), ' quark-1ng ', JOIN_DATA);
    expect(attempt.ok).toBe(true);
    expect(attempt.code.endsWith('_QUARKING')).toBe(true);
  });

  it('reports an empty field, a wrong length, a bad character and a denied word', () => {
    expect(landingJoinAttempt(peer(), '', JOIN_DATA).errorId).toBe('client.join.error_empty');
    expect(landingJoinAttempt(peer(), 'QUARK', JOIN_DATA).errorId).toBe('client.join.error_length');
    expect(landingJoinAttempt(peer(), 'QUARKIN$', JOIN_DATA).errorId)
      .toBe('client.join.error_charset');
    expect(landingJoinAttempt(peer(), 'BADWORDS', JOIN_DATA).errorId)
      .toBe('client.join.error_denied');
  });

  it('reports a paste that lost a part as malformed rather than as letters', () => {
    const half = '22222222-2222-4222-8222-222222222222_QUARKING';
    expect(landingJoinAttempt(peer(), half, JOIN_DATA).errorId)
      .toBe('client.join.error_malformed');
  });

  it('refuses a CREW code pasted into Join as Peer, in the HOST’s wording', () => {
    // The refusal that has to be worded per surface (issue #1114): read out on
    // a viewscreen, the phone's sentence for this is the exact inverse of what
    // happened. The row carries `surface: 'server'` so it is not.
    const crew = '11111111-1111-4111-8111-111111111111'
      + '_33333333-3333-4333-8333-333333333333_QUARKING';
    expect(landingJoinAttempt(peer(), crew, JOIN_DATA).errorId)
      .toBe('server.fleet.error_wrong_type');
  });

  it('refuses a FLEET code pasted into Connect to Host, in the phone’s wording', () => {
    const fleet = '22222222-2222-4222-8222-222222222222'
      + '_33333333-3333-4333-8333-333333333333_QUARKING';
    expect(landingJoinAttempt(client(), fleet, JOIN_DATA).errorId)
      .toBe('client.join.error_wrong_type');
  });

  it('refuses a GUID belonging to no Phoenix namespace as not-Phoenix', () => {
    const alien = '44444444-4444-4444-8444-444444444444'
      + '_33333333-3333-4333-8333-333333333333_QUARKING';
    expect(landingJoinAttempt(peer(), alien, JOIN_DATA).errorId)
      .toBe('client.join.error_not_phoenix');
  });

  it('says the SERVICE is unreachable when the authored table never loaded', () => {
    // Not a refusal of the code — it was never judged. Sending an operator back
    // to retype letters that were already right is the one failure the reason
    // map exists to prevent.
    expect(landingJoinAttempt(peer(), 'QUARKING', null).errorId)
      .toBe('client.join.error_unreachable');
    expect(landingJoinAttempt(null, 'QUARKING', JOIN_DATA).errorId)
      .toBe('client.join.error_unreachable');
  });

  it('answers a table it cannot read rather than throwing at the page', () => {
    // A half-fetched or malformed format table makes gui/join-code.js throw
    // rather than half-answer, deliberately. The landing turns that into the
    // sentence an unreachable service gets, because the operator's remedy is
    // identical — and because a throw here would abandon a click handler
    // mid-way and leave the panel saying nothing at all.
    expect(landingJoinAttempt(peer(), 'QUARKING', { format_version: 1 }).errorId)
      .toBe('client.join.error_unreachable');
  });
});

describe('landingViewModel — the join stage (issue #1364)', () => {
  it('carries the open row’s join descriptor, and null on every other stage', () => {
    expect(landingViewModel({ openEntryId: 'join_peer' }).join.action)
      .toBe('boot-game-master');
    expect(landingViewModel({ openEntryId: 'connect_host' }).join.action)
      .toBe('open-client-page');
    expect(landingViewModel({ openEntryId: 'new_game' }).join).toBe(null);
    expect(landingViewModel({ openEntryId: 'load_game' }).join).toBe(null);
    expect(landingViewModel({}).join).toBe(null);
  });

  it('borrows one panel for both routes, under one stage name', () => {
    for (const id of ['join_peer', 'connect_host']) {
      const vm = landingViewModel({ openEntryId: id });
      expect(vm.stage).toBe('join-code');
      expect(vm.docks).toBe('landing-join-panel');
      expect(vm.depth).toBe(1);
      expect(vm.rootClass).toBe('is-open');
    }
  });

  it('fills in what the FIELD says, which is the same on every join route', () => {
    // "Join code", "eight letters", "or paste the whole code" are facts about
    // the authored format and not about the route, so both rows get them and
    // neither repeats them.
    for (const id of ['join_peer', 'connect_host']) {
      const vm = landingViewModel({ openEntryId: id });
      expect(vm.join.labelId).toBe('server.landing.join_code_label');
      expect(vm.join.placeholderId).toBe('server.landing.join_code_placeholder');
      expect(vm.join.hintId).toBe('server.landing.join_code_hint');
    }
    // ...while what the ROUTE says is the row's own.
    expect(landingViewModel({ openEntryId: 'join_peer' }).join.roleId)
      .toBe('server.landing.join_peer_role');
    expect(landingViewModel({ openEntryId: 'connect_host' }).join.roleId)
      .toBe('server.landing.connect_host_role');
  });

  it('lets a row override a field default rather than being overwritten by it', () => {
    // The merge order, pinned: the shared defaults go UNDER the row. A route
    // whose code is not eight letters has to be able to say so.
    const own = [{
      id: 'own',
      labelId: 'x',
      descId: 'y',
      stage: 'join-code',
      join: { namespace: 'server', surface: 'server', action: 'a', hintId: 'x.own_hint' },
    }];
    expect(landingViewModel({ entries: own, openEntryId: 'own' }).join.hintId)
      .toBe('x.own_hint');
  });

  it('folds the caller’s last refusal onto it, and carries null when there is none', () => {
    const refused = landingViewModel({
      openEntryId: 'join_peer', joinErrorId: 'client.join.error_length',
    });
    expect(refused.join.errorId).toBe('client.join.error_length');
    expect(landingViewModel({ openEntryId: 'join_peer' }).join.errorId).toBe(null);
  });

  it('says a joining host is NOT hosting, from a field on the row', () => {
    // A peer awaiting a code has opened no session of its own, and saying so is
    // `statusId` on the row rather than a comparison of the open id to a name.
    expect(landingViewModel({ openEntryId: 'join_peer' }).status.sessionId)
      .toBe('server.landing.status_peer');
    expect(landingViewModel({ openEntryId: 'connect_host' }).status.sessionId)
      .toBe('server.landing.status_client');
    // The routes that ARE hosting keep the default, so the field is worn only
    // by the rows that need it.
    expect(landingViewModel({ openEntryId: 'new_game' }).status.sessionId)
      .toBe('server.landing.status_hosting');
    expect(landingViewModel({ openEntryId: 'load_game' }).status.sessionId)
      .toBe('server.landing.status_hosting');
  });

  it('takes the join descriptor away with the stage where it cannot be served', () => {
    // Join as Peer is offered on the native menu and cannot be opened there:
    // the Game Master profile is a BROWSER profile. A row whose stage is gone
    // must not still carry a code field for something to find.
    const native = landingEntries('native', LANDING_ENTRIES)
      .find((e) => e.id === 'join_peer');
    expect(native.stage).toBe(null);
    expect(native.docks).toBe(null);
    expect(native.join).toBe(null);
    const vm = landingViewModel({ platform: 'native', openEntryId: 'join_peer' });
    expect(vm.stage).toBe('idle');
    expect(vm.join).toBe(null);
    // ...and the shipped row is untouched: the copy is the surface's view of it.
    expect(joinOf('join_peer').action).toBe('boot-game-master');
  });
});

// ── The mod-pack shelf (issue #1366) ────────────────────────────────────────
//
// Two separable claims, and they are separable on purpose:
//
//   1. `needs`/`provides` is a GENERAL rule over the table, exercised through a
//      table this suite owns, so it cannot be satisfied by a special case for
//      the one shipped row that uses it;
//   2. the shelf stage is a pure function of the host's snapshot plus the one
//      thing the host does not know — which row the operator highlighted.

/** A table this suite owns, whose one interesting row needs something. */
const NEEDY = [
  { id: 'plain', labelId: 'x.p', descId: 'x.p_desc', stage: 'plain-stage' },
  {
    id: 'shelf',
    labelId: 'x.s',
    descId: 'x.s_desc',
    stage: 'mod-packs',
    needs: 'packs',
    // What makes the view model publish a shelf: a field on the ROW. The stage
    // name here is deliberately the shipped one, so a branch that had gone back
    // to reading the name would still pass — and the case below, whose row
    // names a stage of its own, is what would fail.
    shelf: true,
    action: 'x_install',
  },
];

/** A host snapshot in the shape `native_host::host_lobby::packs` encodes. */
const SHELF = {
  dir: 'mods',
  offered: [
    { file: 'thin-margin.zip', label: 'thin-margin' },
    { file: 'borrowed-sun.zip', label: 'borrowed-sun' },
  ],
  installed: [{ id: 'thin-margin', name: 'Thin Margin', version: '1.2' }],
  attempted: null,
  accepted: false,
  findings: [],
  conflicts: [],
};

describe('a row that NEEDS something the surface must provide', () => {
  it('is inert until the surface says it can answer it', () => {
    const without = landingViewModel({ entries: NEEDY });
    expect(without.entries.find((e) => e.id === 'shelf').inert).toBe(true);
    const with_ = landingViewModel({ entries: NEEDY, provides: ['packs'] });
    expect(with_.entries.find((e) => e.id === 'shelf').inert).toBe(false);
  });

  it('is one word for two reasons, because they look the same on screen', () => {
    // A route with no stage yet and a route this host cannot answer are both
    // "renders, and pressing it changes nothing".
    const vm = landingViewModel({ entries: NEEDY });
    expect(vm.entries.map((e) => [e.id, e.inert]))
      .toEqual([['plain', false], ['shelf', true]]);
  });

  it('will not open through nextOpenEntry while the need is unmet', () => {
    expect(nextOpenEntry(null, 'shelf', NEEDY)).toBe(null);
    expect(nextOpenEntry('plain', 'shelf', NEEDY)).toBe('plain');
    expect(nextOpenEntry(null, 'shelf', NEEDY, ['packs'])).toBe('shelf');
    expect(nextOpenEntry('shelf', 'shelf', NEEDY, ['packs'])).toBe(null);
  });

  it('reads as closed when a remembered open id needs what this run cannot give', () => {
    // A page reloaded against a host restarted without --mod-pack-dir. The
    // caller's memory outlives the RUN as well as the menu, and opening a shelf
    // stage with no shelf behind it would be worse than the front door.
    const vm = landingViewModel({ entries: NEEDY, openEntryId: 'shelf' });
    expect(vm.stage).toBe('idle');
    expect(vm.openEntryId).toBe(null);
    expect(vm.packs).toBe(null);
  });

  it('keeps the shipped Load mod pack row off the web and inert without a shelf', () => {
    // Both of its availability fields, on the shipped row, from the outside.
    // The web host is not offered it at all — its mod-pack door is the live
    // `#mod-pack-upload` control inside the picker, and a dashed second door
    // beside a working one is the thing the doctrine forbids.
    const web = landingViewModel();
    expect(web.entries.find((e) => e.id === 'load_mod_pack')).toBeUndefined();
    // The native host IS offered it, and it is inert until that RUN says it
    // scanned a folder — which is the half `platforms` could not have said.
    const native = landingViewModel({ platform: 'native' });
    expect(native.entries.find((e) => e.id === 'load_mod_pack').inert).toBe(true);
    const shelved = landingViewModel({ platform: 'native', provides: ['packs'] });
    expect(shelved.entries.find((e) => e.id === 'load_mod_pack').inert).toBe(false);
    // ...and the press is judged over the same two fields, so the memory and
    // the render cannot disagree.
    expect(nextOpenEntry(null, 'load_mod_pack')).toBe(null);
    expect(nextOpenEntry(null, 'load_mod_pack', landingEntries('native'), ['packs']))
      .toBe('load_mod_pack');
  });
});

describe('landingViewModel — the mod-pack shelf stage (issue #1366)', () => {
  const open = (extra) => landingViewModel(Object.assign({
    entries: NEEDY,
    provides: ['packs'],
    openEntryId: 'shelf',
    packs: SHELF,
  }, extra || {}));

  it('is null unless the row that needs it is the open one', () => {
    // The exact sibling of `confirm`: the renderer draws the stage from its
    // presence, and never from a stage name or an entry id.
    expect(landingViewModel({ entries: NEEDY, provides: ['packs'], packs: SHELF }).packs)
      .toBe(null);
    expect(open().packs).not.toBe(null);
    expect(open({ openEntryId: 'plain' }).packs).toBe(null);
  });

  it('is decided by the row`s own `shelf` field and never by the stage`s name', () => {
    // The claim the comment over `packs:` makes, tested the only way it can be:
    // a shelf-shaped row whose stage is called something ELSE still gets a
    // shelf, and a row called 'mod-packs' that never declared one does not.
    // Until this was true, the module compared a stage to a literal in the one
    // place it claimed nothing did.
    const renamed = [{
      id: 'folder',
      labelId: 'x.f',
      descId: 'x.f_desc',
      stage: 'saved-sessions',
      needs: 'packs',
      shelf: true,
      action: 'x_install',
    }];
    const vm = landingViewModel({
      entries: renamed, provides: ['packs'], openEntryId: 'folder', packs: SHELF,
    });
    expect(vm.stage).toBe('saved-sessions');
    expect(vm.packs).not.toBe(null);
    expect(vm.packs.rows.map((r) => r.file))
      .toEqual(['thin-margin.zip', 'borrowed-sun.zip']);

    const unshelved = [{
      id: 'named',
      labelId: 'x.n',
      descId: 'x.n_desc',
      stage: 'mod-packs',
      needs: 'packs',
    }];
    expect(landingViewModel({
      entries: unshelved, provides: ['packs'], openEntryId: 'named', packs: SHELF,
    }).packs).toBe(null);
  });

  it('lists what the host offered, in the order it offered it', () => {
    expect(open().packs.rows.map((r) => [r.file, r.label, r.selected])).toEqual([
      ['thin-margin.zip', 'thin-margin', false],
      ['borrowed-sun.zip', 'borrowed-sun', false],
    ]);
  });

  it('names the folder even when there is nothing in it', () => {
    // "There are no packs here" and "this host was never given a folder" ask
    // the operator to do different things, so the panel always says which.
    const vm = open({ packs: Object.assign({}, SHELF, { offered: [] }) });
    expect(vm.packs.folder).toEqual({
      id: 'server.landing.packs.folder',
      params: { dir: 'mods' },
    });
    expect(vm.packs.emptyId).toBe('server.landing.packs.empty');
    expect(vm.packs.scanError).toBe(null);
  });

  it('says a folder it could not READ is a different emptiness', () => {
    const vm = open({
      packs: Object.assign({}, SHELF, { offered: [], scan_error: 'mods: not found' }),
    });
    expect(vm.packs.emptyId).toBe('server.landing.packs.scan_failed');
    // The host's own sentence about the operator's own path rides beside the
    // id rather than inside it: no string table could hold it.
    expect(vm.packs.scanError).toBe('mods: not found');
  });

  it('installs nothing until a row is highlighted', () => {
    expect(open().packs.ctaEnabled).toBe(false);
    expect(open().packs.chosen).toBe(null);
    const chosen = open({ chosenPack: 'borrowed-sun.zip' });
    expect(chosen.packs.ctaEnabled).toBe(true);
    expect(chosen.packs.chosen).toBe('borrowed-sun.zip');
    expect(chosen.packs.rows.map((r) => r.selected)).toEqual([false, true]);
  });

  it('drops a highlight the host is no longer offering', () => {
    // The shelf is rescanned on every attempt, so an archive can leave the
    // folder between the click that chose it and the render that draws it.
    const vm = open({ chosenPack: 'deleted-since.zip' });
    expect(vm.packs.chosen).toBe(null);
    expect(vm.packs.ctaEnabled).toBe(false);
  });

  it('carries the OPEN ROW’s verb, so no caller keeps a mapping table', () => {
    expect(open().packs.action).toBe('x_install');
  });

  it('says what is wrong when a pack was refused, in the validator’s own words', () => {
    const vm = open({
      packs: Object.assign({}, SHELF, {
        attempted: 'broken.zip',
        accepted: false,
        findings: [{
          severity: 'error',
          category: 'missing-manifest',
          message: 'mod pack is missing its required scenarios.toml manifest',
          file: 'scenarios.toml',
        }],
      }),
    });
    expect(vm.packs.outcome).toEqual({
      tone: 'bad',
      line: { id: 'server.landing.packs.refused', params: { pack: 'broken.zip' } },
    });
    expect(vm.packs.findingsHeadingId).toBe('server.landing.packs.findings_heading');
    expect(vm.packs.findings).toEqual([{
      tone: 'bad',
      labelId: 'server.landing.packs.severity_error',
      category: 'missing-manifest',
      message: 'mod pack is missing its required scenarios.toml manifest',
      file: 'scenarios.toml',
    }]);
  });

  it('reports a warning without calling the install a failure', () => {
    const vm = open({
      packs: Object.assign({}, SHELF, {
        attempted: 'ok.zip',
        accepted: true,
        findings: [{
          severity: 'warning',
          category: 'overlapping-pack-path',
          message: 'ok shadows thin-margin for assets/entities/x.toml',
          file: 'scenarios.toml',
        }],
      }),
    });
    expect(vm.packs.outcome.tone).toBe('ok');
    expect(vm.packs.outcome.line.id).toBe('server.landing.packs.accepted');
    expect(vm.packs.findings[0].tone).toBe('warn');
    expect(vm.packs.findings[0].labelId).toBe('server.landing.packs.severity_warning');
  });

  it('reports nothing at all before the first attempt', () => {
    // A freshly opened shelf must not claim a success nobody asked for.
    expect(open().packs.outcome).toBe(null);
    expect(open().packs.findingsHeadingId).toBe(null);
  });

  it('names which pack won a path two of them carry', () => {
    // The acceptance criterion, and the reason it matters: two packs that both
    // replace one hull produce one hull, and an operator who cannot see which
    // is flying has no way to work out why their change did nothing.
    const vm = open({
      packs: Object.assign({}, SHELF, {
        conflicts: [{
          path: 'assets/entities/alliance_destroyer.toml',
          winner: 'borrowed-sun',
          losers: ['thin-margin'],
        }],
      }),
    });
    expect(vm.packs.conflictsHeadingId).toBe('server.landing.packs.conflict_heading');
    expect(vm.packs.conflicts[0].line).toEqual({
      id: 'server.landing.packs.conflict_line',
      params: {
        path: 'assets/entities/alliance_destroyer.toml',
        winner: 'borrowed-sun',
        losers: 'thin-margin',
      },
    });
  });

  it('lists what is already applied, so a second install is an addition', () => {
    expect(open().packs.installedHeadingId).toBe('server.landing.packs.installed_heading');
    expect(open().packs.installed[0].line).toEqual({
      id: 'server.landing.packs.installed_line',
      params: { name: 'Thin Margin', version: '1.2', id: 'thin-margin' },
    });
  });

  it('renders an open shelf the host has not pushed yet without crashing', () => {
    // The frame between the row opening and the first push. Every list is
    // empty, the folder is empty, and nothing pretends.
    const vm = landingViewModel({
      entries: NEEDY, provides: ['packs'], openEntryId: 'shelf', packs: null,
    });
    expect(vm.packs.rows).toEqual([]);
    expect(vm.packs.installed).toEqual([]);
    expect(vm.packs.conflicts).toEqual([]);
    expect(vm.packs.findings).toEqual([]);
    expect(vm.packs.emptyId).toBe('server.landing.packs.empty');
    expect(vm.packs.ctaEnabled).toBe(false);
  });
});

describe('every id the shipped table names is authored (issue #1368)', () => {
  // A row is only as good as the words behind it: a `labelId` with no CSV row
  // renders the id itself down the menu, which no other check in this suite
  // would notice — every `t` here is a stub that echoes its argument.
  const table = buildTable(
    readFileSync(new URL('../../assets/strings/strings.csv', import.meta.url), 'utf8'),
  );
  const resolves = (id) => !!table.get(id);

  it('authors a label and a description for every row', () => {
    for (const entry of LANDING_ENTRIES) {
      expect(resolves(entry.labelId), entry.labelId).toBe(true);
      expect(resolves(entry.descId), entry.descId).toBe(true);
    }
  });

  it('authors the status line every row that names one asks for', () => {
    for (const entry of LANDING_ENTRIES.filter((e) => e.statusId)) {
      expect(resolves(entry.statusId), entry.statusId).toBe(true);
    }
  });

  it('authors the standalone game master`s own start policy line', () => {
    // Not a landing string, but the sentence the same route puts under the one
    // Start control a fleetless game master is shown (server.html's
    // `paintGmStartControls`). It has no other test.
    expect(resolves('server.gm.start.standalone')).toBe(true);
  });
});
