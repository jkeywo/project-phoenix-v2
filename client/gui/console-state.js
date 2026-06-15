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

function activeObjectiveTargetNames(objectives) {
  const names = new Set();
  for (const obj of (objectives || [])) {
    if (!obj || obj.status && obj.status !== 'Active') continue;
    for (const target of (obj.targets || [])) {
      if (target != null && String(target).trim() !== '') names.add(String(target));
    }
  }
  return names;
}

function entityMatchesObjectiveTarget(entity, targets) {
  if (!entity || !targets || targets.size === 0) return false;
  return [entity.name, entity.id, entity.uuid].some(v => v != null && targets.has(String(v)));
}

function withObjectiveTargets(entities, objectives) {
  const targets = activeObjectiveTargetNames(objectives);
  if (targets.size === 0) return entities || [];
  return (entities || []).map(e => {
    if (!entityMatchesObjectiveTarget(e, targets)) return e;
    return { ...e, objective_target: true };
  });
}

function hasAnyTag(entity, tags) {
  const wanted = new Set((tags || []).map(t => String(t).toLowerCase()));
  if (wanted.size === 0) return false;
  const actual = (entity.tags || entity.entity_tags || []).map(t => String(t).toLowerCase());
  return actual.some(t => wanted.has(t));
}

export function buildRadarRegions(entities, objectives = []) {
  const objectiveEntities = withObjectiveTargets(entities, objectives);
  return objectiveEntities
    .map(e => {
      if (!e || !hasAnyTag(e, ['region', 'asteroid_field', 'objective_marker'])) return null;
      const isAsteroidField = hasAnyTag(e, ['asteroid_field']);
      const shape = e.shape
        ? String(e.shape).toLowerCase()
        : (isAsteroidField ? ((e.inner_radius || 0) > 0 ? 'torus' : 'sphere') : null);
      if (!shape) return null;
      const radius = e.radius ?? e.outer_radius ?? null;
      return {
        uuid: e.uuid,
        x: entityX(e),
        z: entityZ(e),
        shape,
        radius,
        inner_radius: e.inner_radius ?? null,
        outer_radius: e.outer_radius ?? radius,
        half_extents: Array.isArray(e.half_extents)
          ? [e.half_extents[0] || 0, e.half_extents[2] || e.half_extents[1] || 0]
          : null,
        yaw: e.yaw ?? null,
        color: e.colour || e.color || (
          e.objective_target ? [0.83, 0.66, 0.13]
          : isAsteroidField ? [0.52, 0.32, 0.18]
          : [0.6, 0.4, 1.0]
        ),
        name: e.name || null,
        objective_target: !!e.objective_target,
      };
    })
    .filter(Boolean);
}

function projectRadarRegions(regions, shipX, shipZ, shipYaw, range, opts = {}) {
  const safeRange = Math.max(Number(range) || 0, 0.001);
  const rotate = opts.rotate !== false;
  const cosY = rotate ? Math.cos(shipYaw || 0) : 0;
  const sinY = rotate ? Math.sin(shipYaw || 0) : 0;
  return (regions || []).map(region => {
    const dx = region.x - shipX;
    const dz = region.z - shipZ;
    const radar_x = rotate ? (dx * cosY + dz * sinY) / safeRange : dx / safeRange;
    const radar_y = rotate ? (dx * sinY - dz * cosY) / safeRange : dz / safeRange;
    return {
      ...region,
      radar_x,
      radar_y,
      scaled_radius: region.radius != null ? region.radius / safeRange : null,
      scaled_inner_radius: region.inner_radius != null ? region.inner_radius / safeRange : null,
      scaled_outer_radius: region.outer_radius != null ? region.outer_radius / safeRange : null,
      scaled_half_extents: Array.isArray(region.half_extents)
        ? [region.half_extents[0] / safeRange, region.half_extents[1] / safeRange]
        : null,
    };
  });
}

// ── Radar range constants (exported for tests) ──────────────────────────────

export const WEAPONS_RADAR_RANGE = 300.0;
export const HELM_RADAR_RANGE    = 500.0;
export const SENSORS_RADAR_RANGE = 500.0;

const CAMERA_VIEWS = new Set(['Fore', 'Port', 'Starboard', 'Aft']);

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
 *                (weapons, helm, sensors)
 *        false → world-axis frame: rx = dx, ry = dz
 *                (navigation - world-north-up, Z-down screen convention)
 * @param {function} [opts.extra]
 *        Called as `extra(entity)` and merged into each blip object.
 *
 * @returns {Array} Blip objects: { uuid, radar_x, radar_y, scaled_radius, kind, ...extra }
 */
export function buildBlips(entities, shipX, shipZ, shipYaw, range, opts = {}) {
  const rotate = opts.rotate !== false;
  const cosY = rotate ? Math.cos(shipYaw) : 0;
  const sinY = rotate ? Math.sin(shipYaw) : 0;
  const shows = (opts.shows || []).map(t => String(t).toLowerCase());
  const selects = (opts.selects || []).map(t => String(t).toLowerCase());
  return (entities || []).map(a => {
    const tags = (a.tags || a.entity_tags || []).map(t => String(t).toLowerCase());
    if (shows.length > 0 && !tags.some(t => shows.includes(t)) && !a.objective_target) return null;

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
    const radius = (a.radar_world_size !== undefined && a.radar_world_size !== null)
      ? a.radar_world_size
      : entityRadius(a);
    const explicitKind = kindFromRadarIcon(a.radar_icon || a.radarIcon);
    const kind   = explicitKind
                 || (tags.includes('ship')    ? 'ship'
                 : tags.includes('station') ? 'station'
                 : tags.includes('planet')  ? 'planet'
                 : tags.includes('star')    ? 'star'
                 : tags.includes('torpedo') || tags.includes('missile') ? 'torpedo'
                 : tags.includes('region')  ? 'region'
                 : tags.includes('objective_marker') ? 'waypoint'
                 : 'asteroid');
    const targetTags = (a.target_tags || []).map(t => String(t).toLowerCase());
    const selectable = selects.length > 0 && targetTags.some(t => selects.includes(t));
    const blip = {
      uuid: a.uuid,
      radar_x,
      radar_y,
      scaled_radius: radius / range,
      kind,
      icon: a.radar_icon || kind,
      color: a.colour || a.color || null,
      objective_target: !!a.objective_target,
      name: a.name || null,
      selectable,
      threat_level: a.threat_level || null,
      description: a.target_description || a.description || a.name || null,
      target_tags: a.target_tags || [],
    };
    if (opts.extra) Object.assign(blip, opts.extra(a));
    return blip;
  }).filter(Boolean);
}

function kindFromRadarIcon(icon) {
  const value = icon === undefined || icon === null ? '' : String(icon).toLowerCase();
  if (value === 'player_ship') return 'player';
  if (value === 'missile') return 'torpedo';
  if ([
    'ship', 'player', 'asteroid', 'station', 'planet', 'star', 'torpedo',
    'battleship', 'cruiser', 'destroyer',
  ].includes(value)) return value;
  return null;
}

/**
 * Project the shared waypoint into a radar blip.
 *
 * @param {{ x:number, z:number }|null} waypoint
 * @param {number} shipX
 * @param {number} shipZ
 * @param {number} shipYaw
 * @param {number} range
 * @param {object} [opts]
 * @param {boolean} [opts.rotate=true]
 * @param {boolean} [opts.edgeClamp=false]
 * @returns {object|null}
 */
export function buildWaypointBlip(waypoint, shipX, shipZ, shipYaw, range, opts = {}) {
  if (!waypoint || !Number.isFinite(waypoint.x) || !Number.isFinite(waypoint.z)) return null;
  const safeRange = Math.max(Number(range) || 0, 0.001);
  const rotate = opts.rotate !== false;
  const dx = waypoint.x - shipX;
  const dz = waypoint.z - shipZ;
  let radar_x, radar_y;
  if (rotate) {
    const cosY = Math.cos(shipYaw || 0);
    const sinY = Math.sin(shipYaw || 0);
    radar_x = (dx * cosY + dz * sinY) / safeRange;
    radar_y = (dx * sinY - dz * cosY) / safeRange;
  } else {
    radar_x = dx / safeRange;
    radar_y = dz / safeRange;
  }

  const normalizedDistance = Math.hypot(radar_x, radar_y);
  const edge = opts.edgeClamp && normalizedDistance > 1;
  if (edge) {
    const scale = 0.96 / normalizedDistance;
    radar_x *= scale;
    radar_y *= scale;
  }

  return {
    uuid: 'navigation-waypoint',
    radar_x,
    radar_y,
    scaled_radius: 10 / safeRange,
    kind: 'waypoint',
    icon: 'waypoint',
    color: [0.45, 0.95, 1.0],
    name: 'WAYPOINT',
    selectable: false,
    objective_target: false,
    edge,
    world_x: waypoint.x,
    world_z: waypoint.z,
  };
}

// ── Console state builders ──────────────────────────────────────────────────

/**
 * Tactical / Weapons console.
 * @param {{ weaponsTarget, weaponsBanks, weaponsTubes, weaponsTorpedoCount,
 *           weaponsPhaserMode, asteroids, shipX, shipZ, shipYaw, complexity }} state
 */
export function buildWeaponsConsoleState(state) {
  const range = state.weaponsRadarRange ?? WEAPONS_RADAR_RANGE;
  const blips = Array.isArray(state.weaponsBlips)
    ? state.weaponsBlips
    : buildBlips(
      state.asteroids, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
      range,
      {
        rotate: true,
        shows: state.tacticalRadarShows || ['player', 'ship', 'asteroid', 'station', 'missile', 'torpedo', 'region'],
        selects: state.tacticalRadarSelects || ['ship', 'station', 'asteroid'],
      }
    );
  const targetUuid = state.weaponsTarget || null;
  const targetBlip = targetUuid ? blips.find(b => b.uuid === targetUuid) : null;
  const targetName = state.weaponsTargetName || (targetBlip && targetBlip.name) || null;
  return JSON.stringify({
    target_uuid:   targetUuid,
    target_name:   targetName,
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
  const viewDirection = CAMERA_VIEWS.has(state.currentView) ? state.currentView : '';
  return JSON.stringify({
    red_alert:          state.redAlert    || false,
    view_direction:     viewDirection,
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
  // Exclude objective_marker entities — objectives only show on the nav chart.
  const helmEntities = (state.asteroids || []).filter(e => {
    const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
    return !tags.includes('objective_marker');
  });
  const blips = buildBlips(
    helmEntities, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
    range, { rotate: true }
  );
  const waypoint = buildWaypointBlip(
    state.navigationWaypoint || null,
    state.shipX || 0,
    state.shipZ || 0,
    state.shipYaw || 0,
    range,
    { rotate: true, edgeClamp: true }
  );
  if (waypoint) blips.push(waypoint);
  return JSON.stringify({
    heading:                 (((state.shipYaw || 0) * 180 / Math.PI % 360) + 360) % 360,
    speed:                   state.forwardSpeed          || 0,
    x:                       state.shipX                 || 0,
    z:                       state.shipZ                 || 0,
    yaw:                     state.shipYaw               || 0,
    impulse_charge_progress: state.impulseChargeProgress || 0,
    on_screen:               state.currentView === 'Radar',
    blips,
    waypoint:                state.navigationWaypoint || null,
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
  const entities = state.asteroids;
  const blips = buildBlips(
    entities, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
    range,
    {
      rotate: true,
      shows: state.sensorsRadarShows || ['player', 'asteroid_field', 'ship', 'station', 'planet', 'star', 'region'],
      selects: state.sensorsRadarSelects || ['ship', 'station', 'planet'],
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

  if (state.sensorsTarget && entities) {
    const tgt = entities.find(a => a.uuid === state.sensorsTarget);
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
    on_screen:               state.currentView === 'SensorsRadar' || state.currentView === 'ScienceRadar',
    regions:                 state.regions || projectRadarRegions(
      buildRadarRegions(entities, []),
      state.shipX || 0,
      state.shipZ || 0,
      state.shipYaw || 0,
      range,
      { rotate: true }
    ),
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
    on_screen: state.currentView === 'Comms',
  });
}

// ── Navigation radar range ──────────────────────────────────────────────────

export const NAVIGATION_RADAR_RANGE = 5000.0;

// Tags that appear on strategic navigational entities shown in the nav chart.
// Individual asteroid rocks and NPC ships are excluded.
const NAV_CHART_TAGS = new Set([
  'region',
  'asteroid_field',
  'star',
  'planet',
  'station',
  'player_ship',
  'player',
  'objective_marker',
]);

/**
 * Navigation console state builder (issue #458).
 *
 * Produces a world-centred north-up radar snapshot filtered to strategic
 * navigational entities (stars, planets, stations, regions, asteroid fields, player ship).
 * Individual asteroids and NPC ship blips are excluded.
 *
 * @param {{ asteroids, shipX, shipZ, impulseChargeProgress, currentView }} state
 */
export function buildNavigationConsoleState(state) {
  const range = state.navChartRange ?? NAVIGATION_RADAR_RANGE;
  const navShows = state.navChartShows || Array.from(NAV_CHART_TAGS);
  const navShowsLower = navShows.map(s => String(s).toLowerCase());
  const navSelects = (state.navChartSelects || ['ship', 'station', 'planet', 'star', 'region'])
    .map(t => String(t).toLowerCase());
  const entities = withObjectiveTargets(state.asteroids, state.objectives);
  // Filter to navigational entities only.
  const navEntities = entities.filter(e => {
    const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
    if (tags.includes('objective_marker') && !e.objective_target) return false;
    return e.objective_target || tags.some(t => navShowsLower.includes(t));
  });

  const blips = buildBlips(
    navEntities,
    state.shipX || 0, state.shipZ || 0,
    0,                  // north-up: no ship-yaw rotation
    range,
    {
      rotate: false,    // world-axis frame
      shows: navShows,
      selects: navSelects,
      extra: (e) => {
        const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
        const explicitKind = kindFromRadarIcon(e.radar_icon || e.radarIcon);
        const kind = explicitKind && explicitKind !== 'asteroid' ? explicitKind
                   : tags.includes('star')    ? 'star'
                   : tags.includes('planet')  ? 'planet'
                   : tags.includes('station') ? 'station'
                   : tags.includes('region') || tags.includes('asteroid_field') ? 'region'
                   : tags.includes('objective_marker') ? 'waypoint'
                   : explicitKind || 'ship';
        return {
          name: e.name || null,
          kind,
          world_x: entityX(e),
          world_z: entityZ(e),
          stance:  e.stance  || 'neutral',
          faction: e.faction || null,
          selectable: navSelects.length === 0 || tags.some(t => navSelects.includes(t)),
        };
      },
    }
  );
  const waypoint = buildWaypointBlip(
    state.navigationWaypoint || null,
    state.shipX || 0,
    state.shipZ || 0,
    0,
    range,
    { rotate: false, edgeClamp: true }
  );
  if (waypoint) blips.push(waypoint);

  const charge = state.impulseChargeProgress || 0;
  const onScreen = state.currentView === 'NavigationChart';

  return JSON.stringify({
    blips,
    waypoint:                state.navigationWaypoint || null,
    ship_x:                  state.shipX || 0,
    ship_z:                  state.shipZ || 0,
    ship_heading:            (((state.shipYaw || 0) * 180 / Math.PI % 360) + 360) % 360,
    ship_speed:              state.forwardSpeed || 0,
    impulse_charge_progress: charge,
    cancel_visible:          charge > 0,
    on_screen:               onScreen,
    radar_range:             range,
    regions:                 state.regions || buildRadarRegions(navEntities, state.objectives),
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
