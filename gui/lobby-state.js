/**
 * gui/lobby-state.js — Pure JS port of src/lobby/client_panel.rs
 * (LobbyState + LobbyView derivations). Issue #460.
 *
 * Maintains the client's authoritative model of the shared lobby state by
 * applying inbound ServerMessage objects (already-parsed JSON of the form
 * `{ type: 'VariantName', data: {...} }`, matching the serde
 * `#[serde(tag = "type", content = "data")]` wire format).
 *
 * DOM-free so it can be unit-tested in Node via Vitest. Exposed on `window`
 * as `window.lobbyState` (singleton) for the inline script in client.html.
 */

/** All ship stations the lobby UI knows how to render, in display order.
 *
 * Lowercase station ids (issue #618). Consistent with the server-side
 * `StationId` newtype; after issue #619 the client no longer speaks the
 * legacy PascalCase Console enum names — every station-facing wire field
 * (`Player.station`, `StationAssigned.station_id`) is a lowercase id.
 */
export const ALL_STATIONS = Object.freeze([
  'captain', 'helm', 'tactical', 'repair', 'sensors', 'science',
  'shields', 'navigation', 'power', 'comms',
]);

/**
 * Normalise a player's station id.
 * Reads the lowercase station id from `player.station` (or the legacy
 * `station.id` if the wire has the object shape). Returns null when no
 * station is held (spectator).
 */
export function playerStationId(player) {
  if (!player) return null;
  if (typeof player.station === 'string') return player.station;
  if (player.station && typeof player.station.id === 'string') return player.station.id;
  return null;
}

function normalisePlayer(p) {
  return { ready: false, ...p, station: playerStationId(p) };
}

function defaultShipStations() {
  return { stations: [] };
}

/**
 * The client's authoritative model of the shared lobby state.
 * Mirrors `LobbyState` in src/lobby/client_panel.rs.
 */
export class LobbyState {
  constructor() {
    this.reset();
  }

  reset() {
    /** GamePhase: 'Lobby' | 'Loading' | 'InProgress' | 'GameOver' */
    this.phase = 'Lobby';
    /** Array of { token, name, station: string|null, connected } */
    this.players = [];
    /** ShipStations: { stations: [StationDef] } */
    this.shipStations = defaultShipStations();
    /** ShipClientConfig — per-ship static config from Welcome. */
    this.shipConfig = {};
    /** Reason string for game over, if the game has ended. */
    this.gameOverReason = null;
    this.scenarioTitle = '';
    this.scenarioBody = '';
    /** Remaining seconds in the pre-game countdown, 0 when not counting. */
    this.countdownSecs = 0;
    /** True when the host has returned to scenario selection after GameOver
     *  and clients should show a waiting screen instead of the lobby. */
    this.waitingForScenario = false;
  }

  /**
   * Replace the entire lobby state — used on Welcome, the authoritative
   * initial sync. Mirrors `LobbyState::replace_from`.
   */
  replaceFrom(state, shipStations, shipConfig) {
    this.phase = state.phase || 'Lobby';
    this.shipStations = shipStations || defaultShipStations();
    this.players = (state.players || []).map(p => normalisePlayer(p));
    this.shipConfig = shipConfig || {};
    this.scenarioTitle = (state.world && state.world.scenario_title) || '';
    this.scenarioBody = (state.world && state.world.scenario_description) || '';
  }

  /**
   * Apply a single inbound ServerMessage `{ type, data }`. Variants that
   * don't affect the lobby are ignored. Mirrors `LobbyState::apply`.
   */
  apply(msg) {
    if (!msg || !msg.type) return;
    const d = msg.data || {};
    switch (msg.type) {
      case 'Welcome':
        this.replaceFrom(d.state || {}, d.ship_stations, d.ship_config);
        this.waitingForScenario = false;
        break;
      case 'PlayerJoined': {
        const player = normalisePlayer(d.player || {});
        const idx = this.players.findIndex(p => p.token === player.token);
        if (idx >= 0) this.players[idx] = player;
        else this.players.push(player);
        break;
      }
      case 'PlayerLeft':
        this.players = this.players.filter(p => p.token !== d.token);
        break;
      case 'NameChanged': {
        const p = this.players.find(p => p.token === d.token);
        if (p) p.name = d.name;
        break;
      }
      case 'ReadyChanged': {
        const p = this.players.find(p => p.token === d.token);
        if (p) p.ready = d.ready;
        break;
      }
      case 'StationAssigned': {
        // Post issue #619: StationAssigned carries `station_id` (lowercase
        // station id) and the legacy `station` (display name). A station is
        // held by at most one player at a time — clear it from anyone else
        // first, then assign to the named token.
        const stationId = (d.station_id && (typeof d.station_id === 'string' ? d.station_id : d.station_id.id)) || null;
        if (stationId) {
          for (const p of this.players) {
            if (p.token !== d.token && p.station === stationId) {
              p.station = null;
            }
          }
        }
        const target = this.players.find(p => p.token === d.token);
        if (target) {
          target.station = stationId;
        }
        break;
      }
      case 'GameStartCountdown':
        this.countdownSecs = d.remaining_secs || 0;
        break;
      case 'GameStarted':
        this.phase = 'InProgress';
        this.countdownSecs = 0;
        break;
      case 'GameOver':
        this.phase = 'GameOver';
        this.gameOverReason = d.reason != null ? d.reason : '';
        break;
      case 'ReturnedToLobby':
        this.phase = 'Lobby';
        this.gameOverReason = null;
        this.countdownSecs = 0;
        this.waitingForScenario = true;
        break;
      case 'ScenarioLoaded':
        this.waitingForScenario = false;
        break;
      default:
        // Not relevant to the lobby model.
        break;
    }
  }

  // ── LobbyView derivations (token-parameterised) ───────────────────────────

  /** Station id held by the local player (null if no matching token or spectator). */
  playerStation(myToken) {
    const p = this.players.find(p => p.token === myToken);
    return p ? p.station : null;
  }

  isCaptain(myToken)    { return this.playerStation(myToken) === 'captain'; }
  isHelm(myToken)       { return this.playerStation(myToken) === 'helm'; }
  isSensors(myToken)    { return this.playerStation(myToken) === 'sensors'; }
  isShields(myToken)    { return this.playerStation(myToken) === 'shields'; }
  isNavigation(myToken) { return this.playerStation(myToken) === 'navigation'; }
  isRepair(myToken)     { return this.playerStation(myToken) === 'repair'; }
  isPower(myToken)      { return this.playerStation(myToken) === 'power'; }

  /** True if the local player is a spectator (no station assigned). */
  isSpectator(myToken) {
    return this.playerStation(myToken) == null;
  }

  /** True when the lobby panel should be visible. */
  showLobbyPanel(myToken) {
    if (this.phase === 'Lobby' && !this.waitingForScenario) return true;
    if (this.phase === 'Loading') return true;
    if (this.phase === 'InProgress') {
      const player = this.players.find(p => p.token === myToken);
      return this.isSpectator(myToken) || !!(player && !player.ready);
    }
    return false; // GameOver
  }

  /** True when the "Game in progress" spectator banner should appear. */
  gameInProgressBanner(myToken) {
    return this.phase === 'InProgress' && this.isSpectator(myToken);
  }

  /** True during the asset pre-loading phase. */
  isLoading() {
    return this.phase === 'Loading';
  }

  /**
   * One slot per station from shipStations.stations, classified by holder,
   * followed by spectator rows. Mirrors `LobbyView::station_slots`.
   *
   * Returns: array of
   *   { kind: 'available'|'occupied'|'mine', station, short_code, description,
   *     rank, holder_name? }
   *   | { kind: 'spectator', player_name }
   */
  stationSlots(myToken) {
    const defs = this.shipStations.stations || [];

    // Set of all station ids that appear in the current ship config.
    const stationIds = new Set(defs.map(d => d.id));

    const slots = [];
    for (const def of defs) {
      const holder = this.players.find(p => p.station === def.id);
      const base = {
        station: def.name || '',
        short_code: def.short_code || '',
        description: def.description || '',
        rank: def.rank || '',
      };
      if (holder && holder.token === myToken) {
        slots.push({ kind: 'mine', ...base });
      } else if (holder) {
        slots.push({ kind: 'occupied', ...base, holder_name: holder.name });
      } else {
        slots.push({ kind: 'available', ...base });
      }
    }

    // Spectator rows: players whose station id is not in any station definition.
    for (const p of this.players) {
      const held = p.station;
      if (!held || !stationIds.has(held)) {
        slots.push({ kind: 'spectator', player_name: p.name });
      }
    }

    return slots;
  }

  /**
   * True if every station in shipStations.stations is filled by a connected player.
   * Mirrors `LobbyView::all_stations_filled`.
   */
  allStationsFilled() {
    const defs = this.shipStations.stations || [];
    if (!defs.length) return false;
    const heldStationIds = new Set(this.players
      .filter(p => p.connected !== false)
      .map(p => p.station)
      .filter(Boolean));
    return defs.every(def => heldStationIds.has(def.id));
  }
}

/**
 * Decides which station id to land on after a StationAssigned update.
 * Returns `current` if it is still owned per the roster; otherwise the new
 * assigned station id if present, else null.
 *
 * The current signature (`current`, `newStationIds`) preserves the historical
 * shape from the pre-#619 PascalCase consoles list. When `newStationIds` is
 * empty (spectator assignment) returns null.
 */
export function reconcileActiveConsole(current, newStationIds) {
  if (!newStationIds || newStationIds.length === 0) return null;
  if (current && newStationIds.includes(current)) return current;
  return newStationIds[0];
}

/** Singleton used by client.html. */
export const lobbyState = new LobbyState();

if (typeof window !== 'undefined') {
  window.lobbyState = lobbyState;
  window.reconcileActiveConsole = reconcileActiveConsole;
  window.isLoadingPhase = () => lobbyState.isLoading();
  window.playerStationId = playerStationId;
}
