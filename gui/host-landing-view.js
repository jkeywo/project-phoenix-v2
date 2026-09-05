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
 *   | `confirm`   | the row asks once before its verb runs, and this is everything that stage says and does — `null`/absent for a route that simply opens |
 *
 * Adding an entry is adding a row. Nothing below reads an id by name.
 *
 * ## Two entries work, and the rest are deliberately inert
 *
 * `new_game` carries `stage: 'world-picker'` and `exit_desktop` (issue #1365)
 * carries `stage: 'exit-confirm'`; the others carry `stage: null` and are inert
 * — they render, and clicking them changes nothing. An inert entry that
 * silently pretended to open something would be worse than a button that
 * plainly does not work yet, and each of them has a sibling issue that gives it
 * a stage of its own. [`nextOpenEntry`] is where "inert" is enforced, in one
 * place, rather than at each caller.
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
    id: 'load_mod_pack',
    labelId: 'server.landing.load_mod_pack',
    descId: 'server.landing.load_mod_pack_desc',
    stage: null,
    platforms: ['web', 'native'],
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
 *   - an entry with no `stage` — every entry but New Game in this slice —
 *     changes nothing at all, and neither does an id the table does not hold.
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
 *   platform?: 'web'|'native',
 *   build?: string,
 *   dismissed?: boolean,
 *   entries?: Array<object>,
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
 * @returns {{
 *   stage: string,
 *   dismissed: boolean,
 *   openEntryId: string|null,
 *   depth: number,
 *   rootClass: string,
 *   identity: {titleId: string, taglineId: string, logoAltId: string, platformLabelId: string},
 *   entries: Array<{id: string, ordinal: string, labelId: string, descId: string, stage: string|null, selected: boolean, inert: boolean}>,
 *   confirm: null|{titleId: string, eyebrowId: string|null, leadId: string|null, noteId: string|null, ctaId: string, cancelId: string, tone: string, action: string},
 *   status: {platformLabelId: string, sessionId: string, build: {id: string, params: {build: string}}},
 * }}
 *   `confirm` is the OPEN ROW's own confirmation block, republished — never a
 *   second decision made here, and never keyed off an entry id. It is `null`
 *   for every route that simply opens something (New Game's picker), which is
 *   what lets the renderer draw a confirmation from its presence alone.
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
