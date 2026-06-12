/**
 * gui/console-state.js — Pure state builder functions for HTML console iframes.
 *
 * Each build*(state) function accepts the sim-state object maintained by
 * client.html and returns a JSON string ready for __updateConsole(name, json).
 *
 * All functions are pure (no side effects, no DOM dependency) so they can be
 * unit-tested in Node via Vitest.
 *
 * Exposed on window as `window.buildConsoleState(consoleName, state)` for the
 * inline script in client.html (module scripts run after inline scripts; the
 * inline callers fall back to empty stubs until the module loads — in practice
 * events arrive well after module load so the fallback is never hit).
 */

// ── Entity position / radius helpers ───────────────────────────────────────

/**
 * World X from an entity snapshot.
 * Supports both flat `e.x` field and 3-element `e.position` array.
 * @param {{ x?: number, position?: number[] }} e
 */
export function entityX(e) {
  return e.x !== undefined ? e.x : (e.position ? e.position[0] : 0);
}

/**
 * World Z from an entity snapshot.
 * Supports both flat `e.z` field and 3-element `e.position` array.
 * @param {{ z?: number, position?: number[] }} e
 */
export function entityZ(e) {
  return e.z !== undefined ? e.z : (e.position ? e.position[2] : 0);
}

/**
 * Radar display radius from an entity snapshot.  Defaults to 4.
 * @param {{ radius?: number|null }} e
 */
export function entityRadius(e) {
  return (e.radius !== undefined && e.radius !== null) ? e.radius : 4;
}

// ── Radar range constants (exported for tests) ──────────────────────────────

export const WEAPONS_RADAR_RANGE = 300.0;
export const HELM_RADAR_RANGE    = 500.0;
export const SENSORS_RADAR_RANGE = 500.0;

// ── Shared radar blip builder ───────────────────────────────────────────────

/**
 * Build a filtered, projected array of radar blips.
 *
 * @param {Array}    entities  Raw entity snapshots (e.g. `state.asteroids`)
 * @param {number}   shipX     World X of the ship
 * @param {number}   shipZ     World Z of the ship
 * @param {number}   shipYaw   Ship heading in radians
 * @param {number}   range     Maximum radar range in world units
 * @param {object}   [opts]
 * @param {boolean}  [opts.rotate=true]
 *        true  → ship-local frame: rx = dx·cosY+dz·sinY, ry = dx·sinY−dz·cosY
 *                (weapons, helm)
 *        false → world-axis frame: rx = dx, ry = dz
 *                (sensors — world-north-up, Z-down screen convention)
 * @param {function} [opts.extra]
 *        Called as `extra(entity)` and merged into each blip object.
 *
 * @returns {Array} Blip objects: { uuid, radar_x, radar_y, scaled_radius, kind, ...extra }
 */
export function buildBlips(entities, shipX, shipZ, shipYaw, range, opts = {}) {
  const rotate = opts.rotate !== false;
  const cosY = rotate ? Math.cos(shipYaw) : 0;
  const sinY = rotate ? Math.sin(shipYaw) : 0;
  return (entities || []).map(a => {
    const ax = entityX(a), az = entityZ(a);
    const dx = ax - shipX, dz = az - shipZ;
    if (dx * dx + dz * dz > range * range) return null;
    let radar_x, radar_y;
    if (rotate) {
      radar_x = (dx * cosY + dz * sinY) / range;
      radar_y = (dx * sinY - dz * cosY) / range;
    } else {
      radar_x = dx / range;
      radar_y = dz / range;
    }
    const radius = entityRadius(a);
    const tags   = (a.tags || a.entity_tags || []).map(t => String(t).toLowerCase());
    const kind   = tags.includes('ship')    ? 'ship'
                 : tags.includes('station') ? 'station'
                 : 'asteroid';
    const blip = { uuid: a.uuid, radar_x, radar_y, scaled_radius: radius / range, kind };
    if (opts.extra) Object.assign(blip, opts.extra(a));
    return blip;
  }).filter(Boolean);
}

// ── Console state builders ──────────────────────────────────────────────────

/**
 * Tactical / Weapons console.
 * @param {{ weaponsTarget, weaponsBanks, weaponsTubes, weaponsTorpedoCount,
 *           weaponsPhaserMode, asteroids, shipX, shipZ, shipYaw, complexity }} state
 */
export function buildWeaponsConsoleState(state) {
  const range = state.weaponsRadarRange ?? WEAPONS_RADAR_RANGE;
  const blips = buildBlips(
    state.asteroids, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
    range, { rotate: true }
  );
  return JSON.stringify({
    target_uuid:   state.weaponsTarget      || null,
    banks:         state.weaponsBanks       || [],
    tubes:         state.weaponsTubes       || [],
    torpedo_count: state.weaponsTorpedoCount || 0,
    phaser_mode:   state.weaponsPhaserMode   || 'Auto',
    blips,
    phaser_arcs:   state.phaserArcConfigs  || [],
    torpedo_arcs:  state.torpedoArcConfigs || [],
    // Server complexity preset name (issue #461); drives [data-hideable]
    // element hiding via gui/hideable-elements.js in console-core.
    complexityPreset: state.complexity?.Tactical || 'Std',
  });
}

/**
 * CaptainChair console.
 * @param {{ redAlert, currentView, objectives, hullPct, blips }} state
 */
export function buildCaptainConsoleState(state) {
  return JSON.stringify({
    red_alert:          state.redAlert    || false,
    view_direction:     state.currentView || 'Fore',
    view_mode:          'Camera',
    objectives:         state.objectives  || [],
    hull_integrity_pct: state.hullPct     || 100,
    game_status:        state.redAlert
                          ? 'RED ALERT — All hands to battlestations.'
                          : 'Standing by. All systems nominal.',
    blips:              state.blips       || [],
  });
}

/**
 * Helm console.
 * @param {{ shipYaw, forwardSpeed, shipX, shipZ, impulseChargeProgress,
 *           currentView, weaponsTarget, asteroids }} state
 */
export function buildHelmConsoleState(state) {
  const range = state.helmRadarRange ?? HELM_RADAR_RANGE;
  const blips = buildBlips(
    state.asteroids, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
    range, { rotate: true }
  );
  return JSON.stringify({
    heading:                 (((state.shipYaw || 0) * 180 / Math.PI % 360) + 360) % 360,
    speed:                   state.forwardSpeed          || 0,
    x:                       state.shipX                 || 0,
    z:                       state.shipZ                 || 0,
    yaw:                     state.shipYaw               || 0,
    impulse_charge_progress: state.impulseChargeProgress || 0,
    on_screen:               state.currentView === 'Radar',
    blips,
  });
}

/**
 * Repair console.
 * @param {{ repairTeams, consoleHull }} state
 */
export function buildRepairConsoleState(state) {
  return JSON.stringify({
    teams:                state.repairTeams || [],
    console_hull:         state.consoleHull || [],
    travel_duration_secs: 5.0,
    damageable_consoles:  (state.consoleHull || []).map(h => h.console),
  });
}

/**
 * Power console.
 * @param {{ powerHelm, powerWeapons, powerSensors, powerBattery, powerLocked,
 *           complexity }} state
 */
export function buildPowerConsoleState(state) {
  return JSON.stringify({
    helm:           state.powerHelm    || 0,
    weapons:        state.powerWeapons || 0,
    sensors:        state.powerSensors || 0,
    battery_charge: state.powerBattery || 0,
    locked:         state.powerLocked  || false,
    // Server complexity preset name (issue #461); drives [data-hideable]
    // element hiding via gui/hideable-elements.js in console-core.
    complexityPreset: state.complexity?.Power || 'Std',
  });
}

/**
 * Shields console.
 * @param {{ weaponsTarget, asteroids, shipX, shipZ,
 *           shieldFacings, hullIntegrity, shieldFocusedFacing }} state
 */
export function buildShieldsConsoleState(state) {
  let targetBearing = null;
  if (state.weaponsTarget && state.asteroids) {
    const target = state.asteroids.find(a => a.uuid === state.weaponsTarget);
    if (target) {
      const dx = entityX(target) - (state.shipX || 0);
      const dz = entityZ(target) - (state.shipZ || 0);
      targetBearing = (Math.atan2(dx, -dz) * 180 / Math.PI + 360) % 360;
    }
  }
  return JSON.stringify({
    facings:            state.shieldFacings      || [],
    hull_integrity_pct: state.hullIntegrity       || 100,
    focused_facing:     state.shieldFocusedFacing || null,
    target_bearing:     targetBearing,
    grid_status:        (state.shieldFacings && state.shieldFacings.length > 0)
                          ? 'GRID NOMINAL' : 'GRID OFFLINE',
  });
}

/**
 * Sensors console.
 * @param {{ asteroids, shipX, shipZ, shipYaw, sensorsTarget, regions,
 *           complexity, impulseChargeProgress }} state
 */
export function buildSensorsConsoleState(state) {
  const range = state.sensorsRadarRange ?? SENSORS_RADAR_RANGE;
  const blips = buildBlips(
    state.asteroids, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
    range,
    {
      rotate: false,
      extra: (a) => ({
        color:   null,
        name:    a.name    || null,
        stance:  a.stance  || 'neutral',
        faction: a.faction || null,
      }),
    }
  );

  let targetBearing = null, targetRange = null;
  let targetName = null, targetKind = null, targetStance = null, targetFaction = null;
  let targetClass = null, targetHullPct = null, targetHeading = null, targetSpeed = null;
  let targetThreat = null, targetShieldFreq = null, targetShields = [];

  if (state.sensorsTarget && state.asteroids) {
    const tgt = state.asteroids.find(a => a.uuid === state.sensorsTarget);
    if (tgt) {
      const dx   = entityX(tgt) - (state.shipX || 0);
      const dz   = entityZ(tgt) - (state.shipZ || 0);
      targetBearing   = (Math.atan2(dx, -dz) * 180 / Math.PI + 360) % 360;
      targetRange     = Math.sqrt(dx * dx + dz * dz);
      targetName      = tgt.name      || state.sensorsTarget;
      const tags      = (tgt.tags || tgt.entity_tags || []).map(t => String(t).toLowerCase());
      targetKind      = tags.includes('ship')    ? 'ship'
                      : tags.includes('station') ? 'station' : 'asteroid';
      targetStance    = tgt.stance    || 'neutral';
      targetFaction   = tgt.faction   || null;
      targetClass     = tgt.shipClass || null;
      targetHullPct   = tgt.hull_pct  !== undefined ? tgt.hull_pct  : null;
      targetHeading   = tgt.heading   !== undefined ? tgt.heading   : null;
      targetSpeed     = tgt.speed     !== undefined ? tgt.speed     : null;
      targetThreat    = tgt.threat    || (targetStance === 'hostile' ? 'high' : 'low');
      targetShieldFreq = tgt.shield_freq || null;
      targetShields    = tgt.shields     || [];
    }
  }

  return JSON.stringify({
    scan_range:              range,
    complexity:              state.complexity?.Sensors || 'full',
    impulse_charge_progress: state.impulseChargeProgress || 0,
    regions:                 state.regions || [],
    blips,
    target_uuid:        state.sensorsTarget || null,
    target_name:        targetName,
    target_kind:        targetKind,
    target_stance:      targetStance,
    target_faction:     targetFaction,
    target_bearing:     targetBearing,
    target_range:       targetRange,
    target_class:       targetClass,
    target_hull_pct:    targetHullPct,
    target_heading:     targetHeading,
    target_speed:       targetSpeed,
    target_threat:      targetThreat,
    target_shield_freq: targetShieldFreq,
    target_shields:     targetShields,
  });
}

/**
 * Comms console.
 * @param {{ commsMessages, commsContacts }} state
 */
export function buildCommsConsoleState(state) {
  return JSON.stringify({
    messages: state.commsMessages || [],
    contacts: state.commsContacts || [],
  });
}

// ── Navigation radar range ──────────────────────────────────────────────────

export const NAVIGATION_RADAR_RANGE = 5000.0;

// Tags that appear on strategic navigational entities shown in the nav chart.
// Individual asteroid rocks and NPC ships are excluded.
const NAV_CHART_TAGS = new Set(['star', 'planet', 'station', 'player_ship', 'player']);

/**
 * Navigation console state builder (issue #458).
 *
 * Produces a world-centred north-up radar snapshot filtered to strategic
 * navigational entities (stars, planets, stations, player ship).
 * Individual asteroids and NPC ship blips are excluded.
 *
 * @param {{ asteroids, shipX, shipZ, impulseChargeProgress, currentView }} state
 */
export function buildNavigationConsoleState(state) {
  // Filter to navigational entities only.
  const navEntities = (state.asteroids || []).filter(e => {
    const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
    return tags.some(t => NAV_CHART_TAGS.has(t));
  });

  const blips = buildBlips(
    navEntities,
    state.shipX || 0, state.shipZ || 0,
    0,                  // north-up: no ship-yaw rotation
    NAVIGATION_RADAR_RANGE,
    {
      rotate: false,    // world-axis frame
      extra: (e) => {
        const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
        const kind = tags.includes('star')    ? 'star'
                   : tags.includes('planet')  ? 'planet'
                   : tags.includes('station') ? 'station'
                   : 'ship';
        return { name: e.name || null, kind };
      },
    }
  );

  const charge = state.impulseChargeProgress || 0;
  const onScreen = state.currentView === 'NavigationChart';

  return JSON.stringify({
    blips,
    ship_x:                  state.shipX || 0,
    ship_z:                  state.shipZ || 0,
    impulse_charge_progress: charge,
    cancel_visible:          charge > 0,
    on_screen:               onScreen,
    radar_range:             NAVIGATION_RADAR_RANGE,
  });
}

// ── Window dispatch (for non-module inline scripts in client.html) ──────────

if (typeof window !== 'undefined') {
  window.buildConsoleState = function buildConsoleState(consoleName, state) {
    switch (consoleName) {
      case 'Tactical':     return buildWeaponsConsoleState(state);
      case 'CaptainChair': return buildCaptainConsoleState(state);
      case 'Helm':         return buildHelmConsoleState(state);
      case 'Repair':       return buildRepairConsoleState(state);
      case 'Power':        return buildPowerConsoleState(state);
      case 'Shields':      return buildShieldsConsoleState(state);
      case 'Sensors':      return buildSensorsConsoleState(state);
      case 'Comms':        return buildCommsConsoleState(state);
      case 'Navigation':   return buildNavigationConsoleState(state);
      default:             return '{}';
    }
  };
}
