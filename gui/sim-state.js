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
    this.sensorsTargetSuggestion = null;
    /** Latest ShieldFacingStatus list. */
    this.shieldFacings = [];
    this.torpedoCount = 10;
    /** In-flight torpedoes: { uuid, x, z, heading, tube } */
    this.torpedoesInFlight = [];
    /** Map of modifierKey(source, slot) → bonus. */
    this.modifiers = new Map();
    /** Latest PowerState payload { helm, weapons, sensors, battery_charge, locked } or null. */
    this.powerStatePayload = null;
    /** Current repair team slots ('Idle' or tagged-variant objects). */
    this.repairTeams = [];
    this.phaserFrequency = 0.5;
    this.frequencyHint = null;
    /** Per-console hull integrity from the latest SimState. */
    this.consoleHull = [];
    /** Per-bank phaser state from the latest WeaponsUpdate. */
    this.bankStates = [];
    /** Per-tube torpedo state from the latest WeaponsUpdate. */
    this.tubeStates = [];
    this.currentTargetUuid = null;
  }

  /**
   * Apply a single inbound ServerMessage `{ type, data }`.
   * Mirrors `ClientSimState::apply`.
   */
  apply(msg) {
    if (!msg || !msg.type) return;
    const d = msg.data || {};
    switch (msg.type) {
      case 'SimState': {
        const snap = d.snapshot || {};
        this.consoleHull = snap.console_hull || [];
        // Update live positions/hull of known entities IN PLACE — never append.
        for (const st of (snap.entity_states || [])) {
          const entity = this.world.entities.find(e => e.uuid === st.uuid);
          if (!entity) continue;
          if (st.position != null) entity.position = st.position;
          if (st.hull_fraction != null) entity.hull_fraction = st.hull_fraction;
        }
        break;
      }
      case 'WorldSetup':
        this.world = d.world || defaultWorld();
        break;
      case 'Welcome': {
        const world = (d.state && d.state.world) || null;
        this.reset();
        if (world) {
          // Reset to defaults but preserve the world snapshot from Welcome.
          this.world = world;
          // Pre-seed repair teams with Idle slots so the Repair panel can
          // render rows immediately, before the first RepairState broadcast.
          // Only when entering a game (world present), not during lobby.
          const count = (d.ship_config && d.ship_config.repair_team_count) || 0;
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
      case 'WeaponsUpdate':
        this.currentTargetUuid = d.target_uuid != null ? d.target_uuid : null;
        this.bankStates = d.banks || [];
        this.tubeStates = d.tubes || [];
        this.torpedoCount = typeof d.torpedo_count === 'number' ? d.torpedo_count : 0;
        this.phaserMode = d.phaser_mode || 'Auto';
        break;
      case 'ScienceTargetSuggestion':
        this.scienceTargetSuggestion = d.uuid;
        break;
      case 'SensorsTargetSuggestion':
        this.sensorsTargetSuggestion = d.uuid;
        break;
      case 'ShieldStatus':
        this.shieldFacings = d.facings || [];
        break;
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
      case 'PowerState':
        this.powerStatePayload = {
          helm: d.helm,
          weapons: d.weapons,
          sensors: d.sensors,
          battery_charge: d.battery_charge,
          locked: !!d.locked,
        };
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
        break;
      case 'FrequencyHint':
        this.frequencyHint = d.frequency;
        break;
      default:
        break;
    }
  }

  /**
   * Remove an entity by uuid IN PLACE so external references to
   * `world.entities` (e.g. client.html's `state.asteroids`) stay live.
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
}

// ── Outbound ClientMessage builders ─────────────────────────────────────────
// Each returns a plain `{ type, data? }` object matching the serde wire
// format; callers JSON.stringify before sending over PeerJS.

export function redAlertToggleMessage() {
  return { type: 'ToggleRedAlert' };
}

export function firePhaserMessage(bankId) {
  return { type: 'FirePhaser', data: { bank: String(bankId) } };
}

export function fireTorpedoMessage(tube, targetUuid) {
  return { type: 'FireTorpedo', data: { tube, target_uuid: targetUuid != null ? targetUuid : null } };
}

export function dispatchRepairTeamMessage(teamIdx, console) {
  return { type: 'DispatchRepairTeam', data: { team_idx: teamIdx, console } };
}

export function setTargetMessage(uuid) {
  return { type: 'SetTarget', data: { uuid } };
}

export function setScienceTargetMessage(uuid) {
  return { type: 'SetScienceTarget', data: { uuid } };
}

export function setSensorsTargetMessage(uuid) {
  return { type: 'SetSensorsTarget', data: { uuid } };
}

export function setPhaserModeMessage(mode) {
  return { type: 'SetPhaserMode', data: { mode } };
}

/** Auto → Manual, Manual → Auto. */
export function togglePhaserModeMessage(current) {
  return setPhaserModeMessage(current === 'Auto' ? 'Manual' : 'Auto');
}

export function increasePowerMessage(console) {
  return { type: 'IncreasePower', data: { console } };
}

export function decreasePowerMessage(console) {
  return { type: 'DecreasePower', data: { console } };
}

/** Frequency is clamped to [0, 1] before wrapping. */
export function setPhaserFrequencyMessage(frequency) {
  return { type: 'SetPhaserFrequency', data: { frequency: Math.min(1.0, Math.max(0.0, frequency)) } };
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

/** Battery charge from the PowerState payload, or 0 when none received yet. */
export function batteryPercentage(payload) {
  return payload ? payload.battery_charge : 0.0;
}

/** True when the power system is locked (battery exhausted). */
export function isPowerLocked(payload) {
  return payload ? !!payload.locked : false;
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
