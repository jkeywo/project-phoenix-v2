/**
 * gui/sim-state.js — Pure JS port of src/client_sim.rs (ClientSimState +
 * radar configs + ClientMessage builders + view helpers). Issue #460.
 *
 * `apply(msg)` takes an already-parsed ServerMessage object of the form
 * `{ type: 'VariantName', data: {...} }` (serde tag/content wire format).
 *
 * Radar *projection* is deliberately NOT duplicated here — client-side blip
 * projection already lives in gui/radar-math.js and gui/console-state.js
 * (`buildBlips`). This module only carries the per-console radar configs
 * (range + tag filter) that mirror the Rust `*_radar_config()` functions.
 *
 * DOM-free; exposed on `window` as `window.simState` (singleton).
 */

// ── Radar-range constants (mirror src/radar.rs) ─────────────────────────────

export const HELM_RADAR_RANGE = 250.0;
export const WEAPONS_RADAR_RANGE = 300.0;
export const SCIENCE_RADAR_RANGE = 500.0;
export const SYSTEM_CHART_RANGE = 500.0;

/** RadarConfig for the Helm console radar. */
export function helmRadarConfig() {
  return { range: HELM_RADAR_RANGE, shows: ['asteroid', 'star', 'planet', 'ship'] };
}

/** RadarConfig for the Weapons (Tactical) console radar. */
export function weaponsRadarConfig() {
  return { range: WEAPONS_RADAR_RANGE, shows: ['asteroid', 'ship'] };
}

/** RadarConfig for the Science/Sensors long-range radar. */
export function scienceRadarConfig() {
  return {
    range: SCIENCE_RADAR_RANGE,
    shows: ['asteroid', 'ship', 'asteroid_field', 'region', 'star', 'planet'],
  };
}

/** RadarConfig for the System Chart (navigational entities only). */
export function systemChartConfig() {
  return { range: SYSTEM_CHART_RANGE, shows: ['star', 'planet', 'asteroid_field', 'region'] };
}

// ── Modifier key helper ─────────────────────────────────────────────────────

/**
 * Stable string key for a `(ModifierSource, ModifierSlot)` pair.
 * `source` is the parsed wire value (string for unit variants like
 * "ImpulseDrive", or an object like {"Console":"Helm"}); `slot` is a string.
 */
export function modifierKey(source, slot) {
  return JSON.stringify(source) + '::' + String(slot);
}

function defaultWorld() {
  return { entities: [], scenario_title: '', scenario_description: '' };
}

// ── ClientSimState ──────────────────────────────────────────────────────────

/**
 * Subset of the simulation state the client UI needs. Reset to defaults on
 * Welcome (preserving the world snapshot when present) and refreshed by the
 * per-tick / per-event server messages. Mirrors `ClientSimState` in
 * src/client_sim.rs.
 */
export class ClientSimState {
  constructor() {
    this.reset();
  }

  reset() {
    /** Static world snapshot { entities: [EntitySnapshot], scenario_title, scenario_description } */
    this.world = defaultWorld();
    /** 'Auto' | 'Manual' */
    this.phaserMode = 'Auto';
    this.lastPhaserTarget = null;
    this.scienceTargetSuggestion = null;
    /** Latest ShieldFacingStatus list. */
    this.shieldFacings = [];
    this.torpedoCount = 10;
    /** In-flight torpedoes: { uuid, x, z, heading, tube } */
    this.torpedoesInFlight = [];
    /** Map of modifierKey(source, slot) → bonus. */
    this.modifiers = new Map();
    /** Current repair team slots ('Idle' or tagged-variant objects). */
    this.repairTeams = [];
    this.phaserFrequency = 0.5;
    this.frequencyHint = null;
    /**
     * Per-system hull detail this client is entitled to see.
     *
     * Post issue #737 this is a host-side *projection*, not the whole ship:
     * a station holder gets only its own systems, and Engineering additionally
     * gets core systems plus any system a repair team is on site at. Never sum
     * it to obtain a ship-wide figure — use `hullAggregate`.
     */
    this.consoleHull = [];
    /**
     * Authoritative ship-wide hull fraction (0.0–1.0) across every damageable
     * system, published by the host (issue #737). `null` until the first
     * `SystemHullUpdate` arrives.
     */
    this.hullAggregate = null;
    /** Per-bank blaster state from the latest WeaponsUpdate or blackboard. */
    this.blasterBanks = [];
    /** Per-bank phaser state from the latest WeaponsUpdate. */
    this.bankStates = [];
    /** Per-tube torpedo state from the latest WeaponsUpdate. */
    this.tubeStates = [];
    this.currentTargetUuid = null;
    /** Per-station active ratings, populated from Welcome / RatingChanged. */
    this.stationRatings = {};
    /** Station → system id list, populated from Welcome ship_config.station_systems.
     *  Used by aggregateStationHull to compute per-station damage from consoleHull. */
    this.stationSystems = {};
    /** Per-system control source ("Human" or "Ai"), populated from SimSnapshot. */
    this.controlSources = {};
    /** Per-system blackboard mirror, keyed by SystemId string.
     *  Each value is the inner `data` object of the `SystemBlackboard` variant
     *  (e.g. `this.blackboards['helm']` is a `HelmBlackboard`). */
    this.blackboards = {};
    this.currentTargetName = null;
    /** Shared waypoint set by the Navigation console, or null when clear. */
    this.navigationWaypoint = null;
    /** Latest mission objective snapshots. */
    this.objectives = [];
    /** Radar range from server ship_config, populated on Welcome. */
    this.weaponsRadarRange = 300.0;
    this.helmRadarRange = 500.0;
    this.sensorsRadarRange = 500.0;
    // Radar show/select tag lists are required from the server's ship_config
    // (see Welcome handler below) — no JS-side default. A console whose
    // ship_config omits these shows nothing until the TOML is authored.
    this.tacticalRadarShows = [];
    this.tacticalRadarSelects = [];
    this.sensorsRadarShows = [];
    this.sensorsRadarSelects = [];
    this.navChartRange = 500.0;
    this.navChartShows = [];
    this.navChartSelects = [];
    /** Fire-arc configs from server ship_config, populated on Welcome. */
    this.phaserArcConfigs = [];
    this.torpedoArcConfigs = [];
    // ── Mirror-only UI fields formerly hand-maintained by client.html (#819) ──
    /** Sensors console target uuid. Set locally by set_sensors_target (the
     *  action-map mutate patch lands here), cleared when the entity despawns. */
    this.sensorsTarget = null;
    /** Authoritative server blips from the latest WeaponsUpdate, when sent. */
    this.weaponsBlips = [];
    /** Label of the currently-focused shield facing, derived from ShieldStatus. */
    this.shieldFocusedFacing = null;
  }

  /**
   * Apply a single inbound ServerMessage `{ type, data }`.
   * Mirrors `ClientSimState::apply`.
   */
  apply(msg) {
    if (!msg || !msg.type) return;
    const d = msg.data || {};
    switch (msg.type) {
      case 'SystemHullUpdate':
        // Post issue #618: publisher no longer emits legacy Console-keyed
        // `ConsoleHullUpdate`. Entries carry `{ system_id, display_name,
        // current, max_hp, tier, debuff_magnitude }` — see SystemHullStatus
        // in src/core/messages.rs.
        // `entries` is the recipient's projection (issue #737);
        // `aggregate_fraction` is the ship-wide figure that replaces summing it.
        this.consoleHull = d.entries || [];
        if (typeof d.aggregate_fraction === 'number') {
          this.hullAggregate = d.aggregate_fraction;
        }
        break;
      case 'SimState': {
        const snap = d.snapshot || {};
        this.navigationWaypoint = snap.navigation_waypoint || null;
        this.controlSources = snap.control_sources || {};
        // Update live positions/hull/shield of known entities IN PLACE — never append.
        for (const st of (snap.entity_states || [])) {
          const entity = this.world.entities.find(e => e.uuid === st.uuid);
          if (!entity) continue;
          if (st.position != null) entity.position = st.position;
          if (st.yaw != null) entity.yaw = st.yaw;
          if (st.hull_fraction != null) entity.hull_fraction = st.hull_fraction;
          if (st.shield_fraction != null) entity.shield_fraction = st.shield_fraction;
        }
        break;
      }
      case 'WorldSetup':
        this.world = d.world || defaultWorld();
        break;
      case 'Welcome': {
        const world = (d.state && d.state.world) || null;
        this.reset();
        // Store ship_config radar ranges (data-driven from TOML via server).
        // Used by console-state.js builders; fall back to server defaults.
        const sc = d.ship_config || {};
        this.weaponsRadarRange = sc.tactical_radar_range ?? 300.0;
        this.helmRadarRange    = sc.helm_radar_range    ?? 500.0;
        this.sensorsRadarRange = sc.sensors_radar_range ?? 500.0;
        this.tacticalRadarShows   = sc.tactical_radar_shows   || [];
        this.tacticalRadarSelects = sc.tactical_radar_selects || [];
        this.sensorsRadarShows    = sc.sensors_radar_shows    || [];
        this.sensorsRadarSelects  = sc.sensors_radar_selects  || [];
        this.navChartRange        = sc.nav_chart_range        ?? this.navChartRange;
        this.navChartShows        = sc.nav_chart_shows        || [];
        this.navChartSelects      = sc.nav_chart_selects      || [];
        this.phaserArcConfigs  = sc.phaser_banks        ?? [];
        this.torpedoArcConfigs = sc.torpedo_tubes       ?? [];
        this.stationRatings = d.station_ratings || {};
        this.stationSystems = sc.station_systems || {};
        if (world) {
          // Reset to defaults but preserve the world snapshot from Welcome.
          this.world = world;
          // Pre-seed repair teams with Idle slots so the Repair panel can
          // render rows immediately, before the first RepairState broadcast.
          // Only when entering a game (world present), not during lobby.
          const count = (sc.repair_team_count) || 0;
          if (count > 0) this.repairTeams = new Array(count).fill('Idle');
        }
        break;
      }
      case 'RepairState':
        this.repairTeams = d.teams || [];
        break;
      case 'PhaserFired':
        this.lastPhaserTarget = d.target_uuid != null ? d.target_uuid : null;
        break;
      case 'WeaponsUpdate': {
        const previousTargetUuid = this.currentTargetUuid;
        this.currentTargetUuid = d.target_uuid != null ? d.target_uuid : null;
        this.currentTargetName = d.target_uuid != null
          ? (d.target_name || (previousTargetUuid === d.target_uuid ? this.currentTargetName : null))
          : null;
        this.bankStates = d.banks || [];
        this.tubeStates = d.tubes || [];
        this.torpedoCount = typeof d.torpedo_count === 'number' ? d.torpedo_count : 0;
        this.phaserMode = d.phaser_mode || 'Auto';
        if (typeof d.phaser_frequency === 'number') this.phaserFrequency = d.phaser_frequency;
        if (d.blasters != null) this.blasterBanks = d.blasters;
        if (Array.isArray(d.blips)) this.weaponsBlips = d.blips;
        break;
      }
      case 'TargetLock':
        // Immediate lock feedback ahead of the next WeaponsUpdate (#819).
        if (d.locked) {
          this.currentTargetUuid = d.uuid != null ? d.uuid : null;
        } else {
          this.currentTargetUuid = null;
          this.currentTargetName = null;
        }
        break;
      case 'ShieldStatus': {
        this.shieldFacings = d.facings || [];
        const focused = this.shieldFacings.find(f => f.is_focused);
        this.shieldFocusedFacing = focused ? focused.label : null;
        break;
      }
      case 'TorpedoLaunched':
        this.torpedoesInFlight.push({
          uuid: d.uuid, x: d.x, z: d.z, heading: d.heading, tube: d.tube,
        });
        break;
      case 'TorpedoDestroyed':
        this.torpedoesInFlight = this.torpedoesInFlight.filter(t => t.uuid !== d.uuid);
        break;
      case 'ModifierAdded':
        this.modifiers.set(modifierKey(d.source, d.slot), d.bonus);
        break;
      case 'ModifierRemoved':
        this.modifiers.delete(modifierKey(d.source, d.slot));
        break;
      case 'EntitySpawned': {
        const snap = d.snapshot;
        if (snap && !this.world.entities.some(e => e.uuid === snap.uuid)) {
          this.world.entities.push(snap);
        }
        break;
      }
      case 'EntityDespawned':
        this.removeEntity(d.uuid);
        this._clearTargetsFor(d.uuid);
        break;
      case 'AsteroidSpawned':
        if (!this.world.entities.some(e => e.uuid === d.uuid)) {
          this.world.entities.push({
            uuid: d.uuid,
            position: [d.x, d.y, d.z],
            tags: ['asteroid'],
            radius: d.radius,
            radar_icon: 'asteroid',
          });
        }
        break;
      case 'AsteroidDestroyed':
        this.removeEntity(d.uuid);
        this._clearTargetsFor(d.uuid);
        break;
      case 'CoordinationPopup':
        this.coordinationPopup = { target: d.target, payload: d.payload, senderLabel: d.sender_label, ts: Date.now() };
        // Populate frequencyHint from FrequencyHint coordination payloads.
        if (d.payload && d.payload.type === 'FrequencyHint' && d.payload.data && typeof d.payload.data.frequency === 'number') {
          this.frequencyHint = d.payload.data.frequency;
        }
        break;
      case 'ObjectiveSummary':
        this.objectives = d.objectives || [];
        break;
      case 'CommsState':
        this.objectives = d.objectives || [];
        break;
      case 'RatingChanged':
        if (d.station_id != null) {
          this.stationRatings[d.station_id] = d.rating_name || '';
        }
        break;
      case 'BlackboardUpdate':
        for (const [systemId, bb] of (d.updates || [])) {
          // bb is { kind: "Helm", data: { yaw, forward_speed, ... } }
          if (bb && bb.kind && bb.data) {
            this.blackboards[systemId] = bb.data;
            // The navigation blackboard is the freshest source for the shared
            // waypoint (SimState only carries it at 10 Hz) — mirror it, as
            // client.html's deleted BlackboardUpdate handler used to (#819).
            if (systemId === 'navigation') {
              this.navigationWaypoint = bb.data.navigation_waypoint || null;
            }
          }
        }
        break;
      default:
        break;
    }
  }

  /**
   * Remove an entity by uuid IN PLACE so external references to
   * `world.entities` (e.g. via the `asteroids` getter) stay live.
   */
  removeEntity(uuid) {
    const entities = this.world.entities;
    for (let i = entities.length - 1; i >= 0; i--) {
      if (entities[i].uuid === uuid) entities.splice(i, 1);
    }
  }

  /** Bonus value for `(source, slot)`, or null if absent. */
  modifierBonus(source, slot) {
    const v = this.modifiers.get(modifierKey(source, slot));
    return v === undefined ? null : v;
  }

  /**
   * Clear any weapons / sensors target pointing at a despawned entity, so
   * the consoles never render a lock on something that no longer exists.
   * Formerly done by client.html's EntityDespawned/AsteroidDestroyed mirror.
   */
  _clearTargetsFor(uuid) {
    if (uuid == null) return;
    if (this.currentTargetUuid === uuid) {
      this.currentTargetUuid = null;
      this.currentTargetName = null;
    }
    if (this.sensorsTarget === uuid) this.sensorsTarget = null;
  }

  // ── Console-state view aliases (#819) ─────────────────────────────────────
  // gui/console-state.js builders read these key names; with client.html's
  // hand-maintained mirror deleted, the aliases live here so the builders
  // can take a ClientSimState directly without renaming their inputs.

  /** The live entity array (the builders' historical `state.asteroids`). */
  get asteroids() { return this.world.entities; }

  /** Ship world X from the helm blackboard (0 until the first update). */
  get shipX() { return this.blackboards['helm']?.x ?? 0; }
  /** Ship world Z from the helm blackboard. */
  get shipZ() { return this.blackboards['helm']?.z ?? 0; }
  /** Ship yaw (radians) from the helm blackboard. */
  get shipYaw() { return this.blackboards['helm']?.yaw ?? 0; }
  /** Forward speed from the helm blackboard. */
  get forwardSpeed() { return this.blackboards['helm']?.forward_speed ?? 0; }
  /** Impulse charge progress (0..1) from the helm blackboard. */
  get impulseChargeProgress() { return this.blackboards['helm']?.impulse_charge ?? 0; }

  /**
   * Current viewscreen view derived from the captain blackboard's view_mode
   * (`{kind:'Camera',data:name}` → the camera name, `{kind:'Cinematic'}` →
   * 'cinematic', other kinds → the kind). 'Fore' until the first update —
   * the same default the deleted client.html mirror initialised with.
   */
  get currentView() {
    const vm = this.blackboards['captain']?.view_mode;
    const vd = vm && vm.kind === 'Camera' ? vm.data : null;
    const kind = vm && vm.kind === 'Cinematic' ? 'cinematic' : (vm && vm.kind);
    return vd || kind || 'Fore';
  }

  /** Red-alert flag from the captain blackboard. */
  get redAlert() { return !!this.blackboards['captain']?.red_alert; }

  /** Tactical target uuid alias. The setter exists so the action-map's
   *  optimistic `mutate({ weaponsTarget })` patch lands on the one store. */
  get weaponsTarget() { return this.currentTargetUuid; }
  set weaponsTarget(uuid) { this.currentTargetUuid = uuid != null ? uuid : null; }
  get weaponsTargetName() { return this.currentTargetName; }
  get weaponsBanks() { return this.bankStates; }
  get weaponsTubes() { return this.tubeStates; }
  get weaponsTorpedoCount() { return this.torpedoCount; }
  get weaponsPhaserMode() { return this.phaserMode; }

  /**
   * Comms inbox / contacts delegated to the comms store (gui/comms-state.js).
   * The comms data deliberately lives in its own module; these getters keep
   * `simState` as the single object the console builders read from without
   * duplicating that state. Empty when the comms module isn't loaded.
   */
  get commsMessages() {
    const cs = typeof window !== 'undefined' ? window.commsState : undefined;
    return (cs && cs.messages) || [];
  }
  get commsContacts() {
    const cs = typeof window !== 'undefined' ? window.commsState : undefined;
    return (cs && cs.contacts) || [];
  }
}

// ── Outbound ClientMessage builders ─────────────────────────────────────────
// Each returns a plain `{ type, data? }` object matching the serde wire
// format; callers JSON.stringify before sending over PeerJS.

export function redAlertToggleMessage() {
  return {
    type: 'ControlSystem',
    data: {
      target: 'red-alert',
      payload: { type: 'ToggleRedAlert' },
    },
  };
}

export function firePhaserMessage(bankId) {
  return { type: 'FirePhaser', data: { bank: String(bankId) } };
}

export function fireTorpedoMessage(tube, targetUuid) {
  return { type: 'FireTorpedo', data: { tube, target_uuid: targetUuid != null ? targetUuid : null } };
}

export function setTargetMessage(uuid) {
  return {
    type: 'ControlSystem',
    data: {
      target: 'tactical-radar',
      payload: { type: 'SetTarget', data: { uuid } },
    },
  };
}

export function setScienceTargetMessage(uuid) {
  return {
    type: 'ControlSystem',
    data: {
      target: 'sensors',
      payload: { type: 'SetScienceTarget', data: { uuid } },
    },
  };
}

/** Alias for the sensors console: same wire message as a science target
 *  (the old short-form `SetSensorsTarget` rename is now applied here). */
export function setSensorsTargetMessage(uuid) {
  return setScienceTargetMessage(uuid);
}

export function setPhaserModeMessage(mode) {
  return {
    type: 'ControlSystem',
    data: {
      target: 'phaser-control',
      payload: { type: 'SetPhaserMode', data: { mode } },
    },
  };
}

/** Auto → Manual, Manual → Auto. */
export function togglePhaserModeMessage(current) {
  return setPhaserModeMessage(current === 'Auto' ? 'Manual' : 'Auto');
}

/** Frequency is clamped to [0, 1] before wrapping. */
export function setPhaserFrequencyMessage(frequency) {
  return {
    type: 'ControlSystem',
    data: {
      target: 'phaser-control',
      payload: {
        type: 'SetPhaserFrequency',
        data: { frequency: Math.min(1.0, Math.max(0.0, frequency)) },
      },
    },
  };
}

/**
 * Given a click `{x, y}` and entities `[{uuid, x, y}, ...]` in the same
 * coordinate space, return the uuid of the nearest entity (ties broken
 * first-wins), or null when empty. Port of `nearest_entity_to_point`.
 */
export function nearestEntityToPoint(click, entities) {
  let bestUuid = null;
  let bestD2 = Infinity;
  for (const e of (entities || [])) {
    const dx = e.x - click.x;
    const dy = e.y - click.y;
    const d2 = dx * dx + dy * dy;
    if (d2 < bestD2) {
      bestD2 = d2;
      bestUuid = e.uuid;
    }
  }
  return bestUuid;
}

// ── View helpers ────────────────────────────────────────────────────────────

/**
 * True when the Fire Phaser button for `bankId` should be enabled:
 * the bank reports fire_ready and is not on cooldown.
 */
export function isFireButtonEnabled(state, bankId) {
  const bank = state.bankStates.find(b => b.id === bankId);
  return !!(bank && bank.fire_ready && !bank.on_cooldown);
}

/** True when the torpedo tube with `tubeId` is loaded and ready. */
export function isTubeLoaded(state, tubeId) {
  const tube = state.tubeStates.find(t => t.id === tubeId);
  return !!(tube && tube.loaded);
}

/** Remaining reload seconds for `tubeId`, or 0 when unknown. */
export function tubeReloadSecs(state, tubeId) {
  const tube = state.tubeStates.find(t => t.id === tubeId);
  return tube ? tube.reload_secs : 0.0;
}

/** Human-readable label for the phaser-mode toggle button. */
export function phaserModeLabel(mode) {
  return mode === 'Manual' ? 'MANUAL' : 'AUTO';
}

/**
 * Compute shield arc views for the shield status diagram. Each facing is an
 * equal pie slice; facing 0 is centred on forward (top), clockwise.
 * Port of `shield_status_view`. Angles in radians, clockwise from up.
 */
export function shieldStatusView(facings) {
  const n = (facings || []).length;
  if (n === 0) return [];
  const TAU = Math.PI * 2;
  const arc = TAU / n;
  const halfArc = arc / 2;
  return facings.map((f, i) => {
    const centreAngle = i * arc;
    let fillFraction = 0.0;
    if (f.online && f.max_hp > 0) {
      fillFraction = Math.min(1.0, Math.max(0.0, f.hp / f.max_hp));
    }
    return {
      label: f.label,
      hp: f.hp,
      max_hp: f.max_hp,
      online: !!f.online,
      fill_fraction: fillFraction,
      start_angle: centreAngle - halfArc,
      end_angle: centreAngle + halfArc,
    };
  });
}

/** Sum of the three power allocation levels `[helm, weapons, sensors]`. */
export function powerTotal(levels) {
  return levels[0] + levels[1] + levels[2];
}

const POWER_INDEX = { Helm: 0, Tactical: 1, Sensors: 2 };

/**
 * True when the Power console may send IncreasePower for `console`:
 * not locked, total below 8, that console below 4.
 */
export function canIncreasePower(levels, console, locked) {
  if (locked || powerTotal(levels) >= 8) return false;
  const idx = POWER_INDEX[console];
  return idx !== undefined && levels[idx] < 4;
}

/**
 * True when the Power console may send DecreasePower for `console`:
 * not locked, that console above 1.
 */
export function canDecreasePower(levels, console, locked) {
  if (locked) return false;
  const idx = POWER_INDEX[console];
  return idx !== undefined && levels[idx] > 1;
}

/**
 * True when the Science console phaser-frequency sub-panel should be visible
 * (Tactical currently at Low complexity). `complexity` is the per-console
 * preset map from lobby state.
 */
export function isSciencePhaserPanelVisible(complexity) {
  return (complexity && complexity.Tactical) === 'Low';
}

/** Singleton used by client.html. */
export const simState = new ClientSimState();

if (typeof window !== 'undefined') {
  window.simState = simState;
}
