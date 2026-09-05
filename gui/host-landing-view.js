/**
 * gui/host-landing-view.js — the pure view-model behind the landing screen
 * (issue #1360, PRD #1355).
 *
 * The landing is the first thing a host shows: an identity lockup, a menu of
 * routes, and one contextual column beside it. This module decides what that
 * menu offers and which route is open. It writes nothing — no DOM, no
 * transport, no arbiter — which is what lets the decision be tested without a
 * document, exactly as `gui/host-scenarios.js` does for the scenario picker
 * and `gui/host-lobby-view.js` does for the crew lobby. Its DOM half is
 * `gui/host-landing-render.js`.
 *
 * ## The entries are a TABLE, not a switch
 *
 * This is the load-bearing shape, and the reason it is stated this loudly:
 * six sibling slices (#1362-#1367) each add or activate one menu entry, and
 * they land in these same three files. If "add an entry" meant "edit a
 * switch", all six would collide in the same handful of lines. So an entry is
 * a ROW in [`LANDING_ENTRIES`], carrying everything about itself:
 *
 *   | field         | what it decides |
 *   |---------------|-----------------|
 *   | `id`          | the machine key a click reports and a hook dispatches on |
 *   | `labelId`     | the string id of the entry's name |
 *   | `descId`      | the string id of the line under it |
 *   | `stage`       | which contextual stage opening it reveals, or `null` for an entry that does nothing yet |
 *   | `deeper`      | the stages this entry can descend INTO once open, in order — absent for an entry that is only one column deep |
 *   | `docks`       | this entry's stage is an EXISTING panel node — named by element id — moved into the middle column, rather than markup of its own |
 *   | `platforms`   | which hosts offer it at all — `['native']` is how #1365's Exit to Desktop arrives without a build check anywhere in this file |
 *   | `stagePlatforms` | which of those hosts can actually OPEN its stage — absent means all of them |
 *
 * Adding an entry is adding a row. Nothing below reads an id by name.
 *
 * The last two are deliberately not one field, because they say different
 * things and a slice that conflated them would lose a row rather than record a
 * gap. `platforms` is DOCTRINE: this surface does not offer this route and
 * never will — `connect_host` is absent from the native menu because a native
 * host is always a host and has no leg to connect with. `stagePlatforms` is
 * WORK NOT DONE: the route belongs here, this surface cannot serve it yet, so
 * the row renders and is inert exactly like a row whose `stage` is still
 * `null`. Load Game is the first of those — see its row, and #1363's AC5.
 *
 * ## A ladder is a field, not a second entry (issue #1362)
 *
 * New Game asks two questions: choose a World, then choose a hull. The second
 * is not another menu ROW — the menu still has five — it is one rung deeper
 * inside the entry that is already open, and the operator has to be able to
 * see the World list they came through and step back along it. So `deeper` is
 * an ordered list of stage names ON THE ROW, and the caller says which of them
 * it is currently on (`deepStage`). The DEPTH the stylesheet slides by falls
 * out of the position in that list, so an entry that grows a third rung adds a
 * string to its own array and neither this module, the renderer, nor the
 * stylesheet learns a new name.
 *
 * ## An entry with no stage is inert, and that is deliberate
 *
 * `new_game` carries `stage: 'world-picker'` and, since issue #1363,
 * `load_game` carries `stage: 'save-catalogue'`; the other three carry `stage:
 * null` and are inert — they render, and clicking them changes nothing. An
 * inert entry that silently pretended to open something would be worse than a
 * button that plainly does not work yet, and each of the three has a sibling
 * issue that gives it a stage of its own. [`nextOpenEntry`] is where "inert"
 * is enforced, in one place, rather than at each caller.
 *
 * A row can also be inert on ONE surface: [`landingEntries`] takes the `stage`
 * away from a row whose `stagePlatforms` does not list the platform asking, so
 * "this host cannot open it yet" reaches every reader — the view model, the
 * renderer's `aria-disabled`, [`nextOpenEntry`] — as the one condition all
 * three already understand. It is also why both callers hand `nextOpenEntry`
 * the list [`landingEntries`] gave them rather than the raw table: the judge of
 * a click has to be looking at the same menu the operator is.
 */

/**
 * The menu, one row per entry, in the order they are offered.
 *
 * Exported so a caller can offer a different set (a test, or a surface that
 * curates the menu) without this module growing a parameter for every
 * variation. `landingViewModel` takes the list rather than reaching for this
 * constant, and this is only its default.
 *
 * `Exit to Desktop` is deliberately absent: it is a native-build entry and
 * arrives in #1365 as one more row carrying `platforms: ['native']`.
 */
export const LANDING_ENTRIES = [
  {
    id: 'new_game',
    labelId: 'server.landing.new_game',
    descId: 'server.landing.new_game_desc',
    stage: 'world-picker',
    // Choosing a World asks a second question, and its answer is a column
    // further along the same track rather than a different route (issue
    // #1362). The name matches the stage `gui/host-scenarios.js` reports, so
    // the host hands this module the picker's own answer instead of
    // translating between two vocabularies for the same thing.
    deeper: ['ship-picker'],
    // Both rungs are drawn by the picker that already exists: the middle
    // column holds the live `#scenario-panel`, and the hull column holds the
    // `ph-ship-picker` its renderer mounts. Nothing here re-implements either.
    docks: 'scenario-panel',
    platforms: ['web', 'native'],
  },
  {
    id: 'load_game',
    labelId: 'server.landing.load_game',
    descId: 'server.landing.load_game_desc',
    // Issue #1363. The catalogue itself — listing, the compatibility refusal a
    // row carries, manual against automatic, and the version gate that runs
    // before anything is restored — is `gui/save-slots.js` and predates this
    // row by five hundred commits. What this row adds is WHERE it is shown:
    // the middle column, on the same track the World picker opens on, instead
    // of a permanent second column of the boot panel nobody asked for.
    stage: 'save-catalogue',
    // The live `#save-slots-panel` node, borrowed exactly as New Game borrows
    // `#scenario-panel` above. Borrowed and not re-rendered, for the same
    // reason: a Start still reaches the resume path down the wire it always
    // did, and the save importer travels with the panel because it is now one
    // of its children (`mountSaveSlots`'s `headerAction`).
    docks: 'save-slots-panel',
    // Offered on BOTH hosts, because Load Game is a route a native host has
    // every business showing: #1363's AC5 asks for exactly that. `platforms`
    // is not the field for what is missing here — see `stagePlatforms` below.
    platforms: ['web', 'native'],
    // ...but only the web host can OPEN it yet. #1363's AC5 — "the native
    // surface can resume a save without a startup flag" — IS NOT MET, and this
    // line is the record of that, on the row, rather than in a comment
    // somebody has to go looking for. The row still renders on the viewscreen,
    // dashed and `aria-disabled` like every other not-yet row, because a menu
    // that quietly dropped it would have turned an unfinished AC into a claim
    // that native hosts do not load games.
    //
    // What the slice closing AC5 has to buy, in the order it will meet it:
    //
    //   * the native lobby document (`native_host::host_lobby::document`)
    //     carries no `#save-slots-panel`. It used to arrive by accident, as the
    //     last child of the extracted `#scenario-panel`; #1363 made the
    //     catalogue a body-level sibling, so it now needs an extraction of its
    //     own beside the four already there.
    //   * `HostLobbyBridge` has no save-catalogue channel, so there would be
    //     nothing to fill that panel FROM: the rows, the refusal each carries
    //     and what a Start reports back all need a record vocabulary, the way
    //     the picker's and the landing's do.
    //   * native resume is startup-only BY DESIGN, not by omission.
    //     `save_slots_store::stage_new_native_session_from_slot` refuses at any
    //     `SimTick` past 0 — "this startup route cannot become a live-session
    //     restore by being called later" — and `advance_sim_tick` runs every
    //     fixed step from a native host's first frame, lobby included. So
    //     answering a Start there means either relaxing that guard or building
    //     the App a second time around the restored run. That is a decision
    //     about the native lifecycle, not a port of this row, which is why it
    //     is not taken here.
    stagePlatforms: ['web'],
  },
  {
    id: 'join_peer',
    labelId: 'server.landing.join_peer',
    descId: 'server.landing.join_peer_desc',
    stage: null,
    platforms: ['web', 'native'],
  },
  {
    // WEB ONLY, and this is the doctrine rather than a gap (issues #1361,
    // #1364): a native host is always a host and has no join leg at all, so
    // there is nothing behind this control on that surface. A control exists
    // exactly when something behind it can answer it, and the row — not a
    // build check in the renderer — is where that is said.
    id: 'connect_host',
    labelId: 'server.landing.connect_host',
    descId: 'server.landing.connect_host_desc',
    stage: null,
    platforms: ['web'],
  },
  {
    id: 'load_mod_pack',
    labelId: 'server.landing.load_mod_pack',
    descId: 'server.landing.load_mod_pack_desc',
    stage: null,
    platforms: ['web', 'native'],
  },
];

/** The platform label each host wears, by the `platform` this module is given. */
const PLATFORM_LABEL = {
  web: 'server.landing.platform_web',
  native: 'server.landing.platform_native',
};

/**
 * The entries a given platform offers, each carrying only what that platform
 * can actually do with it.
 *
 * A row with no `platforms` is offered everywhere, and a row with no
 * `stagePlatforms` opens wherever it is offered — both permissive defaults are
 * on purpose, so a slice adding an entry only has to think about either field
 * when its entry is genuinely platform-bound.
 *
 * The second pass is why this returns a copy of a row rather than the table's
 * own object for a surface that is missing one: a row this platform offers but
 * cannot yet SERVE comes back with its `stage` taken away, which is already
 * the whole vocabulary of "renders, and clicking it changes nothing". Nothing
 * downstream learns a platform name — the view model marks it `inert`, the
 * renderer writes `aria-disabled`, and [`nextOpenEntry`] refuses to open it,
 * all off the one field the three of them already read. `docks` and `deeper`
 * go with it: a stage nothing can open has neither a panel nor a ladder.
 */
export function landingEntries(platform, entries) {
  const list = Array.isArray(entries) ? entries : LANDING_ENTRIES;
  return list.filter(function (entry) {
    if (!entry || !entry.id) return false;
    if (!Array.isArray(entry.platforms)) return true;
    return entry.platforms.indexOf(platform) !== -1;
  }).map(function (entry) {
    if (!entry.stage || !Array.isArray(entry.stagePlatforms)) return entry;
    if (entry.stagePlatforms.indexOf(platform) !== -1) return entry;
    return Object.assign({}, entry, { stage: null, docks: null, deeper: null });
  });
}

/**
 * Which entry is open after a click on `entryId`, given `openEntryId` now.
 *
 * The whole of the menu's behaviour, and pure so that "clicking New Game again
 * closes it" is a test rather than a claim about a click handler:
 *
 *   - the entry already open closes (that is the second click on New Game);
 *   - an entry with a `stage` opens, replacing whatever was open;
 *   - an entry with no `stage` changes nothing at all, and neither does an id
 *     the list does not hold.
 *
 * `entries` should be the list [`landingEntries`] gave this surface, not the
 * shipped table: that is where a row's `stage` is taken away on a host which
 * cannot serve it yet (`stagePlatforms`). Judging a click against the whole
 * table would let a surface remember an entry as open that its own view model
 * renders as closed — two memories disagreeing about one press. The default is
 * the shipped table, which is the right answer for a `web` caller and is what
 * the pure tests lean on.
 *
 * @returns {string|null} the id to pass back as `openEntryId`.
 */
export function nextOpenEntry(openEntryId, entryId, entries) {
  const list = Array.isArray(entries) ? entries : LANDING_ENTRIES;
  const entry = list.find(function (e) { return e && e.id === entryId; });
  // Unknown id, or an entry whose slice has not landed yet: nothing moves.
  if (!entry || !entry.stage) return openEntryId == null ? null : openEntryId;
  if (openEntryId === entryId) return null;
  return entryId;
}

/**
 * The landing's view model.
 *
 * @param {{
 *   openEntryId?: string|null,
 *   deepStage?: string|null,
 *   platform?: 'web'|'native',
 *   build?: string,
 *   dismissed?: boolean,
 *   entries?: Array<object>,
 * }} [input]
 *   `deepStage` is how far along the open entry's `deeper` ladder the surface
 *   is (issue #1362). It is an input for the same reason `openEntryId` is: the
 *   answer is not this module's to hold. For New Game it is the picker's OWN
 *   stage — `scenarioCatalogView().stage` — passed through unchanged, so the
 *   two models agree by sharing a value rather than by a mapping somebody has
 *   to keep in step. A name the open entry does not list (or one on an entry
 *   that has no ladder) reads as "not deep", never as a stage nothing can draw.
 *
 *   `openEntryId` is the caller's own memory of which entry is open — held by
 *   the caller rather than here for the reason `scenarioCatalogView` takes
 *   `locked` rather than deriving it: the surfaces that drive this each own
 *   their page lifecycle, and a module-level flag two documents shared would
 *   be a second authority the moment either forgot to update it. `build` is
 *   already-resolved data (a build id), not a string id, and travels as
 *   `params` on a `{id, params}` pair the way `gui/lobby-view.js` carries
 *   data-dependent text.
 *
 *   `dismissed` is "this surface is past the landing" — a world has been
 *   committed and the front door has nothing left to offer. It collapses the
 *   whole model to the `dismissed` stage, which is the exact sibling of
 *   `scenarioCatalogView`'s `locked`: the one stage that means "not on
 *   screen", so a surface that owns its own panel visibility
 *   (`renderHostLanding`'s `ownPanelVisibility` — the native viewscreen,
 *   issue #1361) reads it from the view model rather than being told twice.
 *   The host PAGE passes nothing and keeps `hideLanding()`, which is its own
 *   lifecycle and not this model's.
 *
 * @returns {{
 *   stage: string,
 *   dismissed: boolean,
 *   openEntryId: string|null,
 *   deepStage: string|null,
 *   docks: string|null,
 *   depth: number,
 *   rootClass: string,
 *   identity: {titleId: string, taglineId: string, logoAltId: string, platformLabelId: string},
 *   entries: Array<{id: string, ordinal: string, labelId: string, descId: string, stage: string|null, selected: boolean, inert: boolean}>,
 *   status: {platformLabelId: string, sessionId: string, build: {id: string, params: {build: string}}},
 * }}
 */
export function landingViewModel(input) {
  const opts = input || {};
  const platform = opts.platform === 'native' ? 'native' : 'web';
  const dismissed = !!opts.dismissed;
  const list = landingEntries(platform, opts.entries);

  // An `openEntryId` naming an entry this platform does not offer (or an
  // entry that never had a stage) reads as closed rather than as a stage
  // nothing can render. The caller's memory can outlive a menu change — a
  // native host and a browser host share this module and not their rows.
  // A dismissed landing has no open stage by construction: the operator is
  // past it, and a remembered entry re-opening the moment it came back would
  // be the surface disagreeing with the host about where the session is.
  const open = dismissed ? null : (list.find(function (e) {
    return e.id === opts.openEntryId && !!e.stage;
  }) || null);

  // How far along the OPEN entry's own ladder the surface says it is. The
  // position in `deeper` is the answer to both questions the renderer has —
  // which stage is showing, and how many columns the track has slid — so a
  // name the row does not list is simply -1 and the entry sits at its first
  // rung. Nothing here knows what 'ship-picker' means.
  const rungs = (open && Array.isArray(open.deeper)) ? open.deeper : [];
  const rung = opts.deepStage == null ? -1 : rungs.indexOf(opts.deepStage);
  const deepStage = rung >= 0 ? rungs[rung] : null;

  const entries = list.map(function (entry, i) {
    return {
      id: entry.id,
      // Two digits, as the design draws them; a machine-readable position, so
      // the renderer never has to count.
      ordinal: String(i + 1).padStart(2, '0'),
      labelId: entry.labelId,
      descId: entry.descId,
      stage: entry.stage || null,
      selected: !!open && open.id === entry.id,
      inert: !entry.stage,
    };
  });

  return {
    // `dismissed` outranks every other stage, and is a stage rather than a
    // flag beside one so that a renderer switching on `vm.stage` cannot be
    // shown the landing and told it is gone in the same breath.
    stage: dismissed ? 'dismissed' : open ? (deepStage || open.stage) : 'idle',
    dismissed: dismissed,
    openEntryId: open ? open.id : null,
    // Which rung of the open entry's ladder, as a name, for a caller that
    // needs to tell the two apart without comparing `stage` to a literal.
    deepStage: deepStage,
    // WHICH existing panel node this stage is served by, as an element id, or
    // null for a stage drawn from markup of its own — a fact about the ROW, so
    // the renderer never asks "is the stage called world-picker" and an entry
    // that borrows a panel says which one on its own line (issues #1362,
    // #1363). An id rather than a boolean because there are now two: New Game
    // borrows `#scenario-panel` and Load Game borrows `#save-slots-panel`, and
    // a second boolean beside the first would be the switch this table exists
    // to avoid.
    docks: (open && open.docks) || null,
    // The track's offset, as a number the stylesheet reads through a custom
    // property. The DOCUMENT says only how deep it is; which columns that
    // slides, and whether it slides at all, is the stylesheet's business at
    // each breakpoint (issue #1360's responsiveness rule). One per rung: the
    // open entry's own column is 1, and each `deeper` stage adds another.
    depth: open ? 1 + (rung + 1) : 0,
    // Space-separated, and the renderer splits it: `is-deep` is a SECOND fact
    // about the same root (open, and more than one column along), not a
    // replacement for the first. Written from the depth rather than from a
    // stage name, so a third rung needs no new class.
    rootClass: open ? (rung >= 0 ? 'is-open is-deep' : 'is-open') : 'is-idle',
    identity: {
      titleId: 'server.landing.title',
      taglineId: 'server.landing.tagline',
      logoAltId: 'server.landing.logo_alt',
      platformLabelId: PLATFORM_LABEL[platform],
    },
    entries: entries,
    status: {
      platformLabelId: PLATFORM_LABEL[platform],
      sessionId: open ? 'server.landing.status_hosting' : 'server.landing.status_no_session',
      build: { id: 'server.landing.build', params: { build: opts.build || 'dev' } },
    },
  };
}

// Expose for the classic-script consumer (server.html is not a module) — the
// same self-registering pattern window.hostScenarios uses.
if (typeof window !== 'undefined') {
  window.hostLanding = { LANDING_ENTRIES, landingEntries, nextOpenEntry, landingViewModel };
}
