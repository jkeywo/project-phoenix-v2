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
 *   | field       | what it decides |
 *   |-------------|-----------------|
 *   | `id`        | the machine key a click reports and a hook dispatches on |
 *   | `labelId`   | the string id of the entry's name |
 *   | `descId`    | the string id of the line under it |
 *   | `stage`     | which contextual stage opening it reveals, or `null` for an entry that does nothing yet |
 *   | `platforms` | which hosts offer it at all — `['native']` is how #1365's Exit to Desktop arrives without a build check anywhere in this file |
 *   | `needs`     | a capability the SURFACE must say it provides for the row's stage to open — `'packs'` is how #1366's mod-pack shelf arrives without a check for how the host was started |
 *   | `confirm`   | the row asks once before its verb runs, and this is everything that stage says and does — `null`/absent for a route that simply opens |
 *   | `action`    | the machine verb a stage's own control sends, named once on the row that owns it |
 *
 * Adding an entry is adding a row. Nothing below reads an id by name.
 *
 * ## Three entries work, and the rest are deliberately inert
 *
 * `new_game` carries `stage: 'world-picker'`, `exit_desktop` (issue #1365)
 * carries `stage: 'exit-confirm'` and `load_mod_pack` (issue #1366) carries
 * `stage: 'mod-packs'`; the others carry `stage: null` and are inert — they
 * render, and clicking them changes nothing. An inert entry that silently
 * pretended to open something would be worse than a button that plainly does
 * not work yet, and each of them has a sibling issue that gives it a stage of
 * its own. [`nextOpenEntry`] is where "inert" is enforced, in one place, rather
 * than at each caller.
 *
 * ## `platforms` says WHICH BUILD; `needs` says WHAT THIS HOST HAS
 *
 * Two availability rules, and the second is not a weaker version of the first
 * (issue #1366). Connect to Host is absent on native and Exit to Desktop is
 * absent on the web because of what those builds ARE — no join leg, no
 * application to quit — and that never changes between two runs of one binary.
 * A mod-pack shelf does: the same `phoenix-host` offers one when it was started
 * with `--mod-pack-dir` and none when it was not. So the row declares what it
 * `needs` and the surface declares what it `provides`, and an unmet row is
 * inert exactly as it was before its slice landed. That is also why
 * `server.html` needed no edit to keep the landing it had.
 *
 * ## An entry that ASKS FIRST is still one row
 *
 * Exit to Desktop cannot be undone by pressing the entry again, so its press
 * opens a confirmation rather than doing the thing. That confirmation is a
 * `confirm` block on the row and the view model republishes it as
 * [`landingViewModel`]'s `confirm` — so the renderer draws "the open route's
 * confirmation", never "the exit confirmation", and the third such route costs
 * a row rather than a branch.
 */

/**
 * The menu, one row per entry, in the order they are offered.
 *
 * Exported so a caller can offer a different set (a test, or a surface that
 * curates the menu) without this module growing a parameter for every
 * variation. `landingViewModel` takes the list rather than reaching for this
 * constant, and this is only its default.
 *
 * `Exit to Desktop` is the last row, and is native-only: it arrived in #1365
 * as one more row carrying `platforms: ['native']`, which is the whole of what
 * "the web host does not offer it" cost.
 */
export const LANDING_ENTRIES = [
  {
    id: 'new_game',
    labelId: 'server.landing.new_game',
    descId: 'server.landing.new_game_desc',
    stage: 'world-picker',
    platforms: ['web', 'native'],
  },
  {
    id: 'load_game',
    labelId: 'server.landing.load_game',
    descId: 'server.landing.load_game_desc',
    stage: null,
    platforms: ['web', 'native'],
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
    // OFFERED EVERYWHERE, ANSWERED ONLY WHERE THERE IS A SHELF (issue #1366).
    //
    // The third shape of the same doctrine the two rows above and below state
    // in `platforms`, for a route whose availability is not a fact about the
    // BUILD but a fact about how this particular host was started. A native
    // host given `--mod-pack-dir` has a folder to offer; the same binary
    // started without it has none, and a browser host has none either. So the
    // row cannot say "native" or "web" — it says what it NEEDS, and the surface
    // says what it PROVIDES (`landingViewModel`'s `provides`). Unmet, the row
    // is inert exactly as it was before this slice; met, its stage opens.
    //
    // `platforms` could not have expressed this: it would have made one
    // invocation of the native binary a different platform from another.
    id: 'load_mod_pack',
    labelId: 'server.landing.load_mod_pack',
    descId: 'server.landing.load_mod_pack_desc',
    stage: 'mod-packs',
    needs: 'packs',
    platforms: ['web', 'native'],
    // The machine verb, named ONCE, on the row that owns it — the same
    // arrangement `exit_desktop`'s `confirm.action` makes below, and
    // deliberately the same token as the `kind` of the record the native
    // surface sends (`native_host::host_lobby::HostLobbyRecord::InstallModPack`).
    // `host_lobby_link.js` forwards the verb it is handed rather than keeping a
    // mapping table that would be the second place to edit.
    action: 'install_mod_pack',
  },
  {
    // NATIVE ONLY (issue #1365), and for the plainest reason in the table: a
    // browser tab cannot quit an application, so on the web there is nothing
    // behind this control at all. The same doctrine `connect_host` above
    // states, in the same field, pointing the other way.
    //
    // It is also the one row that carries a `confirm` block. Quitting is the
    // only route on this menu that an operator cannot take back by pressing
    // the entry again, so the press does not do it — it opens a stage that
    // says what is about to happen and asks once. The block is DATA for the
    // same reason the rows are: the next entry that needs a confirmation adds
    // one of these, and neither this module nor its renderer learns an id by
    // name to draw it.
    id: 'exit_desktop',
    labelId: 'server.landing.exit_desktop',
    descId: 'server.landing.exit_desktop_desc',
    stage: 'exit-confirm',
    platforms: ['native'],
    confirm: {
      titleId: 'server.landing.exit_confirm_title',
      eyebrowId: 'server.landing.exit_confirm_eyebrow',
      leadId: 'server.landing.exit_confirm_lead',
      noteId: 'server.landing.exit_confirm_note',
      ctaId: 'server.landing.exit_confirm_cta',
      // Why the tone is a field and not a class the renderer picks: which
      // confirmations are destructive is knowledge the ROW has, and a renderer
      // deciding it would be deciding it a second time.
      tone: 'danger',
      // The machine verb the caller dispatches on, and deliberately the same
      // token as the `kind` of the record the native surface sends
      // (`native_host::host_lobby::HostLobbyRecord::ExitDesktop`). A confirming
      // row names its verb once; `host_lobby_link.js` forwards it rather than
      // keeping a mapping table that would be the second place to edit.
      action: 'exit_desktop',
    },
  },
];

/**
 * The label a confirmation's cancel control wears when its row does not name
 * one.
 *
 * Shared rather than repeated per row because "the way back" is the same act on
 * every confirmation there will ever be. A row that genuinely needs other words
 * still overrides it with a `cancelId` of its own.
 */
export const CONFIRM_CANCEL_ID = 'server.landing.confirm_back';

/**
 * Whether `entry` needs something this surface has not said it provides.
 *
 * The one place `needs`/`provides` is read, so "a row whose requirement is
 * unmet is inert" is decided once rather than at `nextOpenEntry`, at the entry
 * mapping and at the open-stage lookup separately — three places that would
 * eventually disagree about the same row.
 */
function unmet(entry, provides) {
  return !!entry.needs && provides.indexOf(entry.needs) === -1;
}

/** The `provides` list, normalised. Anything else reads as "provides nothing". */
function providedBy(input) {
  return Array.isArray(input.provides) ? input.provides : [];
}

/**
 * Which note tone a finding severity wears, and what it is called.
 *
 * A table rather than a branch for the reason the entries are one: the
 * validator's severities are its own vocabulary, and a third one arriving
 * should cost a row here rather than an `else if` in a renderer.
 */
const FINDING_SEVERITY = {
  error: { tone: 'bad', labelId: 'server.landing.packs.severity_error' },
  warning: { tone: 'warn', labelId: 'server.landing.packs.severity_warning' },
};

/**
 * The mod-pack shelf stage (issue #1366).
 *
 * Everything it draws comes off the HOST's snapshot — which folder is being
 * scanned, what is in it, what is installed, what the last attempt said, and
 * which pack wins each path two of them share — plus the one thing the host
 * does not know, which is the row the operator has highlighted.
 *
 * Two shapes of text, kept apart on purpose:
 *
 *   - **string ids**, for every word this surface owns: the heading, the empty
 *     state, the severity labels, the CTA. Data-dependent ones travel as
 *     `{id, params}` pairs, the way `gui/lobby-view.js` carries them.
 *   - **prose**, for a validator's sentence about the operator's own archive
 *     and for a scan failure naming their own folder. Neither could be a table
 *     entry: they are generated from the file in front of them, and the browser
 *     host shows exactly the same sentences in `#mod-pack-findings`.
 */
function packsStage(row, packs, chosenPack) {
  const shelf = packs || {};
  const offered = Array.isArray(shelf.offered) ? shelf.offered : [];
  const installed = Array.isArray(shelf.installed) ? shelf.installed : [];
  const conflicts = Array.isArray(shelf.conflicts) ? shelf.conflicts : [];
  const findings = Array.isArray(shelf.findings) ? shelf.findings : [];
  // A highlighted row the host is no longer offering is no highlight at all —
  // the shelf is rescanned on every attempt, so a pack can leave the folder
  // between the click that chose it and the render that draws it.
  const chosen = offered.some(function (p) { return p.file === chosenPack; })
    ? chosenPack
    : null;
  const attempted = shelf.attempted || null;
  return {
    titleId: 'server.landing.packs.title',
    // Which folder, always — a shelf with nothing on it is otherwise
    // indistinguishable from a host that was never given one, and those two
    // ask the operator to do different things.
    folder: { id: 'server.landing.packs.folder', params: { dir: shelf.dir || '' } },
    rows: offered.map(function (pack) {
      return {
        file: pack.file,
        label: pack.label || pack.file,
        selected: pack.file === chosen,
      };
    }),
    // The empty state names WHICH emptiness this is. `scanError` is the host's
    // own sentence about the operator's own path and rides beside the id
    // rather than inside it.
    emptyId: offered.length
      ? null
      : (shelf.scan_error
        ? 'server.landing.packs.scan_failed'
        : 'server.landing.packs.empty'),
    scanError: shelf.scan_error || null,
    // What the last attempt did, as one line. `null` before the first one, so
    // a freshly opened shelf reports nothing rather than reporting success.
    outcome: attempted
      ? {
        tone: shelf.accepted ? 'ok' : 'bad',
        line: {
          id: shelf.accepted
            ? 'server.landing.packs.accepted'
            : 'server.landing.packs.refused',
          params: { pack: attempted },
        },
      }
      : null,
    findingsHeadingId: findings.length ? 'server.landing.packs.findings_heading' : null,
    findings: findings.map(function (finding) {
      const severity = FINDING_SEVERITY[finding.severity]
        || { tone: 'warn', labelId: 'server.landing.packs.severity_warning' };
      return {
        tone: severity.tone,
        labelId: severity.labelId,
        category: finding.category || '',
        // Prose. See the note above.
        message: finding.message || '',
        file: finding.file || '',
      };
    }),
    installedHeadingId: installed.length ? 'server.landing.packs.installed_heading' : null,
    installed: installed.map(function (pack) {
      return {
        id: pack.id,
        line: {
          id: 'server.landing.packs.installed_line',
          params: { name: pack.name || pack.id, version: pack.version || '', id: pack.id },
        },
      };
    }),
    // Which pack won a path two of them carry. Shown rather than only logged:
    // two packs that both replace one hull produce one hull, and an operator
    // who cannot see which is flying has no way to work out why their change
    // did nothing.
    conflictsHeadingId: conflicts.length ? 'server.landing.packs.conflict_heading' : null,
    conflicts: conflicts.map(function (conflict) {
      return {
        path: conflict.path,
        line: {
          id: 'server.landing.packs.conflict_line',
          params: {
            path: conflict.path,
            winner: conflict.winner,
            losers: (conflict.losers || []).join(', '),
          },
        },
      };
    }),
    ctaId: 'server.landing.packs.install',
    // Nothing highlighted is nothing to install. Said in the model rather than
    // in the renderer so "the button is dead until a row is chosen" is a test.
    ctaEnabled: !!chosen,
    chosen: chosen,
    cancelId: CONFIRM_CANCEL_ID,
    // The row's own verb, carried verbatim — the same arrangement
    // `confirm.action` makes. This module never runs it and never decides what
    // it means; the caller with something behind it does.
    action: row.action || 'install_mod_pack',
  };
}

/** The platform label each host wears, by the `platform` this module is given. */
const PLATFORM_LABEL = {
  web: 'server.landing.platform_web',
  native: 'server.landing.platform_native',
};

/**
 * The entries a given platform offers.
 *
 * A row with no `platforms` is offered everywhere — the permissive default is
 * on purpose, so a slice adding an entry only has to think about the field
 * when its entry is genuinely platform-bound.
 */
export function landingEntries(platform, entries) {
  const list = Array.isArray(entries) ? entries : LANDING_ENTRIES;
  return list.filter(function (entry) {
    if (!entry || !entry.id) return false;
    if (!Array.isArray(entry.platforms)) return true;
    return entry.platforms.indexOf(platform) !== -1;
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
 *     the table does not hold;
 *   - an entry whose `needs` this surface does not provide changes nothing
 *     either (issue #1366). A host with no scanned folder has nothing to draw
 *     in the mod-pack stage, and opening an empty panel would be worse than the
 *     control that plainly does not work yet.
 *
 * @param {Array<object>} [entries] the table to read; defaults to
 *   [`LANDING_ENTRIES`].
 * @param {Array<string>} [provides] what THIS surface can answer. Omitted reads
 *   as "nothing", which is the honest default: a caller that has not said it
 *   can serve a shelf cannot.
 * @returns {string|null} the id to pass back as `openEntryId`.
 */
export function nextOpenEntry(openEntryId, entryId, entries, provides) {
  const list = Array.isArray(entries) ? entries : LANDING_ENTRIES;
  const can = Array.isArray(provides) ? provides : [];
  const entry = list.find(function (e) { return e && e.id === entryId; });
  // Unknown id, an entry whose slice has not landed yet, or one this surface
  // cannot answer: nothing moves.
  if (!entry || !entry.stage || unmet(entry, can)) {
    return openEntryId == null ? null : openEntryId;
  }
  if (openEntryId === entryId) return null;
  return entryId;
}

/**
 * The landing's view model.
 *
 * @param {{
 *   openEntryId?: string|null,
 *   platform?: 'web'|'native',
 *   build?: string,
 *   dismissed?: boolean,
 *   entries?: Array<object>,
 *   provides?: Array<string>,
 *   packs?: object|null,
 *   chosenPack?: string|null,
 * }} [input]
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
 *   `provides` is what THIS surface can answer, against the `needs` a row
 *   declares (issue #1366). It is the third availability rule on this menu and
 *   the only one that is not a fact about the build: the same native binary
 *   offers a mod-pack shelf when it was started with `--mod-pack-dir` and none
 *   when it was not, so `platforms` could not have said it. Omitted reads as
 *   "provides nothing", which keeps every needing row exactly as inert as it
 *   was before its slice landed — and is why `server.html` needs no edit to
 *   keep the behaviour it has.
 *
 *   `packs` is the host's own shelf snapshot
 *   (`native_host::host_lobby::packs::ModPackPanelPayload`), already-resolved
 *   data rather than string ids: a folder path, a list of file names, and the
 *   validator's own sentences about the operator's own archives. `chosenPack`
 *   is which row the operator has highlighted — the caller's memory, for
 *   exactly the reason `openEntryId` is (see above), and never the host's: the
 *   host learns about a choice when it is asked to install one.
 *
 * @returns {{
 *   stage: string,
 *   dismissed: boolean,
 *   openEntryId: string|null,
 *   depth: number,
 *   rootClass: string,
 *   identity: {titleId: string, taglineId: string, logoAltId: string, platformLabelId: string},
 *   entries: Array<{id: string, ordinal: string, labelId: string, descId: string, stage: string|null, selected: boolean, inert: boolean}>,
 *   confirm: null|{titleId: string, eyebrowId: string|null, leadId: string|null, noteId: string|null, ctaId: string, cancelId: string, tone: string, action: string},
 *   packs: null|object,
 *   status: {platformLabelId: string, sessionId: string, build: {id: string, params: {build: string}}},
 * }}
 *   `confirm` is the OPEN ROW's own confirmation block, republished — never a
 *   second decision made here, and never keyed off an entry id. It is `null`
 *   for every route that simply opens something (New Game's picker), which is
 *   what lets the renderer draw a confirmation from its presence alone.
 *
 *   `packs` is the mod-pack shelf, drawn only while the row that needs it is
 *   the open one — the exact sibling of `confirm`, and `null` everywhere else
 *   for the same reason: the renderer draws the stage from its presence and
 *   holds no opinion about which route it belongs to.
 */
export function landingViewModel(input) {
  const opts = input || {};
  const platform = opts.platform === 'native' ? 'native' : 'web';
  const dismissed = !!opts.dismissed;
  const list = landingEntries(platform, opts.entries);
  const provides = providedBy(opts);

  // An `openEntryId` naming an entry this platform does not offer (or an
  // entry that never had a stage) reads as closed rather than as a stage
  // nothing can render. The caller's memory can outlive a menu change — a
  // native host and a browser host share this module and not their rows.
  // A dismissed landing has no open stage by construction: the operator is
  // past it, and a remembered entry re-opening the moment it came back would
  // be the surface disagreeing with the host about where the session is.
  //
  // A row whose `needs` this surface does not provide reads as closed too
  // (issue #1366): the caller's memory can outlive a RUN as well as a menu — a
  // page reloaded against a host restarted without `--mod-pack-dir` would
  // otherwise open a shelf stage with no shelf behind it.
  const open = dismissed ? null : (list.find(function (e) {
    return e.id === opts.openEntryId && !!e.stage && !unmet(e, provides);
  }) || null);

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
      // Two ways to be inert, one word for both, because they look the same to
      // whoever is standing in front of the screen: the route has no stage
      // yet, or this surface cannot answer the stage it has.
      inert: !entry.stage || unmet(entry, provides),
    };
  });

  return {
    // `dismissed` outranks every other stage, and is a stage rather than a
    // flag beside one so that a renderer switching on `vm.stage` cannot be
    // shown the landing and told it is gone in the same breath.
    stage: dismissed ? 'dismissed' : open ? open.stage : 'idle',
    dismissed: dismissed,
    openEntryId: open ? open.id : null,
    // The track's offset, as a number the stylesheet reads through a custom
    // property. The DOCUMENT says only how deep it is; which columns that
    // slides, and whether it slides at all, is the stylesheet's business at
    // each breakpoint (issue #1360's responsiveness rule).
    depth: open ? 1 : 0,
    rootClass: open ? 'is-open' : 'is-idle',
    identity: {
      titleId: 'server.landing.title',
      taglineId: 'server.landing.tagline',
      logoAltId: 'server.landing.logo_alt',
      platformLabelId: PLATFORM_LABEL[platform],
    },
    entries: entries,
    // The open row's confirmation, or nothing. Composed rather than passed
    // straight through so that a row states only what is peculiar to it: the
    // way back reads the shared default, and a row that never named a tone is
    // an ordinary confirmation rather than an undefined one the renderer would
    // have to interpret.
    confirm: open && open.confirm
      ? {
        titleId: open.confirm.titleId,
        eyebrowId: open.confirm.eyebrowId || null,
        leadId: open.confirm.leadId || null,
        noteId: open.confirm.noteId || null,
        ctaId: open.confirm.ctaId,
        cancelId: open.confirm.cancelId || CONFIRM_CANCEL_ID,
        tone: open.confirm.tone || 'normal',
        // The verb, carried verbatim. This module never runs it and never
        // decides what it means — the caller with something behind it does.
        action: open.confirm.action || open.id,
      }
      : null,
    // The mod-pack shelf, drawn only while the row that needs it is open
    // (issue #1366) — the exact sibling of `confirm` above, decided from the
    // open ROW rather than from an id, so a second shelf-shaped stage would be
    // a second row and not a branch here.
    packs: open && open.stage === 'mod-packs'
      ? packsStage(open, opts.packs, opts.chosenPack)
      : null,
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
  window.hostLanding = {
    LANDING_ENTRIES,
    CONFIRM_CANCEL_ID,
    landingEntries,
    nextOpenEntry,
    landingViewModel,
  };
}
