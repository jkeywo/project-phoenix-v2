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
 *   | `stagePreBoot` | its stage opens only BEFORE this host has booted a world — absent means at any point in the session (issue #1364) |
 *   | `statusId`    | what the status bar says while this entry is open, for a route that is not "hosting" |
 *   | `join`        | this entry's stage is a JOIN CODE field: which typed namespace a bare suffix composes into, which surface words a refusal, which action a good code runs, and the four string ids the panel is written from (issue #1364) |
 *
 * Adding an entry is adding a row. Nothing below reads an id by name.
 *
 * `platforms` and `stagePlatforms` are deliberately not one field, because they say different
 * things and a slice that conflated them would lose a row rather than record a
 * gap. `platforms` is DOCTRINE: this surface does not offer this route and
 * never will — `connect_host` is absent from the native menu because a native
 * host is always a host and has no leg to connect with. `stagePlatforms` is
 * WORK NOT DONE: the route belongs here, this surface cannot serve it yet, so
 * the row renders and is inert exactly like a row whose `stage` is still
 * `null`. Load Game is the first of those — see its row, and #1363's AC5.
 *
 * `stagePreBoot` is the third of that family and says WHEN rather than where:
 * the landing comes back after a Game Over (`showLandingAtPicker`, issue #756),
 * and by then this page has a world loaded into a running Bevy that was
 * composed from a boot profile. A route whose whole job is to decide that
 * profile — or to leave the page entirely — cannot be offered on that second
 * landing, so both join rows carry it. Like the other two, it takes the STAGE
 * away and leaves the row: an operator who used Join as Peer before the mission
 * should find it where they left it, saying it is not available now, rather
 * than find the menu silently one row shorter.
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
 * `new_game` carries `stage: 'world-picker'`, `load_game` carries `stage:
 * 'save-catalogue'` (issue #1363) and both join routes carry `stage:
 * 'join-code'` (issue #1364); `load_mod_pack` alone still carries `stage: null`
 * and is inert — it renders, and clicking it changes nothing. An inert entry
 * that silently pretended to open something would be worse than a button that
 * plainly does not work yet, and it has a sibling issue that gives it a stage
 * of its own. [`nextOpenEntry`] is where "inert" is enforced, in one place,
 * rather than at each caller.
 *
 * ## Two rows, one stage, one panel (issue #1364)
 *
 * Join as Peer and Connect to Host both open `join-code` and both borrow
 * `#landing-join-panel`, because they ask the operator the same question —
 * eight letters — and differ only in what a good answer MEANS. That difference
 * is the row's `join` descriptor and nothing else: which typed namespace the
 * suffix composes into, which surface's wording a refusal takes, and which
 * action the caller runs. A third route that also wants a code adds a row with
 * a `join` on it; nothing here, in the renderer or in the stylesheet learns its
 * name.
 *
 * A row can also be inert on ONE surface: [`landingEntries`] takes the `stage`
 * away from a row whose `stagePlatforms` does not list the platform asking, so
 * "this host cannot open it yet" reaches every reader — the view model, the
 * renderer's `aria-disabled`, [`nextOpenEntry`] — as the one condition all
 * three already understand. It is also why both callers hand `nextOpenEntry`
 * the list [`landingEntries`] gave them rather than the raw table: the judge of
 * a click has to be looking at the same menu the operator is.
 */

// The join code's own module, imported rather than reimplemented: it already
// canonicalises what a player typed, tells a bare suffix from a pasted
// structured code, and maps every machine reason to the `strings.csv` id the
// phone's entry field renders. A second parser on the landing would be a second
// answer to "what is wrong with this code" (issue #1364's AC4). It is pure — no
// DOM, no transport — which is what makes importing it here safe for the two
// surfaces that load this file as a module.
import { parseJoinCode, reasonStringId } from './join-code.js';

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
    // Issue #1364. The join itself is not new — `__hostFleetJoin` has answered
    // a typed fleet code since #1114 — and neither is the Game Master profile.
    // What this row adds is WHERE the code is typed: the front door, instead of
    // a field behind the settings cog or a hand-edited `#fragment`.
    stage: 'join-code',
    docks: 'landing-join-panel',
    // Offered on BOTH, because "join someone else's session as a game master"
    // is a route a native build has every business showing.
    platforms: ['web', 'native'],
    // ...but only the browser host can OPEN it, and this is `stagePlatforms`
    // rather than `platforms` for the same reason Load Game's is: the route
    // belongs here and the work does not exist yet, which is a gap to record on
    // the row and not a claim that native hosts never game-master.
    //
    // What a slice closing it has to buy: the Game Master profile is a BROWSER
    // profile (`BootProfile::BrowserGameMaster`, selected in `wasm_init` from a
    // thread-local only `wasm_prepare_game_master` sets), the native lobby
    // document carries no `#landing-join-panel`, and `HostLobbyBridge` has no
    // channel to carry a typed code back to the host process. None of those is
    // a port of this row.
    stagePlatforms: ['web'],
    // ...and only before this host has booted a world. The Game Master profile
    // is read INSIDE `wasm_init` (`is_browser_gm`, src/server/bridge.rs), which
    // throws-to-unwind and runs exactly once per page: after it, the request
    // this route sets is a dead letter and setting it would only compose the
    // document as a GM page over an app Bevy already built as a `BrowserHost`.
    // That is the runtime profile swap issue #1364 says cannot happen, and the
    // landing DOES come back with a world loaded — Return to Lobby lands on the
    // World picker (issue #756). A reload is the honest way to change profile,
    // so the row says the stage is a pre-boot one and the operator is not
    // offered a lever that cannot move.
    stagePreBoot: true,
    statusId: 'server.landing.status_peer',
    join: {
      // A bare suffix typed here is a FLEET code, so it composes into the
      // server namespace — the same one `joinFleetFromFragment` parses with,
      // because they are two doors onto one join.
      namespace: 'server',
      // ...and a refusal is read on a viewscreen, where several of the phone's
      // sentences are actively wrong. `reasonStringId`'s server surface is what
      // says "that is a crew code" instead of its inverse (issue #1114).
      surface: 'server',
      // Which of the caller's join actions a good code runs. A NAME, not a
      // function, because this module is pure and the two actions live where
      // the page lifecycle does.
      action: 'boot-game-master',
      roleId: 'server.landing.join_peer_role',
      blurbId: 'server.landing.join_peer_blurb',
      submitId: 'server.landing.join_peer_submit',
    },
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
    stage: 'join-code',
    docks: 'landing-join-panel',
    platforms: ['web'],
    // Pre-boot for a different reason than its sibling: this route LEAVES the
    // page (`joinUrlForCode`), and on the landing that comes back after a Game
    // Over leaving means discarding a loaded world, a lobby of connected phones
    // and whatever fleet this host is in — none of which the operator asked to
    // end by typing a crew code. Round two is a re-selection, not a fresh page.
    stagePreBoot: true,
    statusId: 'server.landing.status_client',
    // The same panel, the same field, a different answer: a crew code names a
    // ship to take a seat on, so this route hands the code to the CLIENT page
    // rather than booting anything here. That is a navigation and the one place
    // in this slice where a page is left — see `joinUrlForCode`, which is
    // already how a QR sends a phone to exactly that URL.
    join: {
      namespace: 'client',
      surface: 'client',
      action: 'open-client-page',
      roleId: 'server.landing.connect_host_role',
      blurbId: 'server.landing.connect_host_blurb',
      submitId: 'server.landing.connect_host_submit',
    },
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
 * What every join route says about the FIELD itself (issue #1364).
 *
 * "Join code", "eight letters", "or paste the whole code" are facts about the
 * authored format and not about the route, so both rows share them and neither
 * repeats them. A row may still override any of the three by naming it in its
 * own `join`, which is what a third route with a different kind of code would
 * do — the defaults are merged UNDER the row, never over it.
 */
const JOIN_CODE_FIELD = {
  labelId: 'server.landing.join_code_label',
  placeholderId: 'server.landing.join_code_placeholder',
  hintId: 'server.landing.join_code_hint',
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
 * all off the one field the three of them already read. `docks`, `deeper` and
 * `join` go with it: a stage nothing can open has no panel, no ladder and no
 * code field.
 *
 * `opts.booted` is that same pass asked about TIME rather than platform (issue
 * #1364): this surface has a world loaded into a running engine, so every row
 * carrying `stagePreBoot` loses its stage by exactly the mechanism above. One
 * strip with two reasons and not two strips, because "renders, and clicking it
 * changes nothing" has to mean one thing downstream — a second vocabulary for
 * it would be a second answer for `nextOpenEntry` to disagree with.
 *
 * @param {'web'|'native'} platform which host is asking.
 * @param {Array<object>|null} [entries] a table to use instead of the shipped one.
 * @param {{booted?: boolean}} [opts] `booted` is "this host has already
 *   committed to a world" — the caller's own lifecycle, held there for the
 *   reason `openEntryId` is.
 */
export function landingEntries(platform, entries, opts) {
  const list = Array.isArray(entries) ? entries : LANDING_ENTRIES;
  const booted = !!(opts && opts.booted);
  return list.filter(function (entry) {
    if (!entry || !entry.id) return false;
    if (!Array.isArray(entry.platforms)) return true;
    return entry.platforms.indexOf(platform) !== -1;
  }).map(function (entry) {
    if (!entry.stage) return entry;
    const platformGap = Array.isArray(entry.stagePlatforms)
      && entry.stagePlatforms.indexOf(platform) === -1;
    const bootGap = booted && entry.stagePreBoot === true;
    if (!platformGap && !bootGap) return entry;
    return Object.assign({}, entry, {
      stage: null, docks: null, deeper: null, join: null,
    });
  });
}

/**
 * Judge a typed join code against the open row's `join` descriptor.
 *
 * The whole of "a refused code explains what is wrong with it" (issue #1364's
 * AC4), and pure, so that every refusal is a test rather than a claim about a
 * click handler. Three things can be wrong and each has an existing sentence:
 *
 *   - the code does not parse — empty, the wrong length, a character outside
 *     the alphabet, a word on the deny-list, a paste that lost a part. That is
 *     `parseJoinCode`'s verdict, verbatim;
 *   - the code parses but belongs to the OTHER typed namespace: a crew code
 *     typed into Join as Peer, or a fleet code typed into Connect to Host. The
 *     format exists precisely so this is answerable, and `wrong-type` is the
 *     answer — worded per surface, so a viewscreen is not told its own fleet
 *     code is a fleet code;
 *   - the authored format table has not loaded, so nothing can be judged at
 *     all. That is the join SERVICE being unreachable and says so, rather than
 *     sending an operator back to retype a code that was already right.
 *
 * `data` is the parsed `assets/join/join-codes.json` the caller already holds —
 * passed in rather than read from this module's globals, exactly as every
 * function in `gui/join-code.js` takes one, so a test never depends on an
 * installed table.
 *
 * @param {{namespace: string, surface: string, action: string}|null} join the
 *   open row's descriptor. Absent — a stage that is not a code field — is not
 *   an error the operator made, so it reports the service reason rather than
 *   inventing a fourth failure nobody can act on.
 * @param {string} raw what the operator typed or pasted.
 * @param {object|null} data the authored join-code format table.
 * @returns {{ok: true, code: string, action: string, namespace: string}
 *          |{ok: false, errorId: string}}
 */
export function landingJoinAttempt(join, raw, data) {
  const surface = (join && join.surface) || 'client';
  if (!join || !data) return { ok: false, errorId: reasonStringId('unreachable', surface) };
  let parsed;
  try {
    parsed = parseJoinCode(raw, join.namespace, data);
  } catch (_) {
    // A table this build does not speak throws rather than half-answering; the
    // operator's remedy is the same as an unreachable service's.
    return { ok: false, errorId: reasonStringId('unreachable', surface) };
  }
  if (!parsed.ok) return { ok: false, errorId: reasonStringId(parsed.reason, surface) };
  // A PASTED full code carries its own namespace and may not be this route's.
  // A bare suffix cannot get here wrong — it was composed into `join.namespace`
  // above — so this branch is exactly the paste case, which is the one the
  // typed format was invented to answer.
  if (parsed.namespace !== join.namespace) {
    return { ok: false, errorId: reasonStringId('wrong-type', surface) };
  }
  return {
    ok: true, code: parsed.full, action: join.action, namespace: parsed.namespace,
  };
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
 *   booted?: boolean,
 *   entries?: Array<object>,
 *   joinErrorId?: string|null,
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
 *   `booted` is the OTHER half of that lifecycle and the one `dismissed` cannot
 *   answer: the landing is back on screen (so it is not dismissed) but a world
 *   is already loaded into a running engine behind it — the Return to Lobby
 *   round two of issue #756. It reaches [`landingEntries`], where it takes the
 *   stage off every `stagePreBoot` row, and nothing else here reads it: which
 *   routes that silences is on the rows, not in this function.
 *
 *   `joinErrorId` is the string id of the last refusal the open join stage
 *   collected (issue #1364), and it is an input for the same reason
 *   `openEntryId` is: the attempt is made by whoever owns the field, so the
 *   memory of how it went is theirs too. [`landingJoinAttempt`] is what turns a
 *   code into one, so the caller never words a refusal itself.
 *
 * @returns {{
 *   stage: string,
 *   dismissed: boolean,
 *   openEntryId: string|null,
 *   deepStage: string|null,
 *   docks: string|null,
 *   join: object|null,
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
  const list = landingEntries(platform, opts.entries, { booted: !!opts.booted });

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
    // The open row's join descriptor, with the caller's last refusal folded
    // onto it (issue #1364). A COPY of the row's own object rather than a
    // hand-listed set of fields, so a route that grows a field on its `join`
    // reaches the renderer without this line learning its name — the same
    // reason `docks` is an id and not a boolean. `null` for every stage that is
    // not a code field, which is the whole condition the renderer reads.
    join: (open && open.join)
      ? Object.assign({}, JOIN_CODE_FIELD, open.join, { errorId: opts.joinErrorId || null })
      : null,
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
      // What the open route calls itself, from the ROW, falling back to
      // "hosting" for the routes that are hosting (issue #1364). Joining is not
      // hosting — a peer awaiting a code has opened no session of its own — and
      // saying so is one field on a row rather than a comparison of `open.id`
      // to a name here.
      sessionId: open
        ? (open.statusId || 'server.landing.status_hosting')
        : 'server.landing.status_no_session',
      build: { id: 'server.landing.build', params: { build: opts.build || 'dev' } },
    },
  };
}

// Expose for the classic-script consumer (server.html is not a module) — the
// same self-registering pattern window.hostScenarios uses.
if (typeof window !== 'undefined') {
  window.hostLanding = {
    LANDING_ENTRIES, landingEntries, nextOpenEntry, landingViewModel, landingJoinAttempt,
  };
}
