/**
 * gui/host-lobby-view.js — Pure view-model behind server.html's
 * `__updateLobby()` (issue #1229).
 *
 * Computes everything the host's lobby/Station-grid renderer decides (phase
 * transitions, title/subtitle, crew counter, ready badge, countdown, station
 * cards, the reserved-slots chip, the spectator pill list, the status hint,
 * and the AI-only launch button) from the Rust-pushed `LobbyStatePayload`
 * JSON (`src/core/messages.rs`) plus the previously-seen phase. All DOM
 * writes — and all side effects (audio, wake-state variables) — stay in
 * server.html's inline glue, which consumes this view model.
 *
 * It also decides the native host's **monitor row** (issue #1330) and the
 * per-station **screen rows** (issue #1331), from a second, optional input: the
 * bridge layout a native host pushes beside the lobby state.
 * `hostLobbyMonitorRow` and `hostLobbyStationRows` are exported separately
 * because that is where the whole rule lives — no roster, no row — and the
 * browser host, which has no monitors, reaches both by simply not passing one.
 *
 * SIBLING of gui/lobby-view.js, not a reuse of it: the host consumes a
 * Rust-built roster whose station rows already carry resolved display text
 * (`name`/`short_code`/`rank`/`holder_name`/`preset_names` — every host
 * channel push crosses `localiseHostPayload`, issue #949) and there is no
 * `myToken` — this renders the Viewscreen's read-only card grid with crew
 * dots, not the phone's claim/release rows. Do not attempt to unify the two
 * modules or change the Rust lobby payload shape.
 *
 * Text that still needs localisation at render time (badge/hint/pill
 * strings whose id depends on the data) is returned as `{ id, params }`
 * pairs, exactly like gui/lobby-view.js's `statusLine` — the glue resolves
 * them through `t()`. Plain data already resolved by the host (station
 * names, holder names, ranks) passes through as strings.
 */

/**
 * Decide whether this simulation host may contribute a positive validation
 * vote to the fleet's collective start policy.
 *
 * Selection and boot completion are necessary but not sufficient: the Rust
 * lobby payload owns the final `presentationReady` fact, which moves only once
 * this host's render preload is terminal. Keeping the fold pure makes the
 * selected-but-still-preloading boundary directly testable.
 */
export function fleetStartValidationState({
  role = 'ship',
  fleetLinked = false,
  wasmReady = false,
  worldLoaded = false,
  bootReady = false,
  selectedHull = null,
  validatedHullPath = null,
  presentationReady = false,
} = {}) {
  const selectedPath = selectedHull && selectedHull.template_path;
  const hullReady = role === 'gm' || (
    typeof selectedPath === 'string'
    && selectedPath.length > 0
    && validatedHullPath === selectedPath
  );
  return !!(
    fleetLinked
    && wasmReady
    && worldLoaded
    && bootReady
    && hullReady
    && presentationReady
  );
}

/**
 * @param {object} s  Parsed `LobbyStatePayload` — { phase, scenario_title,
 *                    scenario_body, crew_count, max_players, all_ready,
 *                    stations: [{ name, short_code, rank, holder_name,
 *                    preset_names, consoles? }], spectators: string[],
 *                    gms: [{ id, name, connected, ready }],
 *                    loading_progress?: number, countdown_secs }.
 * @param {string} prevPhase  The phase seen on the previous call (server.html's
 *                    `_lobbyPrevPhase`), used to detect the Loading→InProgress
 *                    and "entered InProgress" edges.
 * @param {object|null} [layout]  The native bridge's monitor row, parsed from
 *                    the `BridgeLayoutPayload` a native host pushes (issue
 *                    #1330). Omitted — and therefore `null` — on the browser
 *                    host, which has no monitors of its own to offer.
 * @returns {object} view model — see the return literal below.
 */
export function hostLobbyViewModel(s, prevPhase, layout) {
  const phase = s.phase;
  const isLobby = phase === 'Lobby';
  const maxP = s.max_players || 0;
  const crewN = s.crew_count || 0;
  const stations = s.stations || [];
  const spectators = s.spectators || [];
  const gms = Array.isArray(s.gms) ? s.gms : [];
  const countdownSecs = s.countdown_secs || 0;

  // ── Phase transitions (loading overlay, audio, panel/QR visibility) ────
  // Pure decisions only — starting or stopping audio, and carrying the join
  // panel's action out through `gui/host-qr.js`, are side effects the glue
  // performs. Both surfaces perform them; only this file decides them.
  const showLoadingOverlay = phase === 'Loading';
  const loadingPct = (showLoadingOverlay && typeof s.loading_progress === 'number')
    ? Math.round(s.loading_progress * 100) + '%'
    : null;
  // Only this edge dismisses the loading overlay — there's nothing to
  // dismiss if it was never shown.
  const dismissLoadingOverlay = phase === 'InProgress' && prevPhase === 'Loading';
  // Audio unlocks on ANY entry into InProgress, not just from Loading: when
  // the asset preload is already complete, a direct start sets InProgress
  // without ever passing through Loading.
  const unlockAudio = phase === 'InProgress' && prevPhase !== 'InProgress';
  const menuMusic = phase === 'Lobby' ? 'start'
    : (phase === 'InProgress' || phase === 'Loading') ? 'stop'
    : null;
  // During InProgress the toggles are the sole join-panel controllers — this
  // transition leaves it untouched (null), which is what lets an operator open
  // the code for a late arrival and have it stay open. The toggles are the host
  // page's settings cog, a phone's ToggleQrCode, and (issue #1329) the native
  // surface's own control.
  const qrOverlayAction = isLobby ? 'show'
    : (phase === 'Loading' || phase === 'GameOver') ? 'hide'
    : null;
  const hideGameOverOverlay = phase !== 'GameOver';

  const transitions = {
    phase,
    prevPhase,
    showLoadingOverlay,
    loadingPct,
    dismissLoadingOverlay,
    unlockAudio,
    menuMusic,
    showPanel: isLobby,
    qrOverlayAction,
    hideGameOverOverlay,
  };

  // ── Title / subtitle ────────────────────────────────────────────────────
  // Both are the world's authored title/description, already resolved
  // strings on the wire. `title` collapses to null (falsy) on an empty
  // string so the glue's `vm.title || t('server.unknown_scenario')` fallback
  // matches the original `s.scenario_title || t(...)` exactly.
  const title = s.scenario_title || null;
  const subtitle = s.scenario_body || '';

  // ── Crew counter + spectator tag ────────────────────────────────────────
  const hasSpecs = spectators.length > 0;
  const crew = {
    count: crewN,
    max: maxP,
    dots: Array.from({ length: maxP }, (_, i) => i < crewN),
    spectatorTag: { visible: hasSpecs, count: spectators.length },
  };

  // ── Ready badge ──────────────────────────────────────────────────────────
  let readyBadge;
  if (countdownSecs > 0) {
    readyBadge = { id: 'server.launching_in', params: { secs: countdownSecs }, className: 'go' };
  } else if (s.all_ready) {
    readyBadge = { id: 'client.all_crew_ready', params: {}, className: 'go' };
  } else {
    readyBadge = { id: 'client.awaiting_crew', params: {}, className: '' };
  }

  // ── Countdown display ────────────────────────────────────────────────────
  const countdown = { visible: countdownSecs > 0, secs: countdownSecs };

  // ── Station grid ─────────────────────────────────────────────────────────
  // The grid shows exactly the ship's defined station roster — no padding to
  // a fixed slot count — so every card is populated; there is no empty-slot
  // variant to compute.
  // The per-station screen rows (issue #1331), keyed by the station id the
  // lobby payload now carries beside the card's display name. Built once for
  // the whole roster and looked up per card, because a station the BRIDGE knows
  // and the lobby roster does not (or the other way round) must simply get no
  // row rather than a row for somebody else's station.
  const screenRows = hostLobbyStationRows(layout);
  const cards = stations.map(st => {
    const claimed = !!st.holder_name;
    const avatar = claimed
      ? { text: st.holder_name.substring(0, 2).toUpperCase(), placeholder: false }
      : { text: st.short_code ? st.short_code.substring(0, 2).toUpperCase() : '--', placeholder: true };
    const consoles = st.consoles && st.consoles.length > 0 ? st.consoles : [];
    const presetPills = (st.preset_names && st.preset_names.length > 0)
      ? st.preset_names.map(pn => ({
          low: pn === 'Low',
          id: pn === 'Low' ? 'server.complexity_low' : 'server.complexity_normal',
        }))
      : [];
    return {
      claimed,
      name: st.name || st.short_code || '',
      rank: st.rank || '',
      // The holder in words, beside the avatar's two letters (issue #1358).
      // A held seat carries the name the host already resolved; a free one
      // carries a string id for the rating that flies it instead, on the same
      // `{ id, params }` convention the badge and the hint use. An unheld
      // Station is not "empty" — Backfill runs its systems — and the card is
      // where a room reads which of the two it is looking at.
      holder: claimed
        ? { text: st.holder_name, id: null, params: {} }
        : { text: null, id: 'station.rating.backfill.name', params: {} },
      avatar,
      consoles,
      presetPills,
      // null on the browser host and on any card the bridge has no row for.
      screens: (screenRows && st.id && screenRows[st.id]) || null,
    };
  });

  // ── Reserved-slots chip ──────────────────────────────────────────────────
  // The grid is sized to the roster (MAX_SLOTS === stations.length), so this
  // is always 0 today; the arithmetic is kept in the shape the original
  // fixed-count grid used, in case a future padded layout revives it.
  const MAX_SLOTS = stations.length;
  const reservedCount = MAX_SLOTS - stations.length;
  const reservedChip = reservedCount > 0
    ? {
        active: true,
        id: reservedCount === 1 ? 'server.slots_reserved.one' : 'server.slots_reserved.other',
        params: { n: reservedCount, max: MAX_SLOTS },
      }
    : { active: false, id: null, params: {} };

  // ── Spectator pill list ──────────────────────────────────────────────────
  const crewPills = stations
    .filter(st => st.holder_name)
    .map(st => ({ kind: 'crew', text: `${st.holder_name} · ${st.name}` }));
  const waitingPills = spectators.map(name => ({
    kind: 'waiting',
    id: 'server.spectator_waiting',
    params: { name },
  }));
  const spectatorPills = crewPills.length === 0 && waitingPills.length === 0
    ? [{ kind: 'empty', id: 'server.no_players', params: {} }]
    : [...crewPills, ...waitingPills];

  // Game Masters are peers of one another, not crew, spectators, Stations or
  // player ships. Keep them in their own labelled group and do not feed them
  // into any of the counts above. A disconnected GM remains visible so the
  // room can distinguish an empty control surface from an operator reconnect.
  const gmGroup = {
    visible: gms.length > 0,
    headingId: 'lobby.gms.heading',
    pills: gms.map(gm => ({
      id: gm && gm.id != null ? String(gm.id) : '',
      name: gm && gm.name != null ? String(gm.name) : '',
      connected: !!(gm && gm.connected),
      ready: !!(gm && gm.connected && gm.ready),
      labelId: gm && gm.connected ? 'lobby.gms.connected' : 'lobby.gms.disconnected',
      readinessLabelId: gm && gm.connected && gm.ready
        ? 'lobby.gms.ready'
        : 'lobby.gms.not_ready',
    })),
  };

  // ── Status hint ──────────────────────────────────────────────────────────
  // `tone` is a class name, not a colour (issue #1358). This module used to
  // return three hexes, which put a slice of the palette inside a pure module
  // that cannot see a stylesheet and did not follow #1357's retint — the
  // lobby's hint was still painting pre-graphite navy greys onto a graphite
  // surface. What this decides is whether the line is LIVE; gui/host-lobby.css
  // decides what live looks like.
  let hint;
  if (countdownSecs > 0) {
    hint = { id: 'server.hint_launching', params: { secs: countdownSecs }, tone: 'live' };
  } else if (crewN === 0) {
    hint = { id: 'server.waiting_players', params: {}, tone: '' };
  } else if (s.all_ready) {
    hint = { id: 'client.status_all_ready', params: {}, tone: 'live' };
  } else {
    hint = { id: 'server.waiting_ready', params: {}, tone: '' };
  }

  // ── AI-only launch button ────────────────────────────────────────────────
  const aiLaunchVisible = crewN === 0 && spectators.length === 0;

  return {
    transitions,
    title,
    subtitle,
    crew,
    readyBadge,
    countdown,
    cards,
    reservedChip,
    spectatorPills,
    gmGroup,
    hint,
    aiLaunchVisible,
    monitorRow: hostLobbyMonitorRow(layout),
  };
}

/**
 * The bridge's monitor row (issue #1330) — one button per connected monitor,
 * the current viewscreen marked, plus whatever the layout law had to say about
 * the last press or the last cable that moved.
 *
 * **Answers `null` unless a bridge actually reported monitors.** That is the
 * whole browser-host story: `server.html` calls the view model with no layout
 * at all, so there is no row, and the renderer draws none — rather than the
 * host page carrying a row it must remember to hide. A native host that has
 * pushed a roster with an empty `monitors` list gets the same answer, because a
 * row of no buttons is not a row.
 *
 * Everything it returns is data or a `{ id, params }` pair, on the same
 * convention `readyBadge` and `hint` use: the words are `strings.csv`'s and the
 * caller's `t()` resolves them. The one thing that is neither is `identity` —
 * the monitor's stable key, which is not shown and is what a press carries back
 * to the host.
 *
 * @param {object|null|undefined} layout parsed `BridgeLayoutPayload`.
 * @returns {object|null} `{ buttons, notices }`, or `null` for no bridge.
 */
export function hostLobbyMonitorRow(layout) {
  const monitors = (layout && layout.monitors) || [];
  if (monitors.length === 0) return null;

  const buttons = monitors.map((m) => {
    // A display the OS named and one it did not are two different sentences,
    // not one sentence with an empty slot: "· 1920×1080" reads as a bug. Shared
    // with the station screen rows (issue #1331) so one display reads the same
    // on both.
    const label = monitorLabel(m);
    // Marks are a list rather than a composed label so a monitor that is both
    // the primary AND the viewscreen does not need a fourth string id, and so a
    // translator never has to reproduce this build's ordering inside one row.
    const marks = [];
    if (m.viewscreen) marks.push({ id: 'server.monitor_row.viewscreen', params: {} });
    if (m.primary) marks.push({ id: 'server.monitor_row.primary', params: {} });
    // The consoles this monitor holds. A press onto one of these is refused by
    // the layout law (no silent eviction), and the row says so up front rather
    // than only after the press — which is what `occupants` is for. Kept
    // separate from `marks` because it is not a status word: it is a list of
    // names, and it is styled and read as one.
    const stations = m.stations || [];
    const occupants = stations.length
      // Joined here rather than in the string table because a translator
      // cannot be handed a list — `t()` interpolates values, and a comma is
      // punctuation rather than English. The host joins its own copy of this
      // list the same way for the refusal that names the same consoles.
      ? { id: 'server.monitor_row.stations', params: { stations: stations.join(', ') } }
      : null;
    return {
      identity: m.identity,
      label,
      marks,
      viewscreen: !!m.viewscreen,
      primary: !!m.primary,
      stations,
      occupants,
    };
  });

  return {
    buttons,
    notices: (layout.notices || []).map((n) => ({ id: n.id, params: n.params || {} })),
  };
}

/**
 * One display's button label, from its entry in the monitor row.
 *
 * Shared by the monitor row and the station screen rows so a display reads the
 * same on both: a name the OS gave and one it did not are two different
 * sentences, not one sentence with an empty slot.
 */
function monitorLabel(m) {
  const width = m.width || 0;
  const height = m.height || 0;
  return m.name
    ? { id: 'server.monitor_row.monitor', params: { name: m.name, w: width, h: height } }
    : { id: 'server.monitor_row.monitor_unnamed', params: { w: width, h: height } };
}
/**
 * The per-station **screen rows** (issue #1331) — one row per station on the
 * ship's roster, each offering the displays that station's console may open on
 * plus an off state that closes it.
 *
 * Returned as an object keyed by station id, because the caller has cards in
 * lobby-roster order and rows in bridge-roster order and the two are joined by
 * that key, never by position.
 *
 * **The law decides; this only draws.** Every button's state comes straight
 * from `BridgeLayout::eligibility` on the wire — `selected`, `eligible`, or
 * `excluded` with `is-viewscreen` or `full` — and the one judgement made here
 * is which of those becomes a button:
 *
 * - `is-viewscreen` is **dropped**. A console never opens on the display
 *   showing the shared view, so a button for it could only ever come back as a
 *   refusal. That is the acceptance criterion's "lists exactly the
 *   non-viewscreen monitors", and it is acting on the law's own reason rather
 *   than re-deriving which screen is the viewscreen.
 * - `full` is **kept and greyed**, carrying its reason. It is a screen the
 *   operator can free a slot on, which is a different fact from a screen no
 *   console ever opens on — and a button that vanished would leave them
 *   wondering where their second monitor went.
 *
 * That reason **names what the screen is holding** when the monitor row knows
 * (issue #1332). Still not a second implementation of the rule: `full` is the
 * law's own verdict and the names are the law's own `occupants_on` list, already
 * on this payload's monitor entry; the row simply joins the two by identity
 * rather than making the operator look the screen up themselves. It matters most
 * for the occupant a station row can never show — a console a hand-authored
 * `--profile` opened for a named crew member, which fills a slot, has no row of
 * its own, and would otherwise grey a screen for no visible reason.
 *
 * And when one of them is that kind, the reason **says the slot is not the
 * lobby's to free**. A `--pane` console holds its half of a screen for the
 * lifetime of the host: it has no station row, no off button, and nothing in
 * this surface can release it. A greyed screen that only listed the names would
 * be true and useless — the operator reads it as "close one of these" and there
 * is nothing to close.
 *
 * A row with no button left at all is the single-monitor bridge: there is
 * nowhere but the viewscreen, so the row carries a message instead of an empty
 * strip of nothing.
 *
 * @param {object|null|undefined} layout parsed `BridgeLayoutPayload`.
 * @returns {object|null} `{ [stationId]: row }`, or `null` for no bridge.
 */
export function hostLobbyStationRows(layout) {
  const monitors = (layout && layout.monitors) || [];
  const stations = (layout && layout.stations) || [];
  if (monitors.length === 0 || stations.length === 0) return null;

  const byIdentity = new Map(monitors.map((m) => [m.identity, m]));
  const rows = {};
  for (const st of stations) {
    const buttons = [];
    for (const screen of (st.monitors || [])) {
      if (screen.excluded === 'is-viewscreen') continue;
      const monitor = byIdentity.get(screen.identity);
      // A screen the monitor row is not drawing has no name and no size to put
      // on a button; the host already skips those, so this is belt to braces.
      if (!monitor) continue;
      const excluded = screen.choice === 'excluded';
      // The law's reason, as the words beside a greyed button. `full` is the
      // only one that reaches here; `is-viewscreen` was dropped above. When the
      // monitor row knows what that screen is holding, the reason says so —
      // the same `occupants_on` list the monitor button shows and the same one
      // the refusal names, in the same ORDER, so all three agree by
      // construction: the list is the law's drawn order (issue #1332's fix
      // round), which is left to right on a side-by-side screen and top to
      // bottom on a stacked one, so the words match the glass rather than
      // merely naming the same set.
      const holding = monitor.stations || [];
      // The ones with no off button anywhere: consoles a hand-authored
      // `--profile` opened. They cost a slot like any other console and there is
      // no lobby control that frees one, so a greyed screen held by one has to
      // say that rather than sending the operator hunting the station rows for
      // an unassign that does not exist.
      const authored = monitor.reserved || [];
      let reason = null;
      if (screen.excluded === 'full') {
        // Joined here rather than in the string table, for `occupants`' reason:
        // `t()` interpolates values, and a comma is punctuation rather than
        // English a translator can be handed a list for.
        if (authored.length) {
          reason = {
            id: 'server.station_row.full_authored',
            params: { stations: holding.join(', '), authored: authored.join(', ') },
          };
        } else if (holding.length) {
          reason = { id: 'server.station_row.full_holding', params: { stations: holding.join(', ') } };
        } else {
          reason = { id: 'server.station_row.full', params: {} };
        }
      }
      buttons.push({
        identity: screen.identity,
        label: monitorLabel(monitor),
        selected: screen.choice === 'selected',
        disabled: excluded,
        reason,
      });
    }
    rows[st.station] = buttons.length === 0
      // Phones only: the one display is the viewscreen, and a console never
      // covers it.
      ? {
          station: st.station,
          assigned: null,
          off: null,
          buttons: [],
          message: { id: 'server.station_row.needs_second_monitor', params: {} },
        }
      : {
          station: st.station,
          assigned: st.assigned_to || null,
          // The off state is a button like any other, and `selected` when the
          // console is closed — so which state the row is in is legible without
          // comparing the others.
          off: { selected: !st.assigned_to },
          buttons,
          message: null,
        };
  }
  return rows;
}

// Expose for the classic (non-module) script in server.html.
if (typeof window !== 'undefined') {
  window.hostLobbyViewModel = hostLobbyViewModel;
  window.fleetStartValidationState = fleetStartValidationState;
}
