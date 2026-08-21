/**
 * gui/lobby-view.js — Pure selectors behind client.html's renderLobby()
 * (issue #827).
 *
 * Computes everything renderLobby decides (row classes, button kinds, the
 * detail panel's auto-selected console, ready-button state, status-line
 * string-id selection) from (uiState, myToken, lobbyConsole). All DOM writes
 * stay in client.html's inline glue, which consumes this view model.
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
 *           stationRatings?: Object<string,string>|null }} [opts]
 *        labelFor resolves a station row to its display label (client.html
 *        passes its string-table stationLabel); describeFor resolves the
 *        station's "what this seat does" line (PRD #1023 module 4 — see the
 *        `description` field below); stationRatings is
 *        simState.stationRatings for the active-rating highlight.
 * @returns {object} view model — see the return literal.
 */
export function lobbyViewModel(s, myToken, lobbyConsole, opts = {}) {
  const labelFor = opts.labelFor || (st => (st && (st.name || st.id)) || '');
  const describeFor = opts.describeFor || (st => (st && st.description) || '');
  const stationRatings = opts.stationRatings || null;
  // Anonymous accessibility eligibility per row (issue #1103 AC1). Injected so
  // this module stays pure: `eligibilityFor(station)` → { eligible, reason },
  // where `reason` is the PRIVATE functional explanation shown only to this
  // player. Default: everything eligible (no profile / no projection).
  const eligibilityFor = opts.eligibilityFor || (() => ({ eligible: true, reason: null }));

  const myPlayer = (s.players || []).find(p => p.token === myToken) || null;
  // Explicit Spectator role (issue #1105) — a real flag on the player, not the
  // "no station" heuristic. Drives the Spectate/Join toggle and the status line.
  const isSpectator = !!(myPlayer && myPlayer.spectator);
  const myStation = myPlayer
    ? ((s.stations || []).find(st => st.holder_token === myToken) || null)
    : null;
  const hasStation = !!myStation;
  const selectedConsole = nextLobbyConsole(lobbyConsole, myStation);

  const rows = (s.stations || []).map(st => {
    const isMine = !!st.holder_token && st.holder_token === myToken;
    const elig = eligibilityFor(st) || { eligible: true, reason: null };
    const eligible = elig.eligible !== false;
    // A free seat the local player is ineligible for becomes an 'ineligible'
    // button (blocked + privately explained), not a 'claim'. A held/mine seat
    // keeps its button — eligibility only gates NEW direct claims.
    const baseButton = isMine ? 'release' : (st.holder_name ? 'taken' : 'claim');
    const button = baseButton === 'claim' && !eligible ? 'ineligible' : baseButton;
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
      occupant: (st.holder_name && !isMine) ? st.holder_name : null,
      // 'release' (mine) | 'taken' (someone else's) | 'claim' (free) |
      // 'ineligible' (free but incompatible with this player's assist profile)
      button,
      // Anonymous eligibility + the PRIVATE functional reason (local-only).
      eligible,
      ineligibleReason: eligible ? null : (elig.reason || null),
    };
  });

  const detail = hasStation
    ? {
        active: true,
        stationName: labelFor(myStation),
        stationDescription: describeFor(myStation),
        consoles: myStation.id ? [myStation.id] : [],
        selectedConsole,
        ratings: (myStation.ratings && myStation.ratings.length > 1)
          ? {
              list: myStation.ratings,
              active: (stationRatings && stationRatings[myStation.id]) || myStation.ratings[0],
            }
          : null,
      }
    : { active: false, stationName: null, stationDescription: '', consoles: [], selectedConsole: null, ratings: null };

  let readyBtn;
  if (myPlayer && hasStation && selectedConsole) {
    const isReady = !!myPlayer.ready;
    if (s.countdownSecs > 0) {
      // Countdown active — keep button in ready state, show timer.
      readyBtn = { visible: true, mode: 'countdown', secs: s.countdownSecs, sendReady: false };
    } else if (isReady) {
      readyBtn = { visible: true, mode: 'ready-confirmed', sendReady: false };
    } else if (s.phase === 'InProgress') {
      // In-progress claiming: the same SetReady{true} hand-off, relabelled as
      // "Take Station" (issue #771 AC1). Lobby keeps the 'ready' mode below.
      readyBtn = { visible: true, mode: 'take-station', sendReady: true };
    } else {
      readyBtn = { visible: true, mode: 'ready', sendReady: true };
    }
  } else {
    readyBtn = { visible: false };
  }

  // Spectate toggle (issue #1105): a participant may join or leave the
  // Spectator role from the lobby. Visible whenever we have a player record;
  // 'join' when already spectating (sends SetSpectator{false}), else 'spectate'
  // (sends SetSpectator{true}). A spectator can't ready, so readyBtn stays
  // hidden for them (hasStation is false → the branch above already hides it).
  const spectateBtn = myPlayer
    ? { visible: true, mode: isSpectator ? 'join' : 'spectate' }
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
    allReady: !!s.allReady,
  };
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.lobbyViewModel = lobbyViewModel;
  window.nextLobbyConsole = nextLobbyConsole;
  window.releaseConfirmStep = releaseConfirmStep;
}
