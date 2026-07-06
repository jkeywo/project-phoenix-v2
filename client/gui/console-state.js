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

function ownHull(stationId, state) {
  // Post issue #618: hull entries carry `.system_id` (lowercase, matches the
  // stable Rust `SystemId` newtype) rather than the legacy `.console`
  // PascalCase Console enum name. Callers pass the lowercase station id (see
  // buildConsoleState dispatch and each build*ConsoleState call).
  return (state.consoleHull || []).find(h => h.system_id === stationId) || null;
}

/**
 * Aggregate hull health across all systems that belong to a station.
 *
 * Destroyed or offline systems remain in the denominator (their `max_hp` still
 * counts; their `current` is 0) so that station damage can only increase as
 * systems are destroyed.
 *
 * @param {string}   stationId      - lowercase station id (e.g. 'helm')
 * @param {Array}    consoleHull    - full ship hull array from state.consoleHull
 * @param {object}   stationSystems - map of stationId → [systemId, ...] from state.stationSystems
 * @returns {{ entries: Array, totalCurrent: number, totalMax: number, pct: number, damagePct: number }}
 */
export function aggregateStationHull(stationId, consoleHull, stationSystems) {
  const systemIds = (stationSystems || {})[stationId] || [];
  const entries = (consoleHull || []).filter(h => systemIds.includes(h.system_id));
  const totalCurrent = entries.reduce((s, h) => s + h.current, 0);
  const totalMax = entries.reduce((s, h) => s + h.max_hp, 0);
  const pct = totalMax > 0 ? totalCurrent / totalMax : 1;
  return { entries, totalCurrent, totalMax, pct, damagePct: 1 - pct };
}

function withObjectiveTargets(entities, objectives) {
  const targets = activeObjectiveTargetNames(objectives);
  if (targets.size === 0) return entities || [];
  return (entities || []).map(e => {
    if (!entityMatchesObjectiveTarget(e, targets)) return e;
    return { ...e, objective_target: true };
  });
}

export function buildRadarRegions(entities, objectives = []) {
  const objectiveEntities = withObjectiveTargets(entities, objectives);
  return objectiveEntities
    .map(e => {
      // An entity is a region on radar iff [radar_appearance].region_colour
      // was authored for it. No tag-based detection, no colour fallback —
      // shape geometry still comes from the entity's own [shape]/
      // [asteroid_field] fields.
      if (!e || !e.region_colour) return null;
      const shape = e.shape
        ? String(e.shape).toLowerCase()
        : ((e.inner_radius || 0) > 0 ? 'torus' : 'sphere');
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
        color: e.region_colour,
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
    // No [radar_appearance].icon → this entity has no point blip (it may
    // still render as a region via buildRadarRegions, or not at all).
    if (!a || !a.radar_icon) return null;

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
    const radius = (a.radar_size !== undefined && a.radar_size !== null)
      ? a.radar_size
      : entityRadius(a);
    const targetTags = (a.target_tags && a.target_tags.length > 0 ? a.target_tags : (a.tags || [])).map(t => String(t).toLowerCase());
    const selectable = selects.length > 0 && targetTags.some(t => selects.includes(t));
    const blip = {
      uuid: a.uuid,
      radar_x,
      radar_y,
      scaled_radius: radius / range,
      kind: a.radar_icon,
      icon: a.radar_icon,
      color: a.colour || null,
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

  // When the waypoint is anchored to a parent entity, expose the parent
  // UUID so the navigation iframe can route a tap on the waypoint blip
  // back to the parent's selection. Anchored waypoints are selectable;
  // free waypoints remain non-selectable (no meaningful target).
  const sourceUuid =
    typeof waypoint.source_uuid === 'string' && waypoint.source_uuid.length > 0
      ? waypoint.source_uuid
      : null;

  return {
    uuid: 'navigation-waypoint',
    radar_x,
    radar_y,
    scaled_radius: 10 / safeRange,
    kind: 'waypoint',
    icon: 'waypoint',
    color: [0.83, 0.66, 0.13],
    name: 'WAYPOINT',
    selectable: sourceUuid !== null,
    objective_target: false,
    edge,
    world_x: waypoint.x,
    world_z: waypoint.z,
    source_uuid: sourceUuid,
  };
}

/**
 * Build a radar blip for a target entity (tactical or science) with off-screen
 * edge arrows, matching the same projection as buildWaypointBlip.
 *
 * @param {string|null}  targetUuid UUID of the target entity
 * @param {Array|null}   entities   Full entity list (state.asteroids / world.entities)
 * @param {number}       shipX
 * @param {number}       shipZ
 * @param {number}       shipYaw
 * @param {number}       range
 * @param {object}       [opts]
 * @param {boolean}      [opts.rotate=true]
 * @param {boolean}      [opts.edgeClamp=false]
 * @param {string}       [opts.kind='target-marker']  blip kind for the radar widget
 * @param {Array}        [opts.color=[1,1,1]]         RGB float array
 * @param {string}       [opts.label='TARGET']         display name
 * @returns {object|null}
 */
export function buildTargetBlip(targetUuid, entities, shipX, shipZ, shipYaw, range, opts = {}) {
  if (!targetUuid || !entities) return null;
  const target = entities.find(e => e.uuid === targetUuid);
  if (!target) return null;

  const safeRange = Math.max(Number(range) || 0, 0.001);
  const rotate = opts.rotate !== false;
  const dx = entityX(target) - shipX;
  const dz = entityZ(target) - shipZ;

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
    uuid: targetUuid,
    radar_x,
    radar_y,
    scaled_radius: 10 / safeRange,
    kind: opts.kind || 'target-marker',
    icon: opts.icon || 'target-marker',
    color: opts.color || [1.0, 1.0, 1.0],
    name: opts.label || target.name || 'TARGET',
    selectable: false,
    objective_target: false,
    edge,
    world_x: entityX(target),
    world_z: entityZ(target),
  };
}

// ── Console state builders ──────────────────────────────────────────────────

/**
 * Compute per-slot torpedo icon states for a single tube (issue #637).
 *
 * Returns an array of `vollMax` slot descriptors, each with:
 *   - `state`: 'filled' | 'queued-to-fill' | 'queued-to-empty' | 'empty'
 *   - `progress`: 0..1 fill fraction for the progress bar (non-zero only on the
 *                 "active" slot — the one currently being loaded or unloaded)
 *
 * Rules:
 *   slot i < loadedCount                       → filled (unless also i >= targetCount → queued-to-empty)
 *   slot i >= loadedCount && i < targetCount   → queued-to-fill
 *   slot i >= targetCount && i < loadedCount   → queued-to-empty
 *   otherwise                                  → empty
 *
 * Active slot (shows load_progress bar):
 *   loading   → slot index == loadedCount       (the slot being filled)
 *   unloading → slot index == loadedCount - 1   (the top-most loaded slot being drained)
 *
 * @param {{ state?: string, loaded?: boolean, loaded_count?: number,
 *            target_count?: number, volley_max?: number, load_progress?: number }} tube
 * @returns {{ state: string, progress: number }[]}
 */
export function torpSlotStates(tube) {
  const loadedCount = typeof tube.loaded_count  === 'number' ? tube.loaded_count  : (tube.loaded ? 1 : 0);
  const targetCount = typeof tube.target_count  === 'number' ? tube.target_count  : 0;
  const vollMax     = typeof tube.volley_max    === 'number' ? tube.volley_max    : 1;
  const loadProg    = typeof tube.load_progress === 'number' ? tube.load_progress : 0;
  const tubeState   = tube.state || (tube.loaded ? 'loaded' : 'unloaded');

  // Index of the slot currently transitioning (−1 = none).
  let activeIdx = -1;
  if (tubeState === 'loading')   activeIdx = loadedCount;
  if (tubeState === 'unloading') activeIdx = loadedCount - 1;

  const slots = [];
  for (let i = 0; i < vollMax; i++) {
    let slotState;
    if (i < loadedCount) {
      slotState = i >= targetCount ? 'queued-to-empty' : 'filled';
    } else {
      slotState = i < targetCount ? 'queued-to-fill' : 'empty';
    }

    let progress = 0;
    if (i === activeIdx && activeIdx >= 0) {
      progress = tubeState === 'loading' ? loadProg : (1 - loadProg);
    }

    slots.push({ state: slotState, progress });
  }
  return slots;
}

/**
 * Tactical / Weapons console.
 * Reads raw sim truth from `state.blackboards['tactical']` (WeaponsBlackboard),
 * falling back to legacy camelCase properties for compatibility.
 *
 * @param {{ blackboards?, weaponsTarget?, weaponsBanks?, weaponsTubes?,
 *           weaponsTorpedoCount?, weaponsPhaserMode?, blasterBanks? }} state
 */
export function buildWeaponsConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['tactical']) || {};
  const targetUuid   = bb.target_uuid   ?? state.weaponsTarget       ?? null;
  const targetName   = bb.target_name   ?? state.weaponsTargetName   ?? null;
  const banks        = bb.banks         ?? state.weaponsBanks        ?? [];
  const tubes        = bb.tubes         ?? state.weaponsTubes        ?? [];
  const torpedoCount = bb.torpedo_count ?? state.weaponsTorpedoCount ?? 0;
  const phaserMode   = bb.phaser_mode   ?? state.weaponsPhaserMode   ?? 'Auto';
  const regions      = bb.regions       ?? [];
  const phaserArcs   = bb.phaser_arcs   ?? state.phaserArcConfigs   ?? [];
  const torpedoArcs  = bb.torpedo_arcs  ?? state.torpedoArcConfigs  ?? [];
  const blasters     = bb.blasters      ?? state.blasterBanks        ?? [];

  const range = state.weaponsRadarRange ?? WEAPONS_RADAR_RANGE;
  const mappedPhaserArcs = phaserArcs.map(a => ({
    ...a,
    range_frac: a.beam_range != null ? a.beam_range / range : null,
  }));

  // Blips: authoritative server blips if provided, otherwise build from asteroids.
  let blips = bb.blips;
  if (!blips || blips.length === 0) {
    blips = state.weaponsBlips || [];
  }
  if (!blips || blips.length === 0) {
    blips = buildBlips(
      state.asteroids || [],
      state.shipX || 0,
      state.shipZ || 0,
      state.shipYaw || 0,
      range,
      { rotate: true }
    );
  }

  // Derive target_name from the locked server blip when no explicit name is stored.
  const resolvedTargetName = targetName || (targetUuid && blips.find(b => b.uuid === targetUuid)?.name) || null;

  // Add shared target markers (science target + navigation waypoint)
  const entities = state.asteroids || [];
  const sensBb = state.blackboards?.['sensors'];
  const sciTargetUuid = sensBb?.science_target_uuid || state.sensorsTarget || null;
  const sciMarker = buildTargetBlip(
    sciTargetUuid, entities, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0, range,
    { rotate: true, edgeClamp: true, kind: 'science-target', color: [0.2, 0.4, 1.0], label: 'SCIENCE TARGET' }
  );
  if (sciMarker) blips.push(sciMarker);
  const waypoint = buildWaypointBlip(
    state.navigationWaypoint || null, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0, range,
    { rotate: true, edgeClamp: true }
  );
  if (waypoint) blips.push(waypoint);

  return JSON.stringify({
    target_uuid:   targetUuid,
    target_name:   resolvedTargetName,
    banks,
    tubes,
    torpedo_count: torpedoCount,
    phaser_mode:   phaserMode,
    blips,
    regions,
    phaser_arcs:   mappedPhaserArcs,
    torpedo_arcs:  torpedoArcs,
    blasters,
    own_hull:      aggregateStationHull('tactical', state.consoleHull, state.stationSystems),
    tactical_auto: state.stationRatings?.['tactical'] === 'Backfill',
  });
}

/**
 * CaptainChair console.
 * @param {{ blackboards, redAlert, currentView, objectives, hullPct, blips }} state
 */
export function buildCaptainConsoleState(state) {
  const bb = state.blackboards && state.blackboards['captain'];
  if (bb) {
    return JSON.stringify({
      red_alert:             bb.red_alert             ?? false,
      red_alert_system_id:   bb.red_alert_system_id   ?? 'red-alert',
      red_alert_auto:        bb.red_alert_auto         ?? false,
      viewscreen_system_id:  bb.viewscreen_system_id  ?? 'viewscreen',
      viewscreen_auto:       bb.viewscreen_auto        ?? false,
      view_direction:        bb.view_direction         ?? '',
      view_mode:             'Camera',
      objectives:            bb.objectives             ?? [],
      hull_integrity_pct:    bb.hull_integrity_pct     ?? 100,
      game_status:           bb.game_status            ?? '',
      blips:                 state.blips               || [],
      own_hull:              aggregateStationHull('captain', state.consoleHull, state.stationSystems),
    });
  }
  // Legacy fallback.
  const controlSources = state.controlSources || {};
  const redAlertAuto = controlSources['red-alert'] === 'Ai';
  const viewscreenAuto = controlSources['viewscreen'] === 'Ai';
  const viewDirection = CAMERA_VIEWS.has(state.currentView) ? state.currentView : '';
  return JSON.stringify({
    red_alert:             state.redAlert    || false,
    red_alert_system_id:   'red-alert',
    red_alert_auto:        redAlertAuto,
    viewscreen_system_id:  'viewscreen',
    viewscreen_auto:       viewscreenAuto,
    view_direction:        viewDirection,
    view_mode:             'Camera',
    objectives:            state.objectives  || [],
    hull_integrity_pct:    state.hullPct     || 100,
    game_status:           state.redAlert
                             ? 'RED ALERT — All hands to battlestations.'
                             : 'Standing by. All systems nominal.',
    blips:                 state.blips       || [],
    own_hull:              aggregateStationHull('captain', state.consoleHull, state.stationSystems),
  });
}

/**
 * Helm console.
 *
 * Reads raw sim truth from the blackboard mirror (`state.blackboards['helm']`)
 * when available, falling back to legacy camelCase properties for compatibility.
 *
 * @param {{ blackboards?, shipYaw?, forwardSpeed?, shipX?, shipZ?,
 *           impulseChargeProgress?, currentView?, asteroids?,
 *           boostEnabled?, boostBattery?, boostActive? }} state
 */
export function buildHelmConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['helm']) || {};
  const shipYaw    = bb.yaw            ?? state.shipYaw            ?? 0;
  const shipX      = bb.x              ?? state.shipX              ?? 0;
  const shipZ      = bb.z              ?? state.shipZ              ?? 0;
  const forwardSpeed         = bb.forward_speed  ?? state.forwardSpeed         ?? 0;
  const impulseChargeProgress = bb.impulse_charge ?? state.impulseChargeProgress ?? 0;
  const boostEnabled = bb.boost_enabled ?? state.boostEnabled ?? false;
  const boostBattery = bb.boost_battery ?? state.boostBattery ?? 0;
  const boostActive  = bb.boost_active  ?? state.boostActive  ?? false;

  const range = state.helmRadarRange ?? HELM_RADAR_RANGE;
  // Exclude objective_marker entities — objectives only show on the nav chart.
  const helmEntities = (state.asteroids || []).filter(e => {
    const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
    return !tags.includes('objective_marker');
  });
  const blips = buildBlips(helmEntities, shipX, shipZ, shipYaw, range, { rotate: true });
  const waypoint = buildWaypointBlip(
    state.navigationWaypoint || null, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true }
  );
  if (waypoint) blips.push(waypoint);

  // Shared target markers (every radar shows tactical + science targets)
  const entities = state.asteroids || [];
  const tacBb = state.blackboards?.['tactical'];
  const sensBb = state.blackboards?.['sensors'];
  const tacMarker = buildTargetBlip(
    tacBb?.target_uuid, entities, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true, kind: 'tactical-target', color: [1.0, 0.2, 0.2], label: 'TACTICAL TARGET' }
  );
  if (tacMarker) blips.push(tacMarker);
  const sciTargetUuid = sensBb?.science_target_uuid || state.sensorsTarget || null;
  const sciMarker = buildTargetBlip(
    sciTargetUuid, entities, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true, kind: 'science-target', color: [0.2, 0.4, 1.0], label: 'SCIENCE TARGET' }
  );
  if (sciMarker) blips.push(sciMarker);

  return JSON.stringify({
    heading:                 (((shipYaw * 180 / Math.PI % 360) + 360) % 360),
    speed:                   forwardSpeed,
    x:                       shipX,
    z:                       shipZ,
    yaw:                     shipYaw,
    impulse_charge_progress: impulseChargeProgress,
    on_screen:               state.currentView === 'Radar',
    blips,
    waypoint:                state.navigationWaypoint || null,
    own_hull:                aggregateStationHull('helm', state.consoleHull, state.stationSystems),
    boost_enabled:           !!boostEnabled,
    boost_battery:           boostBattery,
    boost_active:            !!boostActive,
    helm_auto:               state.stationRatings?.['helm'] === 'Backfill',
    // Fine-system engine state (issue #511): read from per-system blackboards.
    engine_port_thrust:  (state.blackboards?.['helm-engine-port']?.thrust_fraction) ?? 0,
    engine_stbd_thrust:  (state.blackboards?.['helm-engine-starboard']?.thrust_fraction) ?? 0,
    engine_port_auto:    state.stationRatings?.['helm'] === 'Backfill',
    engine_stbd_auto:    state.stationRatings?.['helm'] === 'Backfill',
  });
}

/**
 * Repair console.
 * @param {{ blackboards, repairTeams, consoleHull }} state
 */
export function buildRepairConsoleState(state) {
  const bb = state.blackboards && state.blackboards['repair'];
  if (bb) {
    return JSON.stringify({
      teams:                bb.teams                ?? [],
      // SystemId-keyed fields (post issues #618/#619).
      system_hull:          bb.system_hull          ?? [],
      damageable_systems:   bb.damageable_systems   ?? [],
      travel_duration_secs: bb.travel_duration_secs ?? 5.0,
      repair_auto:          state.stationRatings?.['repair'] === 'Backfill',
    });
  }
  // Legacy fallback: derive damageable_systems from consoleHull (SystemId-keyed
  // after issue #618) so the repair panel renders even without the blackboard.
  return JSON.stringify({
    teams:                state.repairTeams || [],
    system_hull:          state.consoleHull || [],
    damageable_systems:   (state.consoleHull || []).map(h => h.system_id),
    travel_duration_secs: 5.0,
    repair_auto:          state.stationRatings?.['repair'] === 'Backfill',
  });
}

/**
 * Power console.
 *
 * Reads raw sim truth from `state.blackboards['power']` (aggregate
 * PowerBlackboard) plus the two fine blackboards (issue #513) at
 * `state.blackboards['power-reactor']` and `state.blackboards['power-battery']`
 * for per-instance online flags. Falls back to legacy camelCase properties
 * from PowerState messages when no blackboards are present.
 *
 * @param {{ blackboards?, powerHelm?, powerWeapons?, powerSensors?,
 *           powerBattery?, powerLocked? }} state
 */
export function buildPowerConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['power']) || null;
  const reactorBb = (state.blackboards && state.blackboards['power-reactor']) || null;
  const batteryBb = (state.blackboards && state.blackboards['power-battery']) || null;
  // Default to online when the fine blackboard is missing (legacy safety).
  const reactorOnline = reactorBb ? !!reactorBb.is_online : true;
  const batteryOnline = batteryBb ? !!batteryBb.is_online : true;
  if (bb) {
    return JSON.stringify({
      // Reads the PowerGroupId-keyed `groups` field from the publisher (the
      // legacy `consoles` mirror was removed from the wire when the parent
      // issue #516 cleanup closed out).
      consoles:       bb.groups        || [],
      total:          bb.total          ?? 0,
      total_max:      bb.total_max      ?? 8,
      battery_charge: bb.battery_charge ?? 0,
      battery_max:    bb.battery_max    ?? 100,
      locked:         bb.locked         || false,
      reactor_online: reactorOnline,
      battery_online: batteryOnline,
      own_hull:       aggregateStationHull('power', state.consoleHull, state.stationSystems),
      power_auto:     state.stationRatings?.['power'] === 'Backfill',
    });
  }
  // Legacy fallback: PowerState message fields.
  return JSON.stringify({
    helm:           state.powerHelm    || 0,
    weapons:        state.powerWeapons || 0,
    sensors:        state.powerSensors || 0,
    battery_charge: state.powerBattery || 0,
    locked:         state.powerLocked  || false,
    reactor_online: reactorOnline,
    battery_online: batteryOnline,
    own_hull:       aggregateStationHull('power', state.consoleHull, state.stationSystems),
    power_auto:     state.stationRatings?.['power'] === 'Backfill',
  });
}

/**
 * Shields console.
 * @param {{ blackboards, shieldFacings, hullIntegrity, shieldFocusedFacing }} state
 */
export function buildShieldsConsoleState(state) {
  const bb = state.blackboards && state.blackboards['shields'];
  if (bb) {
    return JSON.stringify({
      facings:            bb.facings            ?? [],
      hull_integrity_pct: bb.hull_integrity_pct ?? 100,
      focused_facing:     bb.focused_facing     ?? null,
      target_bearing:     bb.target_bearing     ?? null,
      grid_status:        bb.grid_status        ?? 'GRID NOMINAL',
      own_hull: aggregateStationHull('shields', state.consoleHull, state.stationSystems),
      shields_auto: state.stationRatings?.['shields'] === 'Backfill',
    });
  }
  // Legacy fallback: read from ShieldStatus broadcast fields.
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
    own_hull: aggregateStationHull('shields', state.consoleHull, state.stationSystems),
    shields_auto: state.stationRatings?.['shields'] === 'Backfill',
  });
}

/**
 * Sensors console.
 * @param {{ blackboards, asteroids, shipX, shipZ, shipYaw, sensorsTarget,
 *           regions, complexity, impulseChargeProgress }} state
 */
export function buildSensorsConsoleState(state) {
  const bb = state.blackboards && state.blackboards['sensors'];
  const range = bb ? (bb.radar_range ?? SENSORS_RADAR_RANGE)
                   : (state.sensorsRadarRange ?? SENSORS_RADAR_RANGE);
  const radarShows   = bb ? (bb.radar_shows   ?? state.sensorsRadarShows)
                          : state.sensorsRadarShows;
  const radarSelects = bb ? (bb.radar_selects ?? state.sensorsRadarSelects)
                          : state.sensorsRadarSelects;
  const entities = state.asteroids;
  const blips = buildBlips(
    entities, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0,
    range,
    {
      rotate: true,
      shows: radarShows,
      selects: radarSelects,
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
  let targetShieldFraction = null;

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
      targetHeading   = tgt.yaw != null
        ? (((tgt.yaw * 180 / Math.PI) % 360) + 360) % 360
        : null;
      targetSpeed     = tgt.speed     !== undefined ? tgt.speed     : null;
      targetThreat    = tgt.threat    || (targetStance === 'hostile' ? 'high' : 'low');
      targetShieldFreq = tgt.shield_freq || null;
      targetShields    = tgt.shields     || [];
      // Single-facing NPC shield fraction (#473). `null` for shieldless
      // entities (no [shields] block on the TOML); `0..=1` for shielded
      // NPCs; `0` for broken shields.
      targetShieldFraction = tgt.shield_fraction !== undefined && tgt.shield_fraction !== null
        ? tgt.shield_fraction
        : null;
    }
  }

  // Shared target markers (tactical target + navigation waypoint)
  const shipX = state.shipX || 0, shipZ = state.shipZ || 0, shipYaw = state.shipYaw || 0;
  const tacBb = state.blackboards?.['tactical'];
  const tacMarker = buildTargetBlip(
    tacBb?.target_uuid, entities, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true, kind: 'tactical-target', color: [1.0, 0.2, 0.2], label: 'TACTICAL TARGET' }
  );
  if (tacMarker) blips.push(tacMarker);
  const waypoint = buildWaypointBlip(
    state.navigationWaypoint || null, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true }
  );
  if (waypoint) blips.push(waypoint);

  return JSON.stringify({
    scan_range:              range,
    complexity:              state.complexity?.Sensors || 'full',
    impulse_charge_progress: state.impulseChargeProgress || 0,
    on_screen:               state.currentView === 'SensorsRadar' || state.currentView === 'ScienceRadar',
    regions:                 state.regions || projectRadarRegions(
      buildRadarRegions(entities, []),
      shipX,
      shipZ,
      shipYaw,
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
    target_shield_fraction: targetShieldFraction,
    own_hull: aggregateStationHull('sensors', state.consoleHull, state.stationSystems),
    sensors_auto: state.stationRatings?.['sensors'] === 'Backfill',
  });
}

/**
 * Comms console.
 * @param {{ blackboards, commsMessages, commsContacts }} state
 */
export function buildCommsConsoleState(state) {
  const bb = state.blackboards && state.blackboards['comms'];
  if (bb) {
    return JSON.stringify({
      messages:   bb.messages   ?? [],
      objectives: bb.objectives ?? [],
      contacts:   bb.contacts   ?? [],
      on_screen:  state.currentView === 'Comms',
      own_hull:   aggregateStationHull('comms', state.consoleHull, state.stationSystems),
      comms_auto: state.stationRatings?.['comms'] === 'Backfill',
    });
  }
  // Legacy fallback.
  return JSON.stringify({
    messages:  state.commsMessages || [],
    contacts:  state.commsContacts || [],
    on_screen: state.currentView === 'Comms',
    own_hull:  aggregateStationHull('comms', state.consoleHull, state.stationSystems),
    comms_auto: state.stationRatings?.['comms'] === 'Backfill',
  });
}

// ── Navigation radar range ──────────────────────────────────────────────────

export const NAVIGATION_RADAR_RANGE = 5000.0;

/**
 * Navigation console state builder (issue #458).
 *
 * Produces a world-centred north-up radar snapshot filtered to strategic
 * navigational entities, per the ship_config-authored `nav_chart_shows`/
 * `nav_chart_selects` lists. No JS-side default — an unauthored ship_config
 * shows nothing on the nav chart until the TOML specifies these lists.
 *
 * @param {{ blackboards, asteroids, shipX, shipZ, impulseChargeProgress,
 *           currentView }} state
 */
export function buildNavigationConsoleState(state) {
  const bb = state.blackboards && state.blackboards['navigation'];
  const range = bb ? (bb.nav_chart_range ?? NAVIGATION_RADAR_RANGE)
                   : (state.navChartRange ?? NAVIGATION_RADAR_RANGE);
  const navShows = bb ? (bb.nav_chart_shows ?? state.navChartShows ?? [])
                      : (state.navChartShows || []);
  const navShowsLower = navShows.map(s => String(s).toLowerCase());
  const navSelects = (bb ? (bb.nav_chart_selects ?? state.navChartSelects ?? [])
                         : (state.navChartSelects || [])).map(t => String(t).toLowerCase());
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
        return {
          name: e.name || null,
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
    own_hull: aggregateStationHull('navigation', state.consoleHull, state.stationSystems),
    ship_speed:              state.forwardSpeed || 0,
    impulse_charge_progress: charge,
    cancel_visible:          charge > 0,
    on_screen:               onScreen,
    radar_range:             range,
    regions:                 state.regions || buildRadarRegions(navEntities, state.objectives),
    navigation_auto:         state.stationRatings?.['navigation'] === 'Backfill',
  });
}

/**
 * Science console — combined Sensors + Shields view.
 *
 * Delegates to buildSensorsConsoleState and buildShieldsConsoleState,
 * then nests their payloads under `sensors` and `shields` keys so the
 * per-console iframe receives both panels' data without key collision.
 *
 * @param {{ blackboards?, asteroids?, shipX?, shipZ?, shipYaw?,
 *           sensorsTarget?, regions?, shieldFacings?, hullIntegrity?,
 *           shieldFocusedFacing? }} state
 */
export function buildScienceConsoleState(state) {
  const sensors = JSON.parse(buildSensorsConsoleState(state));
  const shields = JSON.parse(buildShieldsConsoleState(state));
  return JSON.stringify({
    sensors,
    shields,
    science_auto: state.stationRatings?.['science'] === 'Backfill',
  });
}

/**
 * Engineering console — combined Power + Repair view (Cruiser).
 *
 * Delegates to buildPowerConsoleState and buildRepairConsoleState,
 * nesting payloads under `power` and `repair` keys so the per-console
 * iframe receives both panels' data without key collision.
 *
 * @param {object} state
 */
export function buildEngineeringConsoleState(state) {
  const power = JSON.parse(buildPowerConsoleState(state));
  const repair = JSON.parse(buildRepairConsoleState(state));
  return JSON.stringify({
    power,
    repair,
    engineering_auto: state.stationRatings?.['engineering'] === 'Backfill',
  });
}

/**
 * Cruiser Comms console — combined Navigation + Comms view (Cruiser).
 *
 * Delegates to buildNavigationConsoleState and buildCommsConsoleState,
 * nesting payloads under `navigation` and `comms` keys so the per-console
 * iframe receives both panels' data without key collision.
 *
 * Used when the state has a navigation blackboard present (cruiser comms
 * station merges navigation + comms). The `comms_auto` field reflects
 * the comms station rating, consistent with the existing comms dispatch.
 *
 * @param {object} state
 */
export function buildCruiserCommsConsoleState(state) {
  const navigation = JSON.parse(buildNavigationConsoleState(state));
  const comms = JSON.parse(buildCommsConsoleState(state));
  return JSON.stringify({
    navigation,
    comms,
    comms_auto: state.stationRatings?.['comms'] === 'Backfill',
  });
}

/**
 * Destroyer Captain console — combined CaptainChair + Sensors view.
 *
 * Delegates to buildCaptainConsoleState and buildSensorsConsoleState,
 * nesting payloads under `captain` and `sensors` keys.
 *
 * Used when the state has a sensors blackboard present and the station is
 * captain — indicating a Destroyer captain who also owns the sensors system.
 *
 * @param {object} state
 */
export function buildDestroyerCaptainConsoleState(state) {
  const captain = JSON.parse(buildCaptainConsoleState(state));
  const sensors = JSON.parse(buildSensorsConsoleState(state));
  return JSON.stringify({
    captain,
    sensors,
    captain_auto: state.stationRatings?.['captain'] === 'Backfill',
  });
}

/**
 * Destroyer Tactical console — combined Weapons + Navigation + Comms view.
 *
 * Delegates to buildWeaponsConsoleState, buildNavigationConsoleState, and
 * buildCommsConsoleState, nesting payloads under `weapons`, `navigation`,
 * and `comms` keys so the per-console iframe receives all three panels'
 * data without key collision.
 *
 * Used when the state has both a navigation blackboard and a comms
 * blackboard present on the tactical station — indicating a Destroyer
 * tactical officer who also owns navigation and comms.
 *
 * @param {object} state
 */
export function buildDestroyerTacticalConsoleState(state) {
  const weapons = JSON.parse(buildWeaponsConsoleState(state));
  const navigation = JSON.parse(buildNavigationConsoleState(state));
  const comms = JSON.parse(buildCommsConsoleState(state));
  return JSON.stringify({
    weapons,
    navigation,
    comms,
    tactical_auto: state.stationRatings?.['tactical'] === 'Backfill',
  });
}

/**
 * Destroyer Engineering console — combined Shields + Power + Repair view.
 *
 * Delegates to buildShieldsConsoleState, buildPowerConsoleState, and
 * buildRepairConsoleState, nesting payloads under `shields`, `power`, and
 * `repair` keys so the per-console iframe receives all three panels' data
 * without key collision.
 *
 * Used when the state has a shields blackboard present on the engineering
 * station — indicating a Destroyer engineer who owns shields, power, and
 * repair (unlike the Cruiser engineer who owns only power and repair).
 *
 * @param {object} state
 */
export function buildDestroyerEngineeringConsoleState(state) {
  const shields = JSON.parse(buildShieldsConsoleState(state));
  const power = JSON.parse(buildPowerConsoleState(state));
  const repair = JSON.parse(buildRepairConsoleState(state));
  return JSON.stringify({
    shields,
    power,
    repair,
    engineering_auto: state.stationRatings?.['engineering'] === 'Backfill',
  });
}

// ── Window dispatch (for non-module inline scripts in client.html) ──────────

if (typeof window !== 'undefined') {
  window.buildConsoleState = function buildConsoleState(consoleName, state) {
    // Post issue #618: `consoleName` is a lowercase station id (from each
    // per-console iframe's `initConsole({ name: '...' })` and from
    // `__updateConsole('...', ...)`). Pre-#618 these were PascalCase
    // Console enum names.
    switch (consoleName) {
      case 'tactical':
        // Destroyer merged console: if navigation and comms blackboards are
        // present on the tactical station, return the 3-system merged payload.
        if (state.blackboards && state.blackboards['navigation'] && state.blackboards['comms']) {
          return buildDestroyerTacticalConsoleState(state);
        }
        return buildWeaponsConsoleState(state);
      case 'captain':
        // Destroyer merged console: if a sensors blackboard is present on
        // the captain station, return the captain + sensors merged payload.
        if (state.blackboards && state.blackboards['sensors']) {
          return buildDestroyerCaptainConsoleState(state);
        }
        return buildCaptainConsoleState(state);
      case 'helm':        return buildHelmConsoleState(state);
      case 'repair':      return buildRepairConsoleState(state);
      case 'power':       return buildPowerConsoleState(state);
      case 'shields':     return buildShieldsConsoleState(state);
      case 'sensors':     return buildSensorsConsoleState(state);
      case 'comms':
        // Cruiser merged console: if a navigation blackboard is present,
        // the comms station holds both navigation and comms systems —
        // return the merged payload. Battleship pure comms falls through
        // to buildCommsConsoleState (no navigation blackboard).
        if (state.blackboards && state.blackboards['navigation']) {
          return buildCruiserCommsConsoleState(state);
        }
        return buildCommsConsoleState(state);
      case 'navigation':  return buildNavigationConsoleState(state);
      case 'science':     return buildScienceConsoleState(state);
      case 'engineering':
        // Destroyer merged console: if a shields blackboard is present on
        // the engineering station, return the 3-system merged payload.
        if (state.blackboards && state.blackboards['shields']) {
          return buildDestroyerEngineeringConsoleState(state);
        }
        return buildEngineeringConsoleState(state);
      default:            return '{}';
    }
  };
}
