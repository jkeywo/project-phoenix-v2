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

/** All consoles the lobby UI knows how to render, in display order. */
export const ALL_CONSOLES = Object.freeze([
  'CaptainChair', 'Helm', 'Tactical', 'Repair', 'Sensors',
  'Shields', 'Navigation', 'Power', 'Comms',
]);

/**
 * Normalise a player's console list.
 * Legacy wire payloads carried `consoles` or `console`; current payloads carry
 * `station`, and consoles are derived from shipStations.stations.
 */
export function consolesOf(player, shipStations) {
  if (Array.isArray(player.consoles)) return player.consoles;
  if (player.console) return [player.console];
  const stationId = typeof player.station === 'string'
    ? player.station
    : (player.station && player.station.id) || null;
  if (!stationId) return [];
  const station = ((shipStations && shipStations.stations) || [])
    .find(s => s.id === stationId || s.name === stationId);
  return station && Array.isArray(station.consoles) ? station.consoles : [];
}

function normalisePlayer(p, shipStations) {
  return { ready: false, ...p, consoles: consolesOf(p, shipStations) };
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
    /** Array of { token, name, consoles: [...], connected } */
    this.players = [];
    /** ShipStations: { stations: [StationDef] } */
    this.shipStations = defaultShipStations();
    /** ShipClientConfig — per-ship static config from Welcome. */
    this.shipConfig = {};
    /** Reason string for game over, if the game has ended. */
    this.gameOverReason = null;
    this.scenarioTitle = '';
    this.scenarioBody = '';
  }

  /**
   * Replace the entire lobby state — used on Welcome, the authoritative
   * initial sync. Mirrors `LobbyState::replace_from`.
   */
  replaceFrom(state, shipStations, shipConfig) {
    this.phase = state.phase || 'Lobby';
    this.shipStations = shipStations || defaultShipStations();
    this.players = (state.players || []).map(p => normalisePlayer(p, this.shipStations));
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
        break;
      case 'PlayerJoined': {
        const player = normalisePlayer(d.player || {}, this.shipStations);
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
        // The same console can only be held by one player — first clear any
        // other player who used to hold any of these consoles, then assign
        // to the named token.
        const consoles = d.consoles || [];
        for (const c of consoles) {
          for (const p of this.players) {
            if (p.token !== d.token) {
              p.consoles = consolesOf(p).filter(existing => existing !== c);
            }
          }
        }
        const target = this.players.find(p => p.token === d.token);
        if (target) {
          target.consoles = consoles.slice();
          target.station = d.station_id || d.station || null;
        }
        break;
      }
      case 'GameStarted':
        this.phase = 'InProgress';
        break;
      case 'GameOver':
        this.phase = 'GameOver';
        this.gameOverReason = d.reason != null ? d.reason : '';
        break;
      default:
        // Not relevant to the lobby model.
        break;
    }
  }

  // ── LobbyView derivations (token-parameterised) ───────────────────────────

  /** Consoles held by the local player (empty if no matching token). */
  myConsoles(myToken) {
    const p = this.players.find(p => p.token === myToken);
    return p ? consolesOf(p) : [];
  }

  isCaptain(myToken)    { return this.myConsoles(myToken).includes('CaptainChair'); }
  isHelm(myToken)       { return this.myConsoles(myToken).includes('Helm'); }
  isSensors(myToken)    { return this.myConsoles(myToken).includes('Sensors'); }
  isShields(myToken)    { return this.myConsoles(myToken).includes('Shields'); }
  isNavigation(myToken) { return this.myConsoles(myToken).includes('Navigation'); }
  isRepair(myToken)     { return this.myConsoles(myToken).includes('Repair'); }
  isPower(myToken)      { return this.myConsoles(myToken).includes('Power'); }

  /** True if the local player is a spectator (no consoles assigned). */
  isSpectator(myToken) {
    return this.myConsoles(myToken).length === 0;
  }

  /** True when the lobby panel should be visible. */
  showLobbyPanel(myToken) {
    if (this.phase === 'Lobby') return true;
    if (this.phase === 'Loading') return true;
    if (this.phase === 'InProgress') return this.isSpectator(myToken);
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
   *     rank, consoles, holder_name? }
   *   | { kind: 'spectator', player_name }
   */
  stationSlots(myToken) {
    const defs = this.shipStations.stations || [];

    // Build the set of all consoles that appear in any station definition.
    const stationConsoles = new Set(defs.flatMap(d => d.consoles || []));

    const slots = [];
    for (const def of defs) {
      const defConsoles = def.consoles || [];
      const holder = this.players.find(p =>
        consolesOf(p).some(c => defConsoles.includes(c)));
      const base = {
        station: def.name || '',
        short_code: def.short_code || '',
        description: def.description || '',
        rank: def.rank || '',
        consoles: defConsoles,
      };
      if (holder && holder.token === myToken) {
        slots.push({ kind: 'mine', ...base });
      } else if (holder) {
        slots.push({ kind: 'occupied', ...base, holder_name: holder.name });
      } else {
        slots.push({ kind: 'available', ...base });
      }
    }

    // Spectator rows: players whose consoles are not in any station definition.
    for (const p of this.players) {
      const hasStation = consolesOf(p).some(c => stationConsoles.has(c));
      if (!hasStation) {
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
    const allHeld = this.players.flatMap(p => consolesOf(p));
    return defs.every(def => (def.consoles || []).some(c => allHeld.includes(c)));
  }
}

/**
 * Decides which console to land on after a StationAssigned update.
 * Returns `current` if present in `newConsoles`, else `newConsoles[0]`.
 * Returns null if `newConsoles` is empty (spectator assignment).
 * Port of `reconcile_active_console` (JS guard instead of Rust panic).
 */
export function reconcileActiveConsole(current, newConsoles) {
  if (!newConsoles || newConsoles.length === 0) return null;
  if (current && newConsoles.includes(current)) return current;
  return newConsoles[0];
}

/** Singleton used by client.html. */
export const lobbyState = new LobbyState();

if (typeof window !== 'undefined') {
  window.lobbyState = lobbyState;
  window.reconcileActiveConsole = reconcileActiveConsole;
  window.isLoadingPhase = () => lobbyState.isLoading();
}
