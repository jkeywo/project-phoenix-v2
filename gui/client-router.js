/**
 * gui/client-router.js — Pure reducer-result driver for the client shell.
 *
 * The lobby, simulation and Comms reducers own ServerMessage interpretation.
 * This module receives only their merged semantic result, mirrors the already
 * reduced lobby state into the shell view-model, and routes ordered lifecycle
 * and presentation effects. It never receives or re-decodes a ServerMessage.
 *
 * Result shape:
 *
 *   {
 *     sideEffects:      [{ effect: '<name>', ...args }] in application order,
 *     pendingMidGameClaim: bool,
 *     bezelAlertOn:     bool,
 *     myName:           string|undefined,
 *   }
 *
 * `sideEffects` remain repeatable. In particular, vibration and Coordination
 * feedback are never folded merely because their payloads happen to match.
 */

import { CHANGE_DOMAINS, REDUCER_EFFECTS } from './reducer-result.js';

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

/** True when the local player is an explicit Spectator. */
export function isSpectator(uiState, myToken) {
  const player = playerFor(uiState, myToken);
  return !!(player && player.spectator);
}

/**
 * True while the station-select lobby panel is being shown over an in-progress
 * game. High-rate render requests are suppressed here so they cannot tear down
 * a Ready/Leave control between pointer down and click. Explicit Spectators
 * instead use the live, read-only summary surface and are not suppressed.
 */
export function showingLobbyDuringGame(uiState, myToken, pendingMidGameClaim) {
  if (uiState.phase !== 'InProgress') return false;
  const player = playerFor(uiState, myToken);
  if (player && player.spectator) return false;
  return !!pendingMidGameClaim || !playerStation(uiState, myToken)
    || !!(player && !player.ready);
}

/** Copy the already-reduced authoritative lobby projection into shell state. */
function syncLobbyState(uiState, lobbyState) {
  if (!lobbyState) return;
  uiState.phase = lobbyState.phase;
  uiState.players = lobbyState.players;
  uiState.shipStations = lobbyState.shipStations;
  uiState.countdownSecs = lobbyState.countdownSecs;
  uiState.gameOverReason = lobbyState.gameOverReason;
}

/**
 * Route one merged reducer result.
 *
 * Unknown effects are ignored individually: one future reducer effect must not
 * prevent a later known feedback effect in the same ordered result.
 *
 * @param {{changedDomains?: Set<string>, effects?: object[]}|null|undefined} result
 * @param {{uiState: object, myToken: string|null,
 *          pendingMidGameClaim: boolean, bezelAlertOn: boolean,
 *          lobbyState: object|null}} ctx
 */
export function routeReducerResult(result, ctx) {
  const { uiState, myToken, lobbyState } = ctx;
  let pendingMidGameClaim = !!ctx.pendingMidGameClaim;
  let bezelAlertOn = !!ctx.bezelAlertOn;
  const sideEffects = [];
  let myName;

  if (result?.changedDomains?.has(CHANGE_DOMAINS.LOBBY)) {
    // The reducer is the one and only message interpreter. No payload fallback
    // lives here: if it did not accept the message there is nothing to mirror.
    syncLobbyState(uiState, lobbyState);
  }

  for (const fx of result?.effects || []) {
    switch (fx && fx.effect) {
      case REDUCER_EFFECTS.MOUNT_CONSOLES:
      case REDUCER_EFFECTS.SHIP_THEME:
      case REDUCER_EFFECTS.SHIP_INFO:
      case REDUCER_EFFECTS.STATUS:
      case REDUCER_EFFECTS.HIDE_LOADING:
      case REDUCER_EFFECTS.SHOW_LOADING:
      case REDUCER_EFFECTS.VIBRATE:
      case REDUCER_EFFECTS.COORDINATION_POPUP:
      case REDUCER_EFFECTS.SETTLE_SCENARIO_PICK:
      case REDUCER_EFFECTS.REFRESH_SETTINGS:
      case REDUCER_EFFECTS.REBUILD_STATIONS:
      case REDUCER_EFFECTS.REPORT_ELIGIBILITY:
        sideEffects.push(fx);
        break;
      case REDUCER_EFFECTS.BEZEL_ALERT: {
        const nextAlert = !!fx.on;
        if (bezelAlertOn !== nextAlert) {
          bezelAlertOn = nextAlert;
          sideEffects.push({ ...fx, on: nextAlert });
        }
        break;
      }
      case REDUCER_EFFECTS.REQUEST_RENDER:
        if (fx.force || !showingLobbyDuringGame(
          uiState, myToken, pendingMidGameClaim,
        )) {
          sideEffects.push(fx);
        }
        break;
      case REDUCER_EFFECTS.STATION_ASSIGNED:
        if (fx.token === myToken && !fx.stationId) pendingMidGameClaim = false;
        break;
      case REDUCER_EFFECTS.READY_CHANGED:
        if (fx.token === myToken && fx.ready) pendingMidGameClaim = false;
        break;
      case REDUCER_EFFECTS.NAME_CHANGED:
        if (fx.token === myToken) myName = fx.name;
        break;
      case REDUCER_EFFECTS.SHIP_DESTROYED:
        uiState.shipDestroyed = true;
        break;
      default:
        break;
    }
  }

  return {
    sideEffects,
    pendingMidGameClaim,
    bezelAlertOn,
    myName,
  };
}

if (typeof window !== 'undefined') {
  window.routeReducerResult = routeReducerResult;
  window.routerIsSpectator = isSpectator;
}
