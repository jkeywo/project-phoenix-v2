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
 * Normalise a player's console list. Old wire payloads carried a single
 * `console` field; new ones carry a `consoles` array.
 */
export function consolesOf(player) {
  if (Array.isArray(player.consoles)) return player.consoles;
  return player.console ? [player.console] : [];
}

function normalisePlayer(p) {
  return { ready: false, ...p, consoles: consolesOf(p) };
}

function defaultShipStations() {
  return { configs: {}, min_players: 0, max_players: 0, complexity_presets: {} };
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
    /** ShipStations: { configs: {count: [StationDef]}, min_players, max_players, complexity_presets } */
    this.shipStations = defaultShipStations();
    /** ShipClientConfig — per-ship static config from Welcome. */
    this.shipConfig = {};
    /** Map of console name → current complexity preset name. */
    this.complexity = {};
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
    this.players = (state.players || []).map(normalisePlayer);
    this.shipStations = shipStations || defaultShipStations();
    this.shipConfig = shipConfig || {};
    this.complexity = state.complexity || {};
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
        if (target) target.consoles = consoles.slice();
        break;
      }
      case 'GameStarted':
        this.phase = 'InProgress';
        break;
      case 'ComplexityChanged':
        this.complexity[d.console] = d.preset_name;
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

  /** Current complexity preset for a console, or null. */
  complexityPresetFor(console) {
    return Object.prototype.hasOwnProperty.call(this.complexity, console)
      ? this.complexity[console]
      : null;
  }

  /** Station defs for a given player count (configs keys are strings on the wire). */
  stationDefsFor(count) {
    const cfgs = this.shipStations.configs || {};
    return cfgs[count] || cfgs[String(count)] || [];
  }

  /**
   * One slot per station at the current display player count, classified by
   * holder, followed by spectator rows. Mirrors `LobbyView::station_slots`.
   *
   * Returns: array of
   *   { kind: 'available'|'occupied'|'mine', station, short_code, description,
   *     rank, consoles, preset_names, holder_name? }
   *   | { kind: 'spectator', player_name }
   */
  stationSlots(myToken) {
    const playerCount = this.players.length;
    const max = this.shipStations.max_players || 0;
    const min = this.shipStations.min_players || 0;

    // Fixed roster per #495: always show the max_players layout.
    let displayCount = max > 0 ? max : Math.max(playerCount, min, 1);

    const slots = [];
    for (const def of this.stationDefsFor(displayCount)) {
      const defConsoles = def.consoles || [];
      const holder = this.players.find(p =>
        consolesOf(p).some(c => defConsoles.includes(c)));
      const presetNames = defConsoles.map(c => this.complexityPresetFor(c) || 'Std');
      const base = {
        station: def.name || '',
        short_code: def.short_code || '',
        description: def.description || '',
        rank: def.rank || '',
        consoles: defConsoles,
        preset_names: presetNames,
      };
      if (holder && holder.token === myToken) {
        slots.push({ kind: 'mine', ...base });
      } else if (holder) {
        slots.push({ kind: 'occupied', ...base, holder_name: holder.name });
      } else {
        slots.push({ kind: 'available', ...base });
      }
    }

    // Spectator rows only when connected players exceed max_players.
    if (max > 0 && playerCount > max) {
      for (const p of this.players) {
        if (consolesOf(p).length === 0) {
          slots.push({ kind: 'spectator', player_name: p.name });
        }
      }
    }

    return slots;
  }

  /**
   * True if every station slot at the display player count is filled.
   * Mirrors `LobbyView::all_stations_filled` + `stations_config::all_stations_filled`.
   */
  allStationsFilled() {
    const playerCount = this.players.length;
    const max = this.shipStations.max_players || 0;
    const checkCount = (max > 0 && playerCount > max) ? max : playerCount;

    const allHeld = this.players.flatMap(p => consolesOf(p));
    const defs = this.stationDefsFor(checkCount);
    if (!defs.length) return false;
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
