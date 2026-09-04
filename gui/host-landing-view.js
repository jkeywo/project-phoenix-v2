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
 *
 * Adding an entry is adding a row. Nothing below reads an id by name.
 *
 * ## Only one entry works in this slice, and that is deliberate
 *
 * `new_game` carries `stage: 'world-picker'`; the other four carry `stage:
 * null` and are inert — they render, and clicking them changes nothing. An
 * inert entry that silently pretended to open something would be worse than a
 * button that plainly does not work yet, and each of the four has a sibling
 * issue that gives it a stage of its own. [`nextOpenEntry`] is where "inert"
 * is enforced, in one place, rather than at each caller.
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
    id: 'connect_host',
    labelId: 'server.landing.connect_host',
    descId: 'server.landing.connect_host_desc',
    stage: null,
    platforms: ['web', 'native'],
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
 * @returns {{
 *   stage: string,
 *   openEntryId: string|null,
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
  const list = landingEntries(platform, opts.entries);

  // An `openEntryId` naming an entry this platform does not offer (or an
  // entry that never had a stage) reads as closed rather than as a stage
  // nothing can render. The caller's memory can outlive a menu change — a
  // native host and a browser host share this module and not their rows.
  const open = list.find(function (e) {
    return e.id === opts.openEntryId && !!e.stage;
  }) || null;

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
    stage: open ? open.stage : 'idle',
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
