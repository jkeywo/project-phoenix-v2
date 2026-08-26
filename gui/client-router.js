/**
 * gui/client-router.js — Pure per-message driver for the client shell
 * (issue #827).
 *
 * Extracted from client.html's handleMessage() switch. Given an inbound
 * ServerMessage and the lobby/lifecycle uiState, applies the per-case
 * uiState mutations and returns a description of everything else the shell
 * glue must do:
 *
 *   {
 *     sideEffects:      [{ effect: '<name>', ...args }] in application order,
 *     rebuildStations:  bool — caller must re-fold the station roster
 *                       (which itself schedules a render),
 *     shouldRender:     bool — caller should scheduleRender() (the
 *                       showingLobbyDuringGame guard is already applied),
 *     pendingMidGameClaim: bool — new value for the caller's flag,
 *     bezelAlertOn:     bool — new value for the caller's bezel latch,
 *     myName:           string|undefined — set only when the local player's
 *                       name changed (NameChanged for my token),
 *   }
 *
 * Side-effect names (DOM work stays in client.html's thin glue):
 *   'mount-consoles'     { shipStations }        — (re)mount console iframes
 *   'ship-theme'         { href }                — inject/update ship CSS link
 *   'ship-info'          { title, description }  — lobby header text
 *   'status'             { id }                  — status line string id
 *   'hide-loading'                              — hide the asset overlay
 *   'show-loading'       { pct }                 — show overlay at pct
 *   'bezel-alert'        { on }                  — phone-bezel red alert
 *   'vibrate'            { duration }            — navigator.vibrate ms
 *   'coordination-popup' { payload, senderLabel } — show the popup
 *
 * The state modules (lobbyState / simState / commsState apply) and the
 * dirty-console iframe fan-out run BEFORE this router — see handleMessage().
 *
 * Preserved exactly from the inline switch:
 *   - per-case return (no render) vs break (render via the final guard)
 *   - the showingLobbyDuringGame early-returns on SimState / BlackboardUpdate
 *   - the StationAssigned clears-other-holder fallback logic
 */

/** The local player's entry in uiState.players, or undefined. */
export function playerFor(uiState, myToken) {
  return (uiState.players || []).find(p => p.token === myToken);
}

/** The local player's held station id (string), or null. */
export function playerStation(uiState, myToken) {
  const m = playerFor(uiState, myToken);
  if (!m) return null;
  return typeof m.station === 'string' ? m.station
    : (m.station && typeof m.station.id === 'string' ? m.station.id : null);
}

/**
 * True when the local player is an explicit Spectator (issue #1105) — the real
 * session role, not the "no station" heuristic. A Spectator gets the crew-public
 * summary surface, never the station-select lobby.
 */
export function isSpectator(uiState, myToken) {
  const player = playerFor(uiState, myToken);
  return !!(player && player.spectator);
}

/**
 * True while the station-select lobby panel is being shown over an in-progress
 * game: mid-game claimants who have picked a station but not yet pressed Ready,
 * and stationless-but-not-spectator players. Single source of truth for
 * render()'s vis.lobby override and the SimState/BlackboardUpdate render guards
 * — those messages arrive at ~10Hz and must skip the lobby DOM rebuild in this
 * state, or a click on Ready/Leave can land between the old button being torn
 * down and the new one's listener being attached.
 *
 * An explicit Spectator (issue #1105) is deliberately EXCLUDED: they render the
 * summary surface instead, which is read-only and must re-paint live on each
 * SimState — so it must NOT be gated behind this "skip the lobby rebuild" guard.
 */
export function showingLobbyDuringGame(uiState, myToken, pendingMidGameClaim) {
  if (uiState.phase !== 'InProgress') return false;
  const player = playerFor(uiState, myToken);
  if (player && player.spectator) return false; // spectators get the summary surface
  return !!pendingMidGameClaim || !playerStation(uiState, myToken)
    || !!(player && !player.ready);
}

/**
 * Route one inbound ServerMessage. Mutates ctx.uiState in place (the same
 * mutations the inline switch performed) and returns the side-effect plan.
 *
 * @param {object} msg  Inbound ServerMessage ({ type, data }).
 * @param {{ uiState: object, myToken: string|null,
 *           pendingMidGameClaim: boolean, bezelAlertOn: boolean,
 *           lobbyState: object|null, redAlert: boolean }} ctx
 *        `lobbyState` is the gui/lobby-state.js singleton (null before the
 *        module loads); `redAlert` is the current simState.redAlert flag,
 *        read by the caller just before routing.
 */
export function routeMessage(msg, ctx) {
  const { uiState, myToken, lobbyState } = ctx;
  let pendingMidGameClaim = !!ctx.pendingMidGameClaim;
  let bezelAlertOn = !!ctx.bezelAlertOn;
  const sideEffects = [];
  let rebuildStations = false;
  let myName;

  const done = (shouldRender) => ({
    sideEffects, rebuildStations, shouldRender, pendingMidGameClaim, bezelAlertOn, myName,
  });

  switch (msg.type) {
    case 'Welcome':
      if (lobbyState) {
        uiState.phase        = lobbyState.phase;
        uiState.players      = lobbyState.players;
        uiState.shipStations = lobbyState.shipStations;
      } else {
        uiState.phase        = msg.data.state.phase;
        uiState.shipStations = msg.data.ship_stations || { stations: [] };
        uiState.players      = msg.data.state.players.map(p => ({ ...p }));
      }
      // Mount console sections dynamically from ship_stations. The iframes'
      // load listeners re-push each console's snapshot once loaded, so the
      // pre-mount dirty-console fan-out is not lost.
      sideEffects.push({ effect: 'mount-consoles', shipStations: uiState.shipStations });
      {
        const shipCss = (msg.data.ship_config || {}).ship_css;
        if (shipCss) sideEffects.push({ effect: 'ship-theme', href: shipCss });
      }
      if (msg.data.state.world) {
        sideEffects.push({
          effect: 'ship-info',
          title: msg.data.state.world.scenario_title || 'Phoenix',
          description: msg.data.state.world.scenario_description || '',
        });
      }
      sideEffects.push({ effect: 'status', id: 'client.status_connected' });
      rebuildStations = true;
      break;
    case 'PlayerJoined': {
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        const p = msg.data.player;
        const existing = uiState.players.findIndex(x => x.token === p.token);
        if (existing >= 0) uiState.players[existing] = { ...p };
        else uiState.players.push({ ...p });
      }
      rebuildStations = true;
      break;
    }
    case 'PlayerLeft':
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        uiState.players = uiState.players.filter(p => p.token !== msg.data.token);
      }
      rebuildStations = true;
      break;
    case 'StationAssigned': {
      const stationId = msg.data.station_id
        ? (typeof msg.data.station_id === 'string' ? msg.data.station_id : msg.data.station_id.id)
        : null;
      if (msg.data.token === myToken && !stationId) {
        pendingMidGameClaim = false;
      }
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        const { token, station } = msg.data;
        // A station is held by at most one player — clear any other holder first.
        if (stationId) {
          for (const p of uiState.players) {
            if (p.token !== token && (typeof p.station === 'string' ? p.station : (p.station && p.station.id)) === stationId) {
              p.station = null;
            }
          }
        }
        const target = uiState.players.find(p => p.token === token);
        if (target) {
          target.station = stationId || station || null;
        }
      }
      rebuildStations = true;
      break;
    }
    case 'NameChanged': {
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        const p = uiState.players.find(p => p.token === msg.data.token);
        if (p) p.name = msg.data.name;
      }
      if (msg.data.token === myToken) myName = msg.data.name;
      rebuildStations = true;
      break;
    }
    case 'GameStartCountdown':
      uiState.countdownSecs = (msg.data && msg.data.remaining_secs) || 0;
      rebuildStations = true;
      break;
    case 'GameStarted':
      uiState.phase = 'InProgress';
      uiState.countdownSecs = 0;
      sideEffects.push({ effect: 'hide-loading' });
      break;
    case 'LoadingProgress': {
      const frac = msg.data?.fraction ?? 0;
      sideEffects.push({ effect: 'show-loading', pct: Math.round(frac * 100) });
      break;
    }
    case 'SimState': {
      // SimSnapshot was applied by the state modules; the console fan-out
      // happened in pushDirtyConsoles. Don't re-render the full lobby at
      // 10Hz when spectating or when a mid-game station claim is still
      // pending Ready — it destroys CSS hover state and tears down the
      // Ready/Leave button listeners.
      if (showingLobbyDuringGame(uiState, myToken, pendingMidGameClaim)) return done(false);
      break;
    }
    case 'WorldSetup':
    case 'EntitySpawned':
    case 'AsteroidSpawned': {
      // World / entity list handled by the sim-state module; no re-render.
      return done(false);
    }
    case 'BlackboardUpdate': {
      const updatedSystems = new Set((msg.data.updates || []).map(([id]) => id));
      if (updatedSystems.has('captain') || updatedSystems.has('viewscreen')) {
        // Phone-bezel red-alert glow: toggle only on change.
        const nextAlert = !!ctx.redAlert;
        if (bezelAlertOn !== nextAlert) {
          bezelAlertOn = nextAlert;
          sideEffects.push({ effect: 'bezel-alert', on: nextAlert });
        }
      }
      // Same guard as SimState.
      if (showingLobbyDuringGame(uiState, myToken, pendingMidGameClaim)) return done(false);
      break;
    }
    case 'TargetLock':
    case 'WeaponsUpdate':
    case 'BeamStarted':
    case 'BeamEnded':
    case 'SystemHullUpdate':
    case 'RepairState':
    case 'PowerState':
    case 'ShieldStatus':
    case 'AsteroidDestroyed':
    case 'EntityDespawned': {
      // State applied by the modules, iframe pushes fanned out by
      // pushDirtyConsoles; fall through to the render guard.
      break;
    }
    case 'DamageTaken': {
      const { hull } = msg.data;
      if (hull > 0) {
        const clamped = Math.min(hull, 30);
        const duration = Math.round(50 + (clamped / 30) * 250); // 50–300 ms
        sideEffects.push({ effect: 'vibrate', duration });
      }
      return done(false);
    }
    case 'CoordinationPopup': {
      const { payload, sender_label, target } = msg.data;
      sideEffects.push({ effect: 'coordination-popup', payload, senderLabel: sender_label, targetLabel: target });
      return done(false);
    }
    case 'CommsState': {
      // Handled by the comms-state module; pushDirtyConsoles refreshed the
      // console already.
      break;
    }
    case 'ShipDestroyed':
      uiState.shipDestroyed = true;
      break;
    case 'GameOver':
      uiState.phase = 'GameOver';
      uiState.gameOverReason = msg.data.reason || null;
      break;
    case 'RatingChanged': {
      // sim-state.js already updated stationRatings; pushDirtyConsoles pushed
      // the captain console (unconditionally — see DIRTY_ALWAYS_PUSH).
      break;
    }
    case 'ReadyChanged':
      if (msg.data.token === myToken && msg.data.ready) pendingMidGameClaim = false;
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        const p = uiState.players.find(p => p.token === msg.data.token);
        if (p) p.ready = msg.data.ready;
      }
      rebuildStations = true;
      break;
    case 'SpectatorChanged':
      // Explicit Spectator role delta (issue #1105). The seat-clear and unready
      // ride on their own StationAssigned{None}/ReadyChanged{false} messages;
      // this just tracks the role flag. rebuildStations re-folds the roster and
      // schedules the render that swaps to (or from) the summary surface.
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        const p = uiState.players.find(p => p.token === msg.data.token);
        if (p) p.spectator = msg.data.spectator;
      }
      rebuildStations = true;
      break;
    case 'AfkChanged':
      // AFK presence delta (issue #1104). The delegation/restore ride on their
      // own RatingChanged messages and the seat is never vacated (no
      // StationAssigned), so this only tracks the presence flag on the roster
      // for presence rendering — mirroring ReadyChanged/SpectatorChanged.
      if (lobbyState) {
        uiState.players = lobbyState.players;
      } else {
        const p = uiState.players.find(p => p.token === msg.data.token);
        if (p) p.afk = msg.data.afk;
      }
      rebuildStations = true;
      break;
    case 'ObjectiveSummary':
      break; // fall through to the render guard
    default:
      return done(false); // unknown — skip re-render
  }
  // Don't re-render the full lobby at high frequency during mid-game
  // spectating/claiming — renderLobby destroys button listeners on rebuild.
  // rebuildStations-cases schedule their own render via the roster re-fold.
  return done(!showingLobbyDuringGame(uiState, myToken, pendingMidGameClaim));
}

// Expose for the non-module inline script in client.html. NOTE:
// showingLobbyDuringGame is deliberately NOT window-exposed — client.html's
// classic script declares its own single-argument wrapper of the same name
// at global scope, and a module assignment would silently replace it.
if (typeof window !== 'undefined') {
  window.routeMessage = routeMessage;
  window.routerIsSpectator = isSpectator;
}
