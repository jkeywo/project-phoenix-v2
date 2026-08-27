/**
 * gui/sim-state.js — Pure JS port of src/client_sim.rs (ClientSimState +
 * radar configs + ClientMessage builders + view helpers). Issue #460.
 *
 * `apply(msg)` takes an already-parsed ServerMessage object of the form
 * `{ type: 'VariantName', data: {...} }` (serde tag/content wire format).
 *
 * Radar *projection* is deliberately NOT duplicated here — client-side blip
 * projection already lives in gui/console-state.js
 * (`buildBlips`, `buildWaypointBlip`, `buildTargetBlip`). This module only
 * carries the per-console radar configs
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

  reset({ preserveAuthorityProjection = false } = {}) {
    // An InProgress Welcome is a reconnect handshake. Its targeted SimState
    // resync follows shortly afterwards, so retain the last authoritative
    // projection instead of briefly falling back to the lobby rating.
    const controlSources = preserveAuthorityProjection ? this.controlSources : {};
    const stationHosts = preserveAuthorityProjection ? this.stationHosts : {};
    const stationHealth = preserveAuthorityProjection ? this.stationHealth : {};
    const stationImportance = preserveAuthorityProjection ? this.stationImportance : {};
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
    /**
     * Share of the ship's total hull capacity (0.0–1.0) held by systems the
     * host reports as destroyed (issue #1014). Like `hullAggregate` this spans
     * every damageable system, including ones `consoleHull` never shows this
     * client, so it can never be re-derived locally. `null` until the first
     * `SystemHullUpdate` carrying it arrives.
     */
    this.hullDestroyed = null;
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
    /** Authoritative System id → Console Family projection (issue #1251),
     *  populated from Welcome ship_config.system_console_families. The tracer
     *  carries Command and Dock; missing ids remain on the explicitly temporary
     *  client inference path until #1252 completes the descriptor migration. */
    this.systemConsoleFamilies = {};
    /** Whether a Welcome has supplied the topology projection boundary.
     *  `systemConsoleFamilies === {}` alone is ambiguous before Welcome; once
     *  this is true, an absent descriptor is authoritative and must not revive
     *  a station-name builder fallback. */
    this.hasSystemConsoleFamilyProjection = false;
    /** Anonymous eligibility projection (issue #1103): station → rating →
     *  assist-function ids that station forces manual. From Welcome
     *  ship_config.station_assist_gaps. Hull-derived config, never a profile. */
    this.stationAssistGaps = {};
    /** Authoritative complete human-seeking Station placements by Station id. */
    this.stationHosts = stationHosts;
    /**
     * Authoritative per-Station hull health by Station id (issue #1100).
     * Value is a fraction in [0,1], or `null` for the neutral "no-damage-model"
     * state — a Station that owns no damageable capacity. Published
     * station-level by the host, so the Hero Bar shows a Station's health from
     * this map rather than summing recipient-scoped damage rows.
     */
    this.stationHealth = stationHealth;
    /**
     * Authoritative per-Station importance by Station id (issue #1101).
     * Each value is `{ unread, critical }` — a one-off unread event (cleared
     * when the Station is visited) and a continuing critical condition (cleared
     * only when it resolves), held apart from health. Host-derived and rebuilt
     * from `snap.station_importance` every SimState, so a Station whose
     * importance has resolved simply drops out of the map.
     */
    this.stationImportance = stationImportance;
    /** Per-system control source ("Human" or "Ai"), populated from SimSnapshot. */
    this.controlSources = controlSources;
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
    /** RGBA 0-1 fill for the helm radar's red-alert hostile weapon-arc overlay
     *  (issue #874). Authored per hull in `[helm_console] hostile_arc_color`;
     *  the array below is THE client placeholder until Welcome arrives — the
     *  only one on the client (`ph-helm-radar` deliberately carries none), and
     *  kept equal to `default_hostile_arc_color()` in `src/core/messages.rs` so
     *  a hull that omits the key looks the same as one that authors it. */
    this.hostileArcColor = [1, 0.3, 0.3, 0.07];
    // ── Mirror-only UI fields formerly hand-maintained by client.html (#819) ──
    /** Sensors console target uuid. Set locally by set_sensors_target (the
     *  action-map mutate patch lands here), cleared when the entity despawns. */
    this.sensorsTarget = null;
    /** Authoritative server blips from the latest WeaponsUpdate, when sent. */
    this.weaponsBlips = [];
    /** Label of the currently-focused shield facing, derived from ShieldStatus. */
    this.shieldFocusedFacing = null;
    /**
     * Latest host rejection of a comms response (#761 AC3), or null.
     * `{ message_id, response_index, ts }` — the comms console surfaces it so
     * the attempted response button flashes red. `ts` distinguishes repeat
     * rejections so the flash re-triggers.
     */
    this.commsRejection = null;
    /**
     * Read-only ship manual replica (issue #772), or null until the host's
     * `ShipManual` message arrives. Shape mirrors `ShipManualWire`:
     * `{ stations: [{ station_id, overview, sections: [...] }] }`. Presentation
     * state ONLY — the client never mutates it or authors commands from it.
     */
    this.shipManual = null;
    /**
     * Host debug/session state as the server last reported it (issue #940),
     * or `null` before the first `DebugState` arrives.
     * `{ flags: { Regions: bool, ... }, paused: bool, godMode: bool }`.
     * Presentation state ONLY — the settings panel renders from it and never
     * writes it.
     *
     * `paused` and `godMode` sit beside `flags` rather than in it because the
     * host reports them separately: they are authoritative simulation state,
     * not diagnostic overlays, and each is reached by its own route.
     *
     * PRESERVED across `Welcome` resets, like `tutorialProgress`, for a reason
     * of its own: these are the HOST's flags, and they do not change because a
     * world loaded. Clearing them would also race — the host re-announces when
     * a peer identifies, and that broadcast and the peer's own `Welcome` flush
     * in the same frame, so a reset could wipe the answer that just arrived.
     */
    this.debugFlags = this.debugFlags || null;
    /**
     * Contextual tutorial overlay definitions per station (issue #916), from
     * Welcome `ship_config.station_tutorials` — TOML-authored `[[station.
     * tutorial]]` blocks carried verbatim. Evaluated per push by
     * `withTutorialOverlay` in gui/console-state.js.
     */
    this.stationTutorials = {};
    /** Current player hull identity from Welcome ship_config.hull_id. */
    this.hullId = null;
    /**
     * Client-LOCAL tutorial progress (issue #916): which overlays the player
     * has dismissed and which console actions they have used. Not server
     * state — hydrated from localStorage by gui/tutorial-state.js and
     * PRESERVED across Welcome resets (a reconnect must not replay dismissed
     * tips). One of the two fields `reset()` keeps; `debugFlags` above is the
     * other, for a different reason of its own.
     */
    this.tutorialProgress = this.tutorialProgress || { dismissed: {}, used: {} };
    /**
     * Client-LOCAL, PRIVATE accessibility profile (issue #1102): the player's
     * explicit presentation effects (text scale, contrast, motion) plus a
     * declared-but-inert per-function assistance schema. Not server state and
     * never sent to any peer — hydrated from localStorage by
     * gui/accessibility-profile.js and PRESERVED across Welcome resets, so an
     * explicit choice survives a reconnect. Kept OUT of every server-projected
     * field for the same privacy reason (AC5). The default shape is spelled out
     * here (rather than imported) to avoid a sim-state ↔ accessibility-profile
     * import cycle — the canonical builder is emptyAccessibilityProfile().
     */
    this.accessibilityProfile = this.accessibilityProfile || {
      presentation: { textScale: 'default', contrast: 'default', reducedMotion: 'default' },
      assistance: {},
    };
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
        // `destroyed_fraction` is the companion ship-wide scalar (issue #1014):
        // how much of the ship's capacity is gone rather than merely damaged.
        // Same rule as the aggregate — keep the last known value when a payload
        // omits it, so a legacy host never blanks the loss band.
        if (typeof d.destroyed_fraction === 'number') {
          this.hullDestroyed = d.destroyed_fraction;
        }
        break;
      case 'SimState': {
        const snap = d.snapshot || {};
        this.stationHosts = Object.fromEntries(
          (snap.station_hosts || []).filter(Boolean).map(entry => [entry.station, entry]),
        );
        // Authoritative per-Station health (issue #1100). `health` is omitted on
        // the wire for the neutral no-damage-model state, so a missing value
        // becomes an explicit `null` the Hero Bar renders as that neutral cue.
        this.stationHealth = Object.fromEntries(
          (snap.station_health || []).filter(Boolean)
            .map(entry => [entry.station, typeof entry.health === 'number' ? entry.health : null]),
        );
        // Authoritative per-Station importance (issue #1101), rebuilt wholesale
        // each tick: a Station absent from the list carries no importance, so a
        // resolved unread/critical clears authoritatively (never optimistically).
        this.stationImportance = Object.fromEntries(
          (snap.station_importance || []).filter(Boolean)
            .map(entry => [entry.station, { unread: !!entry.unread, critical: !!entry.critical }]),
        );
        this.navigationWaypoint = snap.navigation_waypoint || null;
        // The host publishes the LocalShip's effective fine-System authority.
        // Missing on older protocol-compatible hosts, where the console
        // builders retain their station-rating fallback.
        this.controlSources = snap.control_sources || {};
        // Update live positions/hull/shield of known entities IN PLACE — never append.
        for (const st of (snap.entity_states || [])) {
          const entity = this.world.entities.find(e => e.uuid === st.uuid);
          if (!entity) continue;
          if (st.position != null) entity.position = st.position;
          if (st.yaw != null) entity.yaw = st.yaw;
          if (st.hull_fraction != null) entity.hull_fraction = st.hull_fraction;
          if (st.shield_fraction != null) entity.shield_fraction = st.shield_fraction;
          // Per-facing shield detail + generator frequency (issue #927).
          // Server-side `EntityStateSnapshot.shields`/`.shield_freq` were
          // always absent before #927 (sim_state_broadcaster hardcoded
          // `shields: None` and had no shield_freq field at all), so these
          // were dead reads on this side too — buildSensorsConsoleState has
          // read `tgt.shields`/`tgt.shield_freq` since #473/#870 but nothing
          // ever set them. Same delta-compressed pattern as hull/shield
          // fraction above: only present on a tick where the server decided
          // something changed.
          if (st.shields != null) entity.shields = st.shields;
          if (st.shield_freq != null) entity.shield_freq = st.shield_freq;
        }
        break;
      }
      case 'WorldSetup':
        this.world = d.world || defaultWorld();
        break;
      case 'ReturnedToLobby':
      case 'GameStarted':
        // These are the real round boundaries. Welcome is instead the
        // reconnect handshake for a peer already in the current round.
        this.stationHosts = {};
        this.stationHealth = {};
        this.stationImportance = {};
        this.controlSources = {};
        break;
      case 'Welcome': {
        const world = (d.state && d.state.world) || null;
        // The server only sends Welcome during an active game to reconnecting
        // peers. Lobby/new-game resets still clear the previous projection.
        this.reset({ preserveAuthorityProjection: d.state?.phase === 'InProgress' });
        // Store ship_config radar ranges (data-driven from TOML via server).
        // Used by console-state.js builders; fall back to server defaults.
        const sc = d.ship_config || {};
        this.hullId = sc.hull_id || null;
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
        this.hostileArcColor   = sc.hostile_arc_color   ?? this.hostileArcColor;
        this.stationTutorials  = sc.station_tutorials   || {};
        this.stationRatings = d.station_ratings || {};
        this.stationSystems = sc.station_systems || {};
        this.systemConsoleFamilies = sc.system_console_families || {};
        // Welcome is the projection boundary even when the map is empty (or an
        // older compatible payload omitted the additive field). After this
        // point a missing Command descriptor must not be guessed by station id.
        this.hasSystemConsoleFamilyProjection = true;
        // Anonymous eligibility projection (issue #1103): per station → per
        // rating → the assist-functions that station forces manual. Hull-derived
        // config, never anyone's profile; the lobby glue runs the SAME rule as
        // the host from it. Absent on a legacy server → {} → everyone eligible.
        this.stationAssistGaps = sc.station_assist_gaps || {};
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
        // `y` defaults to 0 so a pre-#768 planar message (no `y` field) keeps
        // the torpedo on the play plane. The client does not dead-reckon
        // torpedo flight, so this stored launch position is the whole record.
        this.torpedoesInFlight.push({
          uuid: d.uuid, x: d.x, y: d.y != null ? d.y : 0, z: d.z, heading: d.heading, tube: d.tube,
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
      case 'CommsResponseRejected':
        // Transient feedback (#761 AC3): the submitting comms holder's attempt
        // was refused. Stamp a fresh timestamp so a repeat rejection for the
        // same button re-triggers the red flash.
        this.commsRejection = {
          message_id: d.message_id,
          response_index: d.response_index,
          ts: Date.now(),
        };
        break;
      case 'RatingChanged':
        if (d.station_id != null) {
          this.stationRatings[d.station_id] = d.rating_name || '';
        }
        break;
      case 'ShipManual':
        // Read-only presentation state (issue #772): store the replica as-is.
        // The manual panel renders from it; nothing here mutates it or emits
        // any command in response.
        this.shipManual = d.manual || null;
        break;
      case 'DebugState':
        // Authoritative read-back of the host's debug/session flags (#940).
        // `flags` is a list of `[DebugFlag, bool]` pairs in a fixed order,
        // folded into a plain object because every consumer asks about one
        // named flag. Replaced wholesale rather than merged: the host sends the
        // complete set every time, so merging could only preserve staleness.
        //
        // Presentation state only. The settings panel paints its Debug/Cheat
        // buttons from this instead of from its own click, which is what makes
        // a refused toggle (a demo build has no route) visible rather than
        // silently ignored.
        this.debugFlags = {
          flags: Object.fromEntries(
            (d.flags || []).filter((pair) => Array.isArray(pair) && pair.length === 2),
          ),
          // Separate wire fields, not entries in `flags`: pause and god mode are
          // authoritative simulation state reached by their own routes, and
          // neither is a `DebugFlag` any more. Reported in every build even
          // though a demo phone can drive neither — a read-back is not a route.
          paused: !!d.paused,
          godMode: !!d.god_mode,
        };
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

export function redAlertSetMessage(active) {
  return {
    type: 'ControlSystem',
    data: {
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: !!active } },
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

/** Sum of the three power allocation levels `[helm, weapons, shields]`. */
export function powerTotal(levels) {
  return levels[0] + levels[1] + levels[2];
}

const POWER_INDEX = { Helm: 0, Tactical: 1, Shields: 2 };

/**
 * True when the Power console may send IncreasePower for `console`:
 * total below 8, that console below 4.
 *
 * The `locked` argument went away with the server's brownout lock (issue #952):
 * a flat battery no longer freezes the controls, it holds groups down to their
 * authored floors and lets them back up as the reserve returns — so a low
 * battery is a reason the request will not TAKE, not a reason to refuse to send
 * it. The only refusals left are the budget and the per-group cap.
 */
export function canIncreasePower(levels, console) {
  if (powerTotal(levels) >= 8) return false;
  const idx = POWER_INDEX[console];
  return idx !== undefined && levels[idx] < 4;
}

/**
 * True when the Power console may send DecreasePower for `console`:
 * that console above 1. See {@link canIncreasePower} on the retired `locked`.
 */
export function canDecreasePower(levels, console) {
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
