/**
 * gui/lobby-view.js — Pure selectors behind the client lobby (issue #827).
 *
 * Computes everything the lobby decides (row classes, button kinds, the
 * detail panel's auto-selected console, ready-button state, status-line
 * string-id selection) from (uiState, myToken, lobbyConsole). The DOM writes
 * live in gui/client-lobby-render.js, which consumes this view model.
 *
 * ## Why the string ids are chosen HERE (issue #1369)
 *
 * Three of them used to be picked inline in client.html's DOM glue: the
 * release control's arm→confirm swap (`client.release` vs
 * `client.release_confirm`), the same swap on the detail panel's LEAVE button,
 * and the ready button's four-way mode switch. A renderer that picks a string
 * id is a renderer making a decision, and a decision inside a DOM writer
 * cannot be tested without a document — which is exactly how the mid-round
 * confirm swap came to have no test of its own while `releaseConfirmStep`,
 * the pure half beside it, had three.
 *
 * So every id the lobby says is decided in this file and handed over as an
 * `{ id, params }` pair (the `statusLine` convention, which the host lobby
 * borrowed for the same reason). Text that is DATA rather than a table lookup
 * — the countdown's "5s" — arrives as `{ text }` instead, so the writer still
 * never has to branch on which one it is holding.
 */

/**
 * Detail-panel console auto-select. Post issue #619 one station == one
 * console, so the single console chip is auto-selected the moment the player
 * holds a station; releasing the station clears the selection.
 *
 * @param {string|null} lobbyConsole  currently selected console id
 * @param {{ id?: string }|null} myStation  the station row I hold, or null
 * @returns {string|null} the console id that should be selected
 */
export function nextLobbyConsole(lobbyConsole, myStation) {
  if (!myStation) return null;
  const consoles = myStation.id ? [myStation.id] : [];
  if (consoles.length === 1 && lobbyConsole !== consoles[0]) return consoles[0];
  return lobbyConsole;
}

/**
 * Decide what a click on a mid-round release control should do (issue #771
 * AC3/AC4). Releasing a station during an active round is a two-step
 * arm→confirm (imitating ph-comms-current-message): the first click arms and
 * swaps the button to a confirm string, the second click sends. In the lobby
 * (phase !== 'InProgress') release stays immediate.
 *
 * @param {string} phase   uiState.phase
 * @param {boolean} armed   whether the release is currently armed
 * @returns {{ send: boolean, armed: boolean }}
 *   send=true means dispatch ReleaseStation now; armed is the next armed state.
 */
export function releaseConfirmStep(phase, armed) {
  if (phase === 'InProgress' && !armed) return { send: false, armed: true };
  return { send: true, armed: false };
}

/**
 * Build the lobby view model.
 *
 * @param {object} s  uiState (players, stations, maxPlayers, allReady,
 *                    countdownSecs, phase)
 * @param {string|null} myToken
 * @param {string|null} lobbyConsole  console selection before auto-select
 * @param {{ labelFor?: (station: object) => string,
 *           describeFor?: (station: object) => string,
 *           consoleLabelFor?: (consoleId: string) => string,
 *           ratingLabelFor?: (rating: string) => string,
 *           releaseArmed?: boolean,
 *           stationRatings?: Object<string,string>|null }} [opts]
 *        labelFor resolves a station row to its display label (client.html
 *        passes its string-table stationLabel); describeFor resolves the
 *        station's "what this seat does" line (PRD #1023 module 4 — see the
 *        `description` field below); consoleLabelFor and ratingLabelFor are
 *        the same idea for a console chip and a complexity button, and they
 *        are injected for the same reason: both read the WIRE string table,
 *        whose shape is the page's business and not this module's.
 *        releaseArmed is client.html's arm→confirm latch (see
 *        `releaseConfirmStep`) — it selects the release control's label, which
 *        is a decision and therefore belongs here. stationRatings is
 *        simState.stationRatings for the active-rating highlight.
 * @returns {object} view model — see the return literal.
 */
export function lobbyViewModel(s, myToken, lobbyConsole, opts = {}) {
  const labelFor = opts.labelFor || (st => (st && (st.name || st.id)) || '');
  const describeFor = opts.describeFor || (st => (st && st.description) || '');
  const consoleLabelFor = opts.consoleLabelFor || (id => String(id || ''));
  const ratingLabelFor = opts.ratingLabelFor || (r => String(r || '').toUpperCase());
  const stationRatings = opts.stationRatings || null;
  // Anonymous accessibility eligibility per row (issue #1103 AC1). Injected so
  // this module stays pure: `eligibilityFor(station)` → { eligible, reason },
  // where `reason` is the PRIVATE functional explanation shown only to this
  // player. Default: everything eligible (no profile / no projection).
  const eligibilityFor = opts.eligibilityFor || (() => ({ eligible: true, reason: null }));
  const gms = Array.isArray(s.gms) ? s.gms : [];

  const myPlayer = (s.players || []).find(p => p.token === myToken) || null;
  // Explicit Spectator role (issue #1105) — a real flag on the player, not the
  // "no station" heuristic. Drives the Spectate/Join toggle and the status line.
  const isSpectator = !!(myPlayer && myPlayer.spectator);
  const myStation = myPlayer
    ? ((s.stations || []).find(st => st.holder_token === myToken) || null)
    : null;
  const hasStation = !!myStation;
  const assignedStation = opts.assignedStation || null;
  const selectedConsole = nextLobbyConsole(lobbyConsole, myStation);

  // #771 AC3/AC4: the armed confirm exists ONLY mid-round. Outside InProgress
  // a release is immediate, so there is nothing to confirm and the label never
  // swaps — the same guard client.html used to spell as `inProgress &&
  // releaseArmed` at each of the two controls that show it.
  const confirming = s.phase === 'InProgress' && !!opts.releaseArmed;

  /** Presentation + label for one row's action control, by button kind. */
  const actionFor = (button) => {
    if (button === 'release') {
      return {
        actionClass: 'mine-btn',
        actionDisabled: false,
        actionLabel: { id: confirming ? 'client.release_confirm' : 'client.release' },
      };
    }
    if (button === 'taken') {
      return { actionClass: 'taken-btn', actionDisabled: true, actionLabel: { id: 'client.taken' } };
    }
    if (button === 'ineligible') {
      // Blocked direct claim: disabled, so the control can never send
      // SelectStation. The private reason rides alongside it below.
      return {
        actionClass: 'ineligible-btn',
        actionDisabled: true,
        actionLabel: { id: 'client.station_ineligible' },
      };
    }
    return { actionClass: 'claim-btn', actionDisabled: false, actionLabel: { id: 'client.claim' } };
  };

  const rows = (s.stations || []).map(st => {
    const isMine = !!st.holder_token && st.holder_token === myToken;
    const elig = eligibilityFor(st) || { eligible: true, reason: null };
    const eligible = elig.eligible !== false;
    // A free seat the local player is ineligible for becomes an 'ineligible'
    // button (blocked + privately explained), not a 'claim'. A held/mine seat
    // keeps its button — eligibility only gates NEW direct claims.
    const baseButton = isMine ? 'release' : (st.holder_name ? 'taken' : 'claim');
    const button = baseButton === 'claim' && !eligible ? 'ineligible' : baseButton;
    // The private explanation belongs to a BLOCKED FREE SEAT and to nothing
    // else. `eligible` above is computed for EVERY row — the anonymous set the
    // page reports to the host is a whole-roster fold — so a seat held by
    // somebody else, or by this player, can be ineligible and must still say
    // nothing about why. That is the rule the retired inline glue kept, and
    // the rule the spectator claim list in `client.html` still keeps beside
    // this one; the two rosters have to agree.
    const blocked = button === 'ineligible' && !!elig.reason;
    return {
      id: st.id,
      name: st.name,
      isMine,
      rowClass: 'station-row'
        + (isMine ? ' mine' : '')
        + (st.holder_name && !isMine ? ' taken' : '')
        + (button === 'ineligible' ? ' ineligible' : ''),
      glyph: st.short_code ? st.short_code.substring(0, 2).toUpperCase() : '--',
      label: labelFor(st),
      // What the seat does. Present on EVERY row regardless of button kind —
      // the whole point (PRD #1023 user story 2) is that a free station is
      // readable before it is claimed, not after.
      description: describeFor(st),
      rank: st.rank || null,
      chipId: st.id || null,
      // Post issue #619 a station carries a single (lowercase) id — the
      // station itself is the "console" the chip names.
      chipLabel: st.id ? consoleLabelFor(st.id) : '',
      occupant: (st.holder_name && !isMine) ? st.holder_name : null,
      // 'release' (mine) | 'taken' (someone else's) | 'claim' (free) |
      // 'ineligible' (free but incompatible with this player's assist profile)
      button,
      actionHidden: !!assignedStation,
      ...actionFor(button),
      // Anonymous eligibility + the PRIVATE functional reason (local-only).
      eligible,
      ineligibleReason: eligible ? null : (elig.reason || null),
      // The reason as string ids, so the writer resolves rather than composes.
      // Local-only: these never leave the device, and only the anonymous
      // station-id set is ever reported to the host (issue #1103 AC2).
      ineligibleReasonId: blocked ? 'client.station_ineligible_reason' : null,
      ineligibleFunctionIds: blocked
        ? ((elig.reason.functions || []).map(f => 'client.assist_function.' + f))
        : [],
    };
  });

  const hasRatingChoice = !!(hasStation && myStation.ratings && myStation.ratings.length > 1);
  const activeRating = hasRatingChoice
    ? ((stationRatings && stationRatings[myStation.id]) || myStation.ratings[0])
    : null;

  const detail = hasStation
    ? {
        active: true,
        stationName: labelFor(myStation),
        // The heading is authored copy when a seat is held and a table lookup
        // when it is not, so it is handed over as the pair either way.
        title: { text: labelFor(myStation) },
        stationDescription: describeFor(myStation),
        // Chips carry their own resolved label and selected flag, so the
        // writer neither looks a string up nor recomputes the selection.
        consoles: myStation.id
          ? [{
              id: myStation.id,
              label: consoleLabelFor(myStation.id),
              selected: selectedConsole === myStation.id,
            }]
          : [],
        selectedConsole,
        // The detail panel's LEAVE control is the SAME arm→confirm as the
        // row's RELEASE (they share one handler); only the resting label
        // differs, which is why both ids are decided in one place.
        releaseLabel: assignedStation ? null
          : { id: confirming ? 'client.release_confirm' : 'client.change_station' },
        ratings: hasRatingChoice
          ? {
              list: myStation.ratings.map(r => ({
                name: r,
                label: ratingLabelFor(r),
                active: r === activeRating,
              })),
              active: activeRating,
            }
          : null,
      }
    : {
        active: false,
        stationName: null,
        title: { id: 'client.no_station' },
        stationDescription: '',
        consoles: [],
        selectedConsole: null,
        releaseLabel: null,
        ratings: null,
      };

  // The four modes each carry their own class and label, so the writer sets
  // what it is given instead of re-deriving the mode it was already handed.
  // `label` is `{ id }` for a table lookup and `{ text }` for the countdown,
  // whose seconds are data rather than copy.
  let readyBtn;
  if (myPlayer && hasStation && selectedConsole) {
    const isReady = !!myPlayer.ready;
    if (s.countdownSecs > 0) {
      // Countdown active — keep button in ready state, show timer.
      readyBtn = {
        visible: true, mode: 'countdown', secs: s.countdownSecs, sendReady: false,
        className: 'armed glow', label: { text: s.countdownSecs + 's' },
      };
    } else if (isReady) {
      readyBtn = {
        visible: true, mode: 'ready-confirmed', sendReady: false,
        className: 'armed glow', label: { id: 'client.ready_confirmed' },
      };
    } else if (s.phase === 'InProgress') {
      // In-progress claiming: the same SetReady{true} hand-off, relabelled as
      // "Take Station" (issue #771 AC1). Lobby keeps the 'ready' mode below.
      readyBtn = {
        visible: true, mode: 'take-station', sendReady: true,
        className: 'armed', label: { id: 'client.take_station' },
      };
    } else {
      readyBtn = {
        visible: true, mode: 'ready', sendReady: true,
        className: 'armed', label: { id: 'client.ready' },
      };
    }
  } else {
    readyBtn = { visible: false };
  }

  // Spectate toggle (issue #1105): a participant may join or leave the
  // Spectator role from the lobby. Visible whenever we have a player record;
  // 'join' when already spectating (sends SetSpectator{false}), else 'spectate'
  // (sends SetSpectator{true}). A spectator can't ready, so readyBtn stays
  // hidden for them (hasStation is false → the branch above already hides it).
  const spectateBtn = myPlayer && !assignedStation
    ? {
        visible: true,
        mode: isSpectator ? 'join' : 'spectate',
        label: { id: isSpectator ? 'client.spectator.join' : 'client.spectator.spectate' },
        // What a press sends, decided here rather than negated at the writer.
        sendSpectator: !isSpectator,
      }
    : { visible: false };

  let statusLine;
  if (isSpectator) {
    statusLine = { id: 'client.spectator.lobby_status', params: {} };
  } else if (!myPlayer || !hasStation) {
    statusLine = { id: 'client.status_select_station', params: {} };
  } else if (!selectedConsole) {
    statusLine = { id: 'client.status_select_console', params: { station: labelFor(myStation) } };
  } else if (s.countdownSecs > 0) {
    statusLine = { id: 'client.status_launching', params: { secs: s.countdownSecs } };
  } else if (s.allReady) {
    statusLine = { id: 'client.status_all_ready', params: {} };
  } else if (myPlayer.ready) {
    statusLine = { id: 'client.status_waiting_crew', params: {} };
  } else {
    statusLine = { id: 'client.status_standing_by', params: { station: labelFor(myStation) } };
  }

  return {
    hasStation,
    showRoster: !hasStation && !assignedStation,
    showStationHelp: hasStation,
    helpActions: Array.isArray(opts.helpActions) ? opts.helpActions : [],
    gamepadConnected: !!opts.gamepadConnected,
    gamepadContext: opts.gamepadContext || selectedConsole,
    isSpectator,
    myStation,
    selectedConsole,
    rows,
    detail,
    readyBtn,
    spectateBtn,
    statusLine,
    crew: {
      filled: (s.stations || []).filter(st => st.holder_name).length,
      max: s.maxPlayers || 0,
    },
    // The header's readiness badge. `className` is the colour swap that
    // `#ready-pill.go` reads, and the LABEL beside it is the non-colour cue
    // (WCAG 1.4.1): the two states say different WORDS, not merely cyan
    // against grey, so "all crew ready" is legible — and announceable — to
    // somebody who cannot tell the two colours apart.
    readyPill: {
      label: { id: s.allReady ? 'client.all_crew_ready' : 'client.awaiting_crew' },
      className: s.allReady ? 'go' : '',
    },
    // Equal GM peers are deliberately separate from the crew counter and the
    // Spectator role. The DOM glue renders this as its own labelled region.
    gmGroup: {
      visible: gms.length > 0,
      headingId: 'lobby.gms.heading',
      entries: gms.map(gm => ({
        id: gm && gm.id != null ? String(gm.id) : '',
        name: gm && gm.name != null ? String(gm.name) : '',
        connected: !!(gm && gm.connected),
        ready: !!(gm && gm.connected && gm.ready),
        labelId: gm && gm.connected ? 'lobby.gms.connected' : 'lobby.gms.disconnected',
        readinessLabelId: gm && gm.connected && gm.ready
          ? 'lobby.gms.ready'
          : 'lobby.gms.not_ready',
      })),
    },
    allReady: !!s.allReady,
  };
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.lobbyViewModel = lobbyViewModel;
  window.nextLobbyConsole = nextLobbyConsole;
  window.releaseConfirmStep = releaseConfirmStep;
}
