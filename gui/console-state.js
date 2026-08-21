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

// ── Shared payload sub-shapes (issue #827) ──────────────────────────────────
// The per-console payload typedefs below each builder reference these instead
// of re-inlining the shapes. Every per-ship console HTML render(s) doc
// comment points at its payload typedef here — this file is the single home
// of the builder ↔ console-iframe payload contract.

/**
 * One radar blip, as produced by buildBlips / buildWaypointBlip /
 * buildTargetBlip. Marker blips (waypoint / tactical-target /
 * science-target) additionally carry `edge`, `world_x`, `world_z` (and the
 * waypoint `source_uuid`).
 *
 * `torpedo_armed` is the server's authoritative weapon-capability fact for a
 * hostile contact (issue #957); `torpedo_badge` is the resolved display text
 * `buildWeaponsConsoleState` folds in from it. The server never sends the badge
 * text — it sends the capability, and the client decides how to say it.
 *
 * @typedef {{ uuid: string, radar_x: number, radar_y: number,
 *             scaled_radius: number, kind: string, icon: string,
 *             color: number[]|null, objective_target: boolean,
 *             name: string|null, selectable: boolean,
 *             threat_level?: string|null, description?: string|null,
 *             target_tags?: string[], edge?: boolean,
 *             world_x?: number, world_z?: number,
 *             torpedo_armed?: boolean, torpedo_badge?: string,
 *             source_uuid?: string|null }} RadarBlip
 */

/**
 * One radar region (area shape), as produced by buildRadarRegions; the
 * projected variant adds the radar_* / scaled_* fields.
 *
 * @typedef {{ uuid: string, x: number, z: number, shape: string,
 *             radius: number|null, inner_radius: number|null,
 *             outer_radius: number|null, half_extents: number[]|null,
 *             yaw: number|null, color: *, name: string|null,
 *             objective_target: boolean, radar_x?: number, radar_y?: number,
 *             scaled_radius?: number|null, scaled_inner_radius?: number|null,
 *             scaled_outer_radius?: number|null,
 *             scaled_half_extents?: number[]|null }} RadarRegion
 */

/**
 * Per-station hull aggregate (aggregateStationHull) — the console footer's
 * `ph-station-damage` bar reads this. `window.buildConsoleState` merges it
 * into EVERY console payload as `own_hull`, so each payload typedef below
 * carries it implicitly even where the builder does not set it itself.
 *
 * @typedef {{ entries: Array<{system_id: string, current: number,
 *                             max_hp: number}>,
 *             totalCurrent: number, totalMax: number,
 *             pct: number, damagePct: number }} StationHullAggregate
 */

/**
 * The systems this station is HOLDING right now — its authored systems, minus
 * any the human seek has moved to another station, plus any it has parked here
 * (issue #984). `window.buildConsoleState` merges it into EVERY console payload
 * as `hosted_systems`, the same way `own_hull` is merged, so a console asks the
 * same question whatever payload shape it has. See `withVisitingSystems`.
 *
 * @typedef {string[]} HostedSystems
 */

// ── Entity position / radius helpers ───────────────────────────────────────

/**
 * World X from an entity snapshot.
 * Supports both flat `e.x` field and 3-element `e.position` array.
 * @param {{ x?: number, position?: number[] }} e
 */
import { t } from './strings.js';
import { buildTutorialState } from './tutorial-state.js';

/**
 * Display name for a station id.
 *
 * Station names in TOML are lookup identifiers (Rust matches them by name, see
 * scripts/strings-rules.mjs), so they are localised here from a derived id
 * instead: `station.<id>.name`. A missing CSV row surfaces via the string
 * table's missing-id policy (rendered as ⟨station.<id>.name⟩) so gaps are
 * visible instead of being papered over by a hardcoded-English fallback.
 */
export function stationDisplayName(id) {
  return t('station.' + id + '.name');
}

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

/**
 * Split the damageable-system list into ownerless "core" systems (which remain
 * on the repair console) and a list of dispatchable repair targets — one per
 * station that owns any damageable system (regardless of current damage),
 * plus a `core` bucket whenever any ownerless system exists. Every damageable
 * station always gets a target/dispatch entry so repair teams can be sent
 * proactively, not just once damage has occurred. Used by the repair console
 * (issue #12).
 *
 * @param {Array<{system_id,display_name,current,max_hp,tier}>} systemHull
 * @param {Object<string,string[]>} stationSystems  station id → system ids
 */
export function repairCoreAndTargets(systemHull, stationSystems, damageableSystems) {
  const hull = Array.isArray(systemHull) ? systemHull : [];
  const stations = stationSystems || {};

  const owned = new Set();
  Object.keys(stations).forEach(st => (stations[st] || []).forEach(id => owned.add(id)));

  const coreSystems = hull.filter(h => !owned.has(h.system_id));

  // Dispatch targets come from the id-only `damageable_systems` list, NOT from
  // `system_hull`. Post issue #737 `system_hull` is a host-side projection —
  // Engineering cannot see another station's rows — but it must still be able
  // to send a team there, and system ids carry no hull detail. Falling back to
  // the visible rows keeps older payloads (and the legacy path) working.
  const dispatchable = Array.isArray(damageableSystems) && damageableSystems.length > 0
    ? damageableSystems.map(id => (id && id.system_id) || id)
    : hull.map(h => h.system_id);

  // `damage_pct` is only reported for stations whose detail this recipient
  // actually holds; `null` means "not visible to me", which is different from
  // "undamaged" and must not be rendered as 0.
  const targets = [];
  Object.keys(stations).forEach(st => {
    const stationIds = stations[st] || [];
    if (!stationIds.some(id => dispatchable.includes(id))) return;
    const agg = aggregateStationHull(st, hull, stations);
    targets.push({
      id: st,
      label: stationDisplayName(st),
      damage_pct: agg.totalMax > 0 ? agg.damagePct : null,
    });
  });

  const coreIds = dispatchable.filter(id => !owned.has(id));
  if (coreIds.length > 0) {
    const coreMax = coreSystems.reduce((s, h) => s + (h.max_hp || 0), 0);
    const coreCur = coreSystems.reduce((s, h) => s + (h.current || 0), 0);
    targets.push({
      id: 'core',
      label: t('console.repair.core'),
      damage_pct: coreMax > 0 ? 1 - coreCur / coreMax : null,
    });
  }

  return { coreSystems, targets };
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

// Camera views are now marker-name-based, supplied by the server blackboard.

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
    name: t('console.common.waypoint'),
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
    name: opts.label || target.name || t('console.common.target'),
    selectable: false,
    objective_target: false,
    edge,
    world_x: entityX(target),
    world_z: entityZ(target),
  };
}

/**
 * Fold the server's torpedo-capability fact into a display badge (issue #957).
 *
 * The server decides WHO is torpedo-armed — `RadarBlip.torpedo_armed` is set
 * from the contact's live torpedo system and its hostility to this ship, and the
 * client never re-derives it from a hull name, icon or model. This function only
 * decides HOW to say it, which is why the string id is resolved here rather than
 * being sent over the wire: the badge is client copy, so it never crosses the
 * `localiseTree` ingress boundary that resolves server-sent ids, and spelling
 * the id out in a `t()` call is what lets `scripts/check-strings.mjs` see it.
 *
 * Returns a new array; blips are copied rather than mutated, so the caller's
 * server-owned blip objects stay untouched. A blip with no capability flag is
 * passed through unchanged and carries no `torpedo_badge` key at all.
 *
 * @param {RadarBlip[]} blips
 * @returns {RadarBlip[]}
 */
export function foldTorpedoBadges(blips) {
  return (blips || []).map(b => (
    b && b.torpedo_armed
      ? { ...b, torpedo_badge: t('console.radar.torpedo_armed') }
      : b
  ));
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
 * Payload contract for the Tactical/Weapons console iframe (issue #827).
 * Rendered by gui/battleship/tactical.html and gui/cruiser/tactical.html.
 *
 * @typedef {{ target_uuid: string|null, target_name: string|null,
 *             banks: Array, tubes: Array, torpedo_count: number,
 *             torpedo_max: number, phaser_mode: string,
 *             blips: RadarBlip[], regions: RadarRegion[],
 *             ship_heading: number, ship_x: number, ship_z: number,
 *             ship_speed: number,
 *             phaser_arcs: Array<{range_frac: number|null}>,
 *             torpedo_arcs: Array, blasters: Array,
 *             own_hull: StationHullAggregate, tactical_auto: boolean,
 *             station_rating: string }} WeaponsConsolePayload
 */

/**
 * Tactical / Weapons console. Returns JSON of {@link WeaponsConsolePayload}.
 * Reads raw sim truth from `state.blackboards['tactical']` (WeaponsBlackboard),
 * falling back to legacy camelCase properties for compatibility.
 *
 * @param {{ blackboards?, weaponsTarget?, weaponsBanks?, weaponsTubes?,
 *           weaponsTorpedoCount?, weaponsPhaserMode?, blasterBanks? }} state
 */
export function buildWeaponsConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['tactical']) || {};
  // Combat Lock + radar blips/regions moved to the tactical-radar blackboard
  // (issue #829). `selected_target` is the authoritative lock; fall back to the
  // Weapons blackboard's `target_uuid` (still published for reconnect resync).
  const tacRadarBb = (state.blackboards && state.blackboards['tactical-radar']) || {};
  const targetUuid   = tacRadarBb.selected_target ?? bb.target_uuid ?? state.weaponsTarget ?? null;
  const targetName   = bb.target_name   ?? state.weaponsTargetName   ?? null;
  const banks        = bb.banks         ?? state.weaponsBanks        ?? [];
  const tubes        = bb.tubes         ?? state.weaponsTubes        ?? [];
  const torpedoCount = bb.torpedo_count ?? state.weaponsTorpedoCount ?? 0;
  const torpedoMagBb = (state.blackboards && state.blackboards['torpedo-magazine']) || {};
  const torpedoMax   = torpedoMagBb.capacity ?? torpedoCount;
  const phaserMode   = bb.phaser_mode   ?? state.weaponsPhaserMode   ?? 'Auto';
  const regions      = tacRadarBb.regions ?? [];
  const phaserArcs   = bb.phaser_arcs   ?? state.phaserArcConfigs   ?? [];
  const torpedoArcs  = bb.torpedo_arcs  ?? state.torpedoArcConfigs  ?? [];
  const blasters     = bb.blasters      ?? state.blasterBanks        ?? [];

  // AUTO gate: per-system, not whole-station — a "Simplified" rating only
  // automates the phaser bank(s), leaving torpedoes/blasters human. Derive
  // from the generic per-system control-source map intersected with this
  // ship's actual phaser system ids (data-driven; works for the destroyer's
  // single bank and the battleship/cruiser's two banks alike).
  const phaserSystemIds = (state.stationSystems?.['tactical'] || []).filter(id => id.startsWith('phaser-'));
  const controlSources = state.controlSources || {};
  const tacticalAuto = phaserSystemIds.length > 0 && phaserSystemIds.every(id => controlSources[id] === 'Ai');

  const range = state.weaponsRadarRange ?? WEAPONS_RADAR_RANGE;
  const mappedPhaserArcs = phaserArcs.map(a => ({
    ...a,
    range_frac: a.beam_range != null ? a.beam_range / range : null,
  }));

  // Blips: authoritative server blips from the tactical-radar blackboard
  // (issue #829 — moved off the Weapons blackboard), otherwise build from
  // asteroids.
  //
  // COPY, never alias. The science marker and waypoint below are `push`ed onto
  // `blips`, and the first two sources are arrays this builder does not own:
  // `sim-state.js` replaces `blackboards[systemId]` only when a
  // `BlackboardUpdate` arrives, and the server's `LastBroadcastBlackboards`
  // delta cache re-sends a blackboard only when it changes. So on a stationary
  // ship the store keeps handing back the same array, and pushing into it would
  // append the same two markers on every build — unbounded growth in the client
  // store. `buildBlips()` already returns a fresh array, so only the two
  // borrowed sources need copying.
  let blips = tacRadarBb.blips ? tacRadarBb.blips.slice() : [];
  if (blips.length === 0) {
    blips = (state.weaponsBlips || []).slice();
  }
  if (blips.length === 0) {
    blips = buildBlips(
      state.asteroids || [],
      state.shipX || 0,
      state.shipZ || 0,
      state.shipYaw || 0,
      range,
      { rotate: true }
    );
  }

  // Torpedo-armed badge (issue #957): fold the server's capability fact into
  // display text before the marker blips below are appended — a waypoint or
  // science marker is client-minted and carries no capability to badge.
  blips = foldTorpedoBadges(blips);

  // Derive target_name from the locked server blip when no explicit name is stored.
  const resolvedTargetName = targetName || (targetUuid && blips.find(b => b.uuid === targetUuid)?.name) || null;

  // Add shared target markers (science target + navigation waypoint)
  const entities = state.asteroids || [];
  const sensBb = state.blackboards?.['sensors'];
  const sciTargetUuid = sensBb?.science_target_uuid || state.sensorsTarget || null;
  const sciMarker = buildTargetBlip(
    sciTargetUuid, entities, state.shipX || 0, state.shipZ || 0, state.shipYaw || 0, range,
    { rotate: true, edgeClamp: true, kind: 'science-target', color: [0.2, 0.4, 1.0], label: t('console.radar.science_target') }
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
    torpedo_max:   torpedoMax,
    phaser_mode:   phaserMode,
    blips,
    regions,
    ship_heading:  (((state.shipYaw || 0) * 180 / Math.PI % 360) + 360) % 360,
    ship_x:        state.shipX || 0,
    ship_z:        state.shipZ || 0,
    ship_speed:    state.forwardSpeed || 0,
    phaser_arcs:   mappedPhaserArcs,
    torpedo_arcs:  torpedoArcs,
    blasters,
    own_hull:      aggregateStationHull('tactical', state.consoleHull, state.stationSystems),
    tactical_auto: tacticalAuto,
    station_rating: state.stationRatings?.['tactical'] || 'Std',
  });
}

/**
 * Payload contract for the Captain console iframe (issue #827).
 * Rendered by gui/battleship/captain.html and gui/cruiser/captain.html.
 *
 * @typedef {{ red_alert: boolean, red_alert_system_id: string,
 *             red_alert_auto: boolean, weapons_hold: boolean,
 *             viewscreen_system_id: string,
 *             viewscreen_auto: boolean, view_direction: string,
 *             camera_views: Array, view_mode: string, objectives: Array,
 *             boosted_objective_id: string|null, deadlines: Array,
 *             operations: {capabilities: Array, active: object|null,
 *                          refusal: string|null},
 *             hull_integrity_pct: number, game_status: string,
 *             blips: RadarBlip[],
 *             own_hull: StationHullAggregate }} CaptainConsolePayload
 */

/**
 * The ship's external-operation readout (issue #1026).
 *
 * Its own blackboard under its own channel key, not a field on the captain's:
 * an operation is something the ship does rather than a system aboard it. A
 * hull that authored no `[operations]` publishes none at all, which is the empty
 * shape returned here — the panel renders its own "no capability" state off it
 * rather than the console guessing.
 * @param {{ blackboards }} state
 */
function operationsPayload(state) {
  const bb = (state.blackboards && state.blackboards['operations']) || {};
  return {
    capabilities: bb.capabilities ?? [],
    active:       bb.active       ?? null,
    refusal:      bb.refusal      ?? null,
  };
}

/**
 * CaptainChair console. Returns JSON of {@link CaptainConsolePayload}.
 * @param {{ blackboards, redAlert, currentView, objectives, hullPct, blips }} state
 */
export function buildCaptainConsoleState(state) {
  const bb = state.blackboards && state.blackboards['captain'];
  if (bb) {
    return JSON.stringify({
      red_alert:             bb.red_alert             ?? false,
      red_alert_system_id:   bb.red_alert_system_id   ?? 'red-alert',
      red_alert_auto:        bb.red_alert_auto         ?? false,
      // The tactical restraint lever (issue #1041): guns cold while the ship
      // stays at stations. Same console and same control source as the alert,
      // so it rides the captain blackboard beside it.
      weapons_hold:          bb.weapons_hold           ?? false,
      viewscreen_system_id:  bb.viewscreen_system_id  ?? 'viewscreen',
      viewscreen_auto:       bb.viewscreen_auto        ?? false,
      view_direction:        bb.view_direction         ?? '',
      camera_views:          bb.camera_views           ?? [],
      view_mode:             'Camera',
      camera_views:          bb.camera_views           ?? [],
      objectives:            bb.objectives             ?? [],
      boosted_objective_id:  bb.boosted_objective_id   ?? null,
      // Visible mission deadlines, already counted down server-side against the
      // authoritative SimTick (issue #1024) — the client formats, never clocks.
      deadlines:             bb.deadlines              ?? [],
      // The external-operation readout (issue #1026) — a blackboard of its own,
      // so it is read from its own key rather than off the captain's.
      operations:            operationsPayload(state),
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
  const viewDirection = state.currentView || '';
  return JSON.stringify({
    red_alert:             state.redAlert    || false,
    red_alert_system_id:   'red-alert',
    red_alert_auto:        redAlertAuto,
    // No blackboard, no hold: the legacy fallback has no wire source for it,
    // and "released" is the state a console with nothing to read should show.
    weapons_hold:          false,
    viewscreen_system_id:  'viewscreen',
    viewscreen_auto:       viewscreenAuto,
    view_direction:        viewDirection,
    camera_views:          state.cameraViews || [],
    view_mode:             'Camera',
    camera_views:          state.cameraViews || [],
    objectives:            state.objectives  || [],
    boosted_objective_id:  null,
    // No blackboard, no deadlines: the legacy fallback has no wire source for
    // them, and an empty list renders the panel's own empty state.
    deadlines:             [],
    // Operations ride their own blackboard, so the legacy fallback carries them
    // unchanged rather than blanking them: a captain blackboard that has not
    // arrived says nothing about whether an operations one has.
    operations:            operationsPayload(state),
    hull_integrity_pct:    state.hullPct     || 100,
    game_status:           state.redAlert
                             ? 'RED ALERT — All hands to battlestations.'
                             : 'Standing by. All systems nominal.',
    blips:                 state.blips       || [],
    own_hull:              aggregateStationHull('captain', state.consoleHull, state.stationSystems),
  });
}

/**
 * Payload contract for the Command console iframe (issue #1107). Rendered by
 * gui/command-console.html.
 *
 * @typedef {{ command_system_id: string, directed_station: string,
 *             directed_station_name: string, directed_station_ai: boolean,
 *             command_auto: boolean, selected_stance: string,
 *             stances: Array<{id: string, label: string, kind: string,
 *                             high_alert: boolean}> }} CommandConsolePayload
 */

/**
 * Command console. Returns JSON of {@link CommandConsolePayload}.
 *
 * Reads the `command` blackboard mirror (`state.blackboards['command']`), which
 * carries the directed proving Station, whether it is currently AI-controlled
 * (and therefore directable), the selectable stances and the stance in force.
 * The persistent non-colour automation cue the console renders is derived from
 * `directed_station_ai` / `command_auto` — never from a colour change.
 *
 * @param {{ blackboards? }} state
 */
export function buildCommandConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['command']) || null;
  return JSON.stringify({
    command_system_id:      bb?.command_system_id     ?? 'command',
    directed_station:       bb?.directed_station       ?? '',
    directed_station_name:  bb?.directed_station_name  ?? '',
    directed_station_ai:    bb?.directed_station_ai    ?? false,
    command_auto:           bb?.command_auto           ?? false,
    selected_stance:        bb?.selected_stance        ?? '',
    stances:                bb?.stances                ?? [],
  });
}

/**
 * Non-binding Command intent advice for a directed target Station (issue #1108).
 *
 * Criterion 2: a human holding the Command-directed Station keeps full ordinary
 * authority, and may ALSO see the current Command intent as advice. When the
 * `command` blackboard names THIS console's station as the directed Station and
 * that Station is human-held (`directed_station_ai === false`), returns the
 * stance currently in force — `{ stance_id, stance_label, high_alert }` — for
 * the console to render as advisory, never binding. The stance in force is
 * always computable server-side (a stored order, else the alert-neutral
 * fallback), so advice is present whenever the console is the directed target
 * and human-held. Any other case returns `null`: an AI-controlled directed
 * Station is *directed* through the Command console, not advised here.
 *
 * @param {object} state simState
 * @param {string} consoleName station id
 * @returns {{stance_id:string, stance_label:string, high_alert:boolean}|null}
 */
export function commandAdviceFor(state, consoleName) {
  const bb = state && state.blackboards && state.blackboards['command'];
  if (!bb) return null;
  if (bb.directed_station !== consoleName) return null;
  if (bb.directed_station_ai) return null;
  const id = bb.selected_stance || '';
  if (!id) return null;
  const opt = (bb.stances || []).find(s => s && s.id === id) || null;
  return {
    stance_id: id,
    stance_label: opt ? (opt.label || '') : '',
    high_alert: opt ? !!opt.high_alert : false,
  };
}

/**
 * Attach `command_advice` to a target console's payload (issue #1108).
 *
 * A general overlay, applied to every console the same way `withTutorialOverlay`
 * is: it adds `command_advice` only to the console that is the current directed
 * target while that Station is human-held, and is a no-op everywhere else. The
 * target console renders it as a non-binding advisory line.
 *
 * @param {string} consoleName station id
 * @param {object} state simState
 * @param {string} json the console payload built so far
 * @returns {string} json, with `command_advice` when this console is advised
 */
export function withCommandAdvice(consoleName, state, json) {
  try {
    const advice = commandAdviceFor(state, consoleName);
    if (!advice) return json;
    const obj = JSON.parse(json);
    obj.command_advice = advice;
    return JSON.stringify(obj);
  } catch (_) {
    return json;
  }
}

/**
 * Payload contract for the Helm console iframe (issue #827). Rendered by
 * gui/battleship/helm.html, gui/cruiser/helm.html and gui/destroyer/helm.html.
 *
 * @typedef {{ range: number, ship_heading: number, speed: number, x: number,
 *             z: number, yaw: number, impulse_charge_progress: number,
 *             on_screen: boolean, blips: RadarBlip[],
 *             waypoint: {x: number, z: number}|null,
 *             own_hull: StationHullAggregate, boost_enabled: boolean,
 *             boost_battery: number, boost_active: boolean,
 *             helm_auto: boolean, engine_port_thrust: number,
 *             engine_stbd_thrust: number, engine_port_auto: boolean,
 *             engine_stbd_auto: boolean, lateral_speed: number,
 *             lateral_input: number, lateral_auto: boolean,
 *             lateral_is_online: boolean, red_alert: boolean,
 *             hostile_arcs: HostileWeaponArcContact[],
 *             hostile_arc_color: number[] }} HelmConsolePayload
 */

/**
 * One hostile contact's weapon arcs, as published on `HelmBlackboard
 * ::hostile_weapon_arcs` (issue #874).
 *
 * `bearing_deg` is a WORLD bearing in the same convention as ship yaw, produced
 * once server-side by `weapons::arc_geometry::weapon_arc_sectors` — the same
 * sectors the backfilled helm AI's exposure fact is reduced from. The client
 * never recomputes these from the hostile's yaw: doing so would make the human
 * and the AI agree only by coincidence.
 *
 * @typedef {{ uuid: string, x: number, z: number,
 *             arcs: {bearing_deg: number, half_angle_deg: number,
 *                    range: number}[] }} HostileWeaponArcContact
 */

/**
 * Helm console. Returns JSON of {@link HelmConsolePayload}.
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

  // Prefer the live per-tick range (shrinks as the helm-radar system takes
  // damage — see `apply_radar_damage_modifiers`) over the static ship config.
  const range = bb.radar_range ?? state.helmRadarRange ?? HELM_RADAR_RANGE;
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
  // Combat Lock now lives on the tactical-radar blackboard (issue #829); every
  // radar renders the other consoles' targets from these aggregated facts.
  const tacRadarBb = state.blackboards?.['tactical-radar'];
  const sensBb = state.blackboards?.['sensors'];
  const tacMarker = buildTargetBlip(
    tacRadarBb?.selected_target ?? tacBb?.target_uuid, entities, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true, kind: 'tactical-target', color: [1.0, 0.2, 0.2], label: t('console.radar.tactical_target') }
  );
  if (tacMarker) blips.push(tacMarker);
  const sciTargetUuid = sensBb?.science_target_uuid || state.sensorsTarget || null;
  const sciMarker = buildTargetBlip(
    sciTargetUuid, entities, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true, kind: 'science-target', color: [0.2, 0.4, 1.0], label: t('console.radar.science_target') }
  );
  if (sciMarker) blips.push(sciMarker);

  return JSON.stringify({
    range,
    ship_heading:            (((shipYaw * 180 / Math.PI % 360) + 360) % 360),
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
    // Lateral thrust state from per-system blackboard.
    lateral_speed:       bb.lateral_speed ?? 0,
    lateral_input:       (state.blackboards?.['helm-lateral-thrust']?.lateral_input) ?? 0,
    lateral_auto:        (state.blackboards?.['helm-lateral-thrust']?.auto) ?? false,
    lateral_is_online:   (state.blackboards?.['helm-lateral-thrust']?.is_online) ?? true,
    // ── Hostile weapon-arc overlay (issue #874) ───────────────────────────
    // The server already gates the blackboard field on red alert AND on this
    // being the local ship, so the list is normally absent entirely. The
    // `state.redAlert` guard here is a second, client-side latch on the same
    // condition: a stale blackboard mirror must not outlive the alert that
    // justified it.
    //
    // Pass-through, deliberately: no projection, no re-derivation. Every arc
    // the helm draws is an arc the server produced.
    red_alert:           !!state.redAlert,
    hostile_arcs:        state.redAlert ? (bb.hostile_weapon_arcs || []) : [],
    // Authored per hull in `[helm_console] hostile_arc_color` (AGENTS.md #11 —
    // a gameplay-adjacent presentation value, so TOML rather than inline JS).
    // Passed straight through. `ClientSimState` owns the one client-side
    // placeholder; the component carries none, and paints no overlay at all
    // rather than invent a colour if this arrives null.
    hostile_arc_color:   state.hostileArcColor || null,
    // ── Contextual dock control (issue #1159) ─────────────────────────────
    // The dock's own blackboard, published under its system id by a hull whose
    // helm owns a `dock` system. A hull without one publishes no such
    // blackboard, so `dock` is null and the helm console shows no dock control
    // at all — the destroyer is unchanged until #1164 gives it a dock. The
    // contextual control appears exactly when `available` is true and becomes
    // the undock control when `docked`; every string it shows is a `t()` id, so
    // no English crosses here.
    dock:                buildHelmDockView(state),
    // ── Under-tow-load indicator (issue #1157) ────────────────────────────
    // The helm feels the tractor's load: while this ship's beam holds a target,
    // its top speed and turn rate are penalised, and the console says so and
    // why. Read from the same `tractor` blackboard the engineering console
    // shows; a hull with no tractor publishes none, so this is null and no
    // indicator appears. The label and the towed hull's name are both `t()` ids
    // — no English crosses here.
    tow_load:            buildHelmTowLoadView(state),
  });
}

/**
 * The under-tow-load indicator for the helm console (issue #1157), read from the
 * raw `tractor` blackboard this ship's tractor system publishes. Returns `null`
 * when the hull has no tractor (no blackboard) or is not currently holding a
 * target, so the console shows no indicator — a hull without a tractor is
 * unchanged.
 *
 * The ship is "under tow load" exactly while the coupling holds a target: the
 * server's own `coupled_target` is the gate, mirrored here rather than
 * re-derived so the helm and the engineering console agree by construction.
 * `target_name` is the towed hull's own name id — the "why" — resolved by the
 * console through `t()`; never English.
 *
 * @param {{ blackboards? }} state
 * @returns {object|null}
 */
export function buildHelmTowLoadView(state) {
  const bb = state.blackboards && state.blackboards['tractor'];
  if (!bb || !bb.coupled_target) return null;
  return {
    active: true,
    target_name: bb.coupled_target_name ?? null,
  };
}

/**
 * The dock control's view for the helm console (issue #1159), read from the raw
 * `dock` blackboard the helm-owned `dock` system publishes. Returns `null` when
 * the hull has no dock system (no blackboard), so the console renders no control.
 *
 * `present` is the server's own gate; the client mirrors it rather than
 * re-deriving range so the human and the AI agree by construction. `available`
 * is when the Dock control shows, `docked` when it becomes Undock, and `refusal`
 * is a `strings.csv` id the console resolves.
 *
 * @param {{ blackboards? }} state
 * @returns {object|null}
 */
export function buildHelmDockView(state) {
  const bb = state.blackboards && state.blackboards['dock'];
  if (!bb) return null;
  return {
    range: bb.range ?? 0,
    available: !!bb.available,
    available_target: bb.available_target ?? null,
    available_target_name: bb.available_target_name ?? null,
    engaged: !!bb.engaged,
    docked: !!bb.docked,
    docked_to: bb.docked_to ?? null,
    docked_to_name: bb.docked_to_name ?? null,
    refusal: bb.refusal ?? null,
  };
}

/**
 * Normalize a Rust `TeamSlot` serde enum into the flat format the
 * `ph-repair-teams` web component expects.
 *
 * Rust TeamSlot is an externally-tagged enum:
 *   {"Idle":{}}
 *   {"Travelling":{"system_id":"..","display_name":"..","elapsed":2.5}}
 *   {"Repairing":{"system_id":"..","display_name":".."}}
 *   {"Returning":{"remaining":1.0,"system_id":"..","display_name":"..",...}}
 *
 * Returns { id, label, status, target, progress_pct }.
 *
 * @param {object} slot - Raw serde serialization of a TeamSlot
 * @param {number} idx  - 0-based index for ID / label generation
 * @param {number} travelDurationSecs - travel duration for progress scaling
 */
function normalizeTeamSlot(slot, idx, travelDurationSecs) {
  const id = idx;
  const label = t('component.repair_teams.team', { n: idx + 1 });
  const dur = travelDurationSecs > 0 ? travelDurationSecs : 5.0;

  // Detect which enum variant is active by checking for known variant keys.
  const variant = slot.Idle !== undefined ? 'Idle'
    : slot.Travelling !== undefined ? 'Travelling'
    : slot.Repairing !== undefined ? 'Repairing'
    : slot.Returning !== undefined ? 'Returning'
    : null;

  if (!variant) {
    // Already flat format or unknown — pass through if it looks flat.
    if (slot.status) return { id, label, status: slot.status, target: slot.target || '', progress_pct: slot.progress_pct || 0 };
    return { id, label, status: 'idle', target: '', progress_pct: 0 };
  }

  switch (variant) {
    case 'Idle':
      return { id, label, status: 'idle', target: '', progress_pct: 0 };
    case 'Travelling': {
      const data = slot.Travelling;
      const elapsed = data.elapsed || 0;
      return {
        id, label,
        status: 'travelling',
        target: data.display_name || data.system_id || '',
        system_id: data.system_id || null,
        progress_pct: Math.min(elapsed / dur, 1),
        priority: data.priority != null ? data.priority : null,
      };
    }
    case 'Repairing': {
      const data = slot.Repairing;
      return {
        id, label,
        status: 'repairing',
        target: data.display_name || data.system_id || '',
        system_id: data.system_id || null,
        progress_pct: 1.0,
        priority: data.priority != null ? data.priority : null,
        // The system the host PINNED on this team's slot (issue #1015).
        // Pass-through, never re-derived: the console cannot see the ranking
        // the host resolved this against.
        priority_system_id: data.priority_system_id || null,
      };
    }
    case 'Returning': {
      const data = slot.Returning;
      const remaining = data.remaining || 0;
      return {
        id, label,
        status: 'returning',
        target: data.display_name || data.system_id || '',
        progress_pct: 1 - Math.min(remaining / dur, 1),
      };
    }
    default:
      return { id, label, status: 'idle', target: '', progress_pct: 0 };
  }
}

/**
 * Aggregate hull health across every damageable system on the ship (all of
 * `system_hull`, not just one station's slice) — the "overall hull" figure
 * for the Repair console's hero bar.
 *
 * `destroyedFraction` is the host's second whole-ship scalar (issue #1014):
 * the share of total capacity held by destroyed systems. It has **no local
 * fallback**, deliberately — the projected rows are a slice of the ship, so a
 * system destroyed at a station this client cannot see is absent from them
 * entirely, and any sum over them would report "nothing destroyed" precisely
 * when something was. Absent host value ⇒ `destroyed_pct: 0` (legacy hosts, and
 * the only honest default when the figure is genuinely unknown).
 *
 * @param {Array<{current,max_hp}>} systemHull
 * @param {number} [aggregateFraction] host ship-wide hull fraction
 * @param {number} [destroyedFraction] host ship-wide destroyed-capacity share
 */
export function overallHull(systemHull, aggregateFraction, destroyedFraction) {
  const hull = Array.isArray(systemHull) ? systemHull : [];
  const current = hull.reduce((s, h) => s + (h.current || 0), 0);
  const max = hull.reduce((s, h) => s + (h.max_hp || 0), 0);
  const destroyed_pct =
    typeof destroyedFraction === 'number' && Number.isFinite(destroyedFraction)
      ? Math.max(0, Math.min(1, destroyedFraction))
      : 0;
  // Post issue #737 `system_hull` is a per-recipient projection, so summing it
  // yields the *visible* slice, not the ship. When the host supplies the
  // authoritative ship-wide fraction, that wins — always. The local sum is only
  // a fallback for payloads predating the aggregate field.
  if (typeof aggregateFraction === 'number' && Number.isFinite(aggregateFraction)) {
    return { current, max, pct: aggregateFraction, destroyed_pct };
  }
  return { current, max, pct: max > 0 ? current / max : 1, destroyed_pct };
}

/**
 * Severity order of the wire `DamageTier` strings, worst last. Used only to
 * SORT the damaged-systems list for display (issue #1015) — the host owns the
 * ranking that decides where a repair team actually goes, and the console never
 * sends an ordinal derived from this.
 */
const TIER_SEVERITY = { Operational: 0, Damaged: 1, Disabled: 2, Destroyed: 3 };

/**
 * The repair console's damaged-systems list (issue #1015): every visible hull
 * row that is not `Operational`, worst-first, flagged with what the repair teams
 * are doing about it.
 *
 * Built from the rows the console was already given rather than from a new fold:
 * `systemHull` is the issue #737 projection (core rows, the Engineering
 * station's own rows, and any row a team is on site at), so this list is exactly
 * "the damage this player can see", which is also exactly the damage they are
 * entitled to steer. Tapping a row sends `set_repair_target_priority` with its
 * `system_id` and nothing else; the host decides whether any team can act on it.
 *
 * `prioritised` is a pure echo of the host's resolved pin, so a highlight can
 * only ever show a choice the server actually made.
 *
 * The `current < max_hp` half of the filter mirrors the host's own candidate
 * guard rather than duplicating a rule for its own sake: a `max_hp = 0` row is
 * permanently `Destroyed` (the tier test reads `current == 0` before any ratio)
 * AND permanently at max, so without it such a row would list forever at 0%
 * damage as a permanent no-op — untappable by anything the host would honour.
 * The same pairing is documented on `sweep_candidates` in
 * `src/modifiers/repair_teams.rs`, and for the same reason: neither predicate
 * implies the other.
 *
 * @param {Array<{system_id,display_name,current,max_hp,tier}>} systemHull
 * @param {Array<{status,system_id,priority_system_id}>} teams normalized slots
 */
export function repairDamagedSystems(systemHull, teams) {
  const rows = Array.isArray(systemHull) ? systemHull : [];
  const slots = Array.isArray(teams) ? teams : [];
  const pinned = new Set(slots.map(s => s && s.priority_system_id).filter(Boolean));
  const onSite = new Set(
    slots.filter(s => s && s.status === 'repairing').map(s => s.system_id).filter(Boolean)
  );
  return rows
    .filter(h => h && h.system_id
      && (TIER_SEVERITY[h.tier] || 0) > 0
      && (h.current || 0) < (h.max_hp || 0))
    .map(h => {
      const max = h.max_hp || 0;
      const current = h.current || 0;
      return {
        system_id:    h.system_id,
        display_name: h.display_name || h.system_id,
        tier:         h.tier,
        current,
        max_hp:       max,
        damage_pct:   max > 0 ? 1 - current / max : 0,
        prioritised:  pinned.has(h.system_id),
        in_progress:  onSite.has(h.system_id),
      };
    })
    .sort((a, b) =>
      (TIER_SEVERITY[b.tier] || 0) - (TIER_SEVERITY[a.tier] || 0)
      || b.damage_pct - a.damage_pct
      || (a.system_id < b.system_id ? -1 : a.system_id > b.system_id ? 1 : 0));
}

/**
 * Payload contract for the Repair console iframe (issue #827). Rendered by
 * gui/battleship/repair.html.
 *
 * @typedef {{ teams: Array<{id: number, label: string, status: string,
 *                           target: string, progress_pct: number}>,
 *             system_hull: Array<{system_id: string, display_name?: string,
 *                                 current: number, max_hp: number,
 *                                 tier?: number}>,
 *             damageable_systems: Array,
 *             damaged_systems: Array<{system_id: string, display_name: string,
 *               tier: string, current: number, max_hp: number,
 *               damage_pct: number, prioritised: boolean,
 *               in_progress: boolean}>,
 *             overall_hull: {current: number, max: number, pct: number,
 *                            destroyed_pct: number},
 *             core_systems: Array, dispatch_targets: Array<{id: string,
 *               label: string, damage_pct: number|null}>,
 *             travel_duration_secs: number,
 *             repair_auto: boolean }} RepairConsolePayload
 */

/**
 * Repair console. Returns JSON of {@link RepairConsolePayload}.
 * @param {{ blackboards, repairTeams, consoleHull }} state
 */
export function buildRepairConsoleState(state) {
  const bb = state.blackboards && state.blackboards['repair'];
  if (bb) {
    const rawTeams = bb.teams || [];
    const travelDur = bb.travel_duration_secs ?? 5.0;
    const teams = rawTeams.map((slot, idx) => normalizeTeamSlot(slot, idx, travelDur));
    // `system_hull` is what the HOST decided this recipient may see (issue
    // #737): core detail, this station's own systems, and any system a repair
    // team is currently on site at. The console renders what it was given and
    // never re-derives a ship-wide view from it.
    const systemHull = bb.system_hull ?? [];
    const damageableSystems = bb.damageable_systems ?? [];
    const aggregate = bb.aggregate_hull_fraction ?? state.hullAggregate;
    const destroyed = bb.destroyed_hull_fraction ?? state.hullDestroyed;
    const { coreSystems, targets } =
      repairCoreAndTargets(systemHull, state.stationSystems, damageableSystems);
    return JSON.stringify({
      teams,
      // SystemId-keyed fields (post issues #618/#619).
      system_hull:          systemHull,
      damageable_systems:   damageableSystems,
      // Tap-to-prioritise list (issue #1015) — the visible rows that are
      // actually broken, worst-first.
      damaged_systems:      repairDamagedSystems(systemHull, teams),
      // Authoritative ship-wide hull aggregate from the host — the only
      // whole-ship figures available now that `system_hull` is a projection,
      // and `destroyed_pct` is the share of it that is gone for good (#1014).
      overall_hull:         overallHull(systemHull, aggregate, destroyed),
      // Only ownerless "core" systems stay on the repair console; per-station
      // system status moved to each console's footer bar (issue #12).
      core_systems:         coreSystems,
      // Dispatchable, currently-damaged repair targets (stations + core).
      dispatch_targets:     targets,
      travel_duration_secs: bb.travel_duration_secs ?? 5.0,
      // Per-system, not whole-station: the repair *system* id is always the
      // literal "repair" regardless of which station owns it (battleship's
      // dedicated Repair station vs. cruiser/destroyer's Engineering), so
      // this works uniformly across all ship classes.
      repair_auto:          state.controlSources?.['repair'] === 'Ai',
      // External repair-team dispatch (issue #1161). Non-null only on a hull
      // that authored `[repair.external_dispatch]` (its blackboard carries a
      // `range`), so the console shows the dispatch control on exactly those
      // hulls; a hull without it renders nothing new. The target/refusal are
      // name/string ids the console resolves through `t()` — no English crosses.
      external_dispatch:    bb.external_dispatch_range == null ? null : {
        range:       bb.external_dispatch_range,
        target:      bb.external_dispatch_target ?? null,
        target_name: bb.external_dispatch_target_name ?? null,
        refusal:     bb.external_dispatch_refusal ?? null,
      },
    });
  }
  // Legacy fallback: derive damageable_systems from consoleHull (SystemId-keyed
  // after issue #618) so the repair panel renders even without the blackboard.
  // `consoleHull` is itself the #737 projection — the rows this recipient is
  // entitled to — so the fallback is likewise a partial view, and the hero bar
  // still reads the host's `hullAggregate` rather than summing those rows.
  const legacyHull = state.consoleHull || [];
  const legacy = repairCoreAndTargets(legacyHull, state.stationSystems);
  // `state.repairTeams` is the RAW wire shape — externally-tagged `TeamSlot`
  // objects from `RepairState`, or the literal `'Idle'` strings `sim-state.js`
  // pre-seeds from `repair_team_count`. Feeding those to the fold above would
  // find no `status` and no `priority_system_id` on any of them, so every row
  // would come out `prioritised: false, in_progress: false` — flags that read as
  // "the host decided nothing" rather than as "this path cannot tell", which is
  // exactly the kind of quiet lie the flags exist to prevent. Normalize first,
  // through the same function the blackboard path uses.
  const legacyTeams = (state.repairTeams || [])
    .map((slot, idx) => normalizeTeamSlot(slot, idx, 5.0));
  return JSON.stringify({
    teams:                state.repairTeams || [],
    system_hull:          legacyHull,
    damageable_systems:   legacyHull.map(h => h.system_id),
    damaged_systems:      repairDamagedSystems(legacyHull, legacyTeams),
    overall_hull:         overallHull(legacyHull, state.hullAggregate, state.hullDestroyed),
    core_systems:         legacy.coreSystems,
    dispatch_targets:     legacy.targets,
    travel_duration_secs: 5.0,
    repair_auto:          state.controlSources?.['repair'] === 'Ai',
  });
}

/**
 * Payload contract for the Power console iframe (issue #827). Rendered by
 * gui/battleship/power.html.
 *
 * @typedef {{ consoles: Array, total: number, total_max: number,
 *             battery_charge: number, battery_max: number, draining: boolean,
 *             charging: boolean,
 *             reactor_online: boolean, battery_online: boolean,
 *             own_hull: StationHullAggregate, power_auto: boolean,
 *             station_rating: string }} PowerConsolePayload
 */

/**
 * Power console. Returns JSON of {@link PowerConsolePayload}.
 *
 * Reads raw sim truth from `state.blackboards['power']` (aggregate
 * PowerBlackboard) plus the two fine blackboards (issue #513) at
 * `state.blackboards['power-reactor']` and `state.blackboards['power-battery']`
 * for per-instance online flags. Before the first blackboard update the
 * payload renders empty defaults (there is no legacy PowerState wire path
 * any more — issue #825 removed the dead fallback).
 *
 * @param {{ blackboards? }} state
 */
export function buildPowerConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['power']) || {};
  const reactorBb = (state.blackboards && state.blackboards['power-reactor']) || null;
  const batteryBb = (state.blackboards && state.blackboards['power-battery']) || null;
  // Default to online when the fine blackboard is missing (legacy safety).
  const reactorOnline = reactorBb ? !!reactorBb.is_online : true;
  const batteryOnline = batteryBb ? !!batteryBb.is_online : true;
  return JSON.stringify({
    // Reads the PowerGroupId-keyed `groups` field from the publisher (the
    // legacy `consoles` mirror was removed from the wire when the parent
    // issue #516 cleanup closed out).
    consoles:       bb.groups        || [],
    total:          bb.total          ?? 0,
    total_max:      bb.total_max      ?? 8,
    battery_charge: bb.battery_charge ?? 0,
    battery_max:    bb.battery_max    ?? 100,
    // Which way the reserve is moving, for the battery gauge.
    draining:       bb.draining       || false,
    // NOT `!draining`. A hull can author a reactor rate of exactly zero for
    // some total, and there the reserve is frozen — neither emptying nor
    // filling. The battery bar's pulsing CHARGING indicator reads this, so a
    // parked reserve says nothing rather than promising a recovery that is
    // never going to arrive.
    charging:       bb.charging       || false,
    // The exhaustion lock (restored when issue #952's floors were reverted): the
    // battery bottomed out, every group was slammed to 1, and the +/- controls
    // are frozen until the reserve recovers past `emergency_threshold`.
    locked:         bb.locked         || false,
    reactor_online: reactorOnline,
    battery_online: batteryOnline,
    own_hull:       aggregateStationHull('power', state.consoleHull, state.stationSystems),
    power_auto:     state.stationRatings?.['power'] === 'Backfill',
    station_rating: state.stationRatings?.['power'] || 'Std',
  });
}

/**
 * Payload contract for the Shields console iframe (issue #827). Rendered by
 * gui/battleship/shields.html and, via `buildSystemStationConsoleState`,
 * every other hull's shields-owning console (e.g.
 * gui/destroyer/engineering.html).
 *
 * `combat_lock_bearing` (renamed from `target_bearing`, issue #926) and
 * `threat_bearing` are two DIFFERENT quantities: the former is this ship's
 * own frozen Combat Lock target; the latter is the standing bearing of the
 * nearest hostile in sensor range — the same fact the backfilled Shields
 * focus AI reads to override its damage-based decision. Both are verbatim
 * pass-throughs of `ShieldsBlackboard` — no client-side re-derivation.
 *
 * @typedef {{ facings: Array, hull_integrity_pct: number,
 *             focused_facing: string|null, combat_lock_bearing: number|null,
 *             threat_bearing: number|null,
 *             grid_status: string, own_hull: StationHullAggregate,
 *             shields_auto: boolean }} ShieldsConsolePayload
 */

/**
 * Shields console. Returns JSON of {@link ShieldsConsolePayload}.
 * @param {{ blackboards, shieldFacings, hullIntegrity, shieldFocusedFacing }} state
 */
export function buildShieldsConsoleState(state) {
  const bb = state.blackboards && state.blackboards['shields'];
  if (bb) {
    return JSON.stringify({
      facings:              bb.facings              ?? [],
      hull_integrity_pct:   bb.hull_integrity_pct   ?? 100,
      focused_facing:       bb.focused_facing       ?? null,
      combat_lock_bearing:  bb.combat_lock_bearing  ?? null,
      threat_bearing:       bb.threat_bearing       ?? null,
      grid_status:          bb.grid_status          ?? 'GRID NOMINAL',
      own_hull: aggregateStationHull('shields', state.consoleHull, state.stationSystems),
      shields_auto: state.stationRatings?.['shields'] === 'Backfill',
    });
  }
  // Legacy fallback: read from ShieldStatus broadcast fields. Predates the
  // ShieldsBlackboard (issue #562); `threat_bearing` has no legacy source —
  // it was never derivable client-side, and issue #926 does not add one.
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
    facings:              state.shieldFacings      || [],
    hull_integrity_pct:   state.hullIntegrity       || 100,
    focused_facing:       state.shieldFocusedFacing || null,
    combat_lock_bearing:  targetBearing,
    threat_bearing:       null,
    grid_status:          (state.shieldFacings && state.shieldFacings.length > 0)
                            ? 'GRID NOMINAL' : 'GRID OFFLINE',
    own_hull: aggregateStationHull('shields', state.consoleHull, state.stationSystems),
    shields_auto: state.stationRatings?.['shields'] === 'Backfill',
  });
}

/**
 * Payload contract for the Sensors console iframe (issue #827). Rendered by
 * gui/battleship/sensors.html.
 *
 * @typedef {{ scan_range: number, ship_x: number, ship_z: number,
 *             ship_heading: number, ship_speed: number, complexity: string,
 *             impulse_charge_progress: number, on_screen: boolean,
 *             regions: RadarRegion[], blips: RadarBlip[],
 *             target_uuid: string|null, target_name: string|null,
 *             target_kind: string|null, target_stance: string|null,
 *             target_faction: string|null, target_bearing: number|null,
 *             target_range: number|null, target_class: string|null,
 *             target_hull_pct: number|null, target_heading: number|null,
 *             target_speed: number|null, target_threat: string|null,
 *             target_shield_freq: number|null, target_shields: Array,
 *             target_shield_fraction: number|null,
 *             target_alert: boolean|null,
 *             scan: {capable: boolean, reading: object|null,
 *                    refusal: string|null},
 *             own_hull: StationHullAggregate,
 *             sensors_auto: boolean }} SensorsConsolePayload
 */

/**
 * The ship's last sensor reading (issue #1032).
 *
 * Its own blackboard under its own channel key, not a field on the sensors
 * one, for the reason {@link operationsPayload} reads from `operations`: the
 * thing aboard the ship that can be commanded and damaged is the sensors
 * system, and a reading is a result rather than a system's live state. A hull
 * that authored no `[scan]` publishes none at all, which is the empty shape
 * returned here — the panel renders its own "no capability" state off
 * `capable`, so the console never has to guess.
 * @param {{ blackboards }} state
 */
function scanPayload(state) {
  const bb = (state.blackboards && state.blackboards['scan']) || {};
  return {
    capable: bb.capable ?? false,
    reading: bb.reading ?? null,
    refusal: bb.refusal ?? null,
  };
}

/**
 * Sensors console. Returns JSON of {@link SensorsConsolePayload}.
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
      targetShieldFreq = tgt.shield_freq != null ? tgt.shield_freq : null;
      targetShields    = tgt.shields     || [];
      // Single-facing NPC shield fraction (#473). `null` for shieldless
      // entities (no [shields] block on the TOML); `0..=1` for shielded
      // NPCs; `0` for broken shields.
      targetShieldFraction = tgt.shield_fraction !== undefined && tgt.shield_fraction !== null
        ? tgt.shield_fraction
        : null;
    }
  }

  // Selected-target Red Alert (issue #749). Authoritative, read ONLY from this
  // ship's own sensor-radar blackboard — never from the per-entity snapshot —
  // so the intelligence stays confined to the Sensors scan surface. The host
  // publishes `Some(bool)` only for a Red-Alert-capable ship it has selected;
  // absent field (non-ship/incapable/no selection) reads as `null` → no row.
  const sensorRadarBb = state.blackboards?.['sensor-radar'];
  const targetAlert = sensorRadarBb?.selected_target_alert ?? null;

  // Shared target markers (tactical target + navigation waypoint)
  const shipX = state.shipX || 0, shipZ = state.shipZ || 0, shipYaw = state.shipYaw || 0;
  const tacBb = state.blackboards?.['tactical'];
  const tacMarker = buildTargetBlip(
    tacBb?.target_uuid, entities, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true, kind: 'tactical-target', color: [1.0, 0.2, 0.2], label: t('console.radar.tactical_target') }
  );
  if (tacMarker) blips.push(tacMarker);
  const waypoint = buildWaypointBlip(
    state.navigationWaypoint || null, shipX, shipZ, shipYaw, range,
    { rotate: true, edgeClamp: true }
  );
  if (waypoint) blips.push(waypoint);

  return JSON.stringify({
    scan_range:              range,
    ship_x:                  state.shipX || 0,
    ship_z:                  state.shipZ || 0,
    ship_heading:            (((state.shipYaw || 0) * 180 / Math.PI % 360) + 360) % 360,
    ship_speed:              state.forwardSpeed || 0,
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
    target_alert:       targetAlert,
    // The last scan reading (issue #1032) — a blackboard of its own, so it is
    // read from its own channel key rather than off the sensors one.
    scan:               scanPayload(state),
    own_hull: aggregateStationHull('sensors', state.consoleHull, state.stationSystems),
    sensors_auto: state.stationRatings?.['sensors'] === 'Backfill',
  });
}

/**
 * Payload contract for the Comms console iframe (issue #827). Rendered by
 * gui/battleship/comms.html.
 *
 * @typedef {{ messages: Array, objectives?: Array, contacts: Array,
 *             on_screen: boolean, own_hull: StationHullAggregate,
 *             dossiers: Array, comms_auto: boolean }} CommsConsolePayload
 */

/**
 * The crew's intelligence picture (issue #1030).
 *
 * Its own blackboard under its own channel key, read from there rather than off
 * the comms one, for the reason {@link operationsPayload} reads from `operations`:
 * a dossier is something the crew knows, not something a system aboard the ship
 * publishes. On the destroyer the Intel panel is mounted on Tactical, which is
 * independent of wherever Comms is currently hosted (issue #1098 made Comms a
 * complete visiting Station in its own right) — so `buildSystemStationConsoleState`
 * merges this onto every system-composed station's top-level `dossiers` field
 * rather than nesting it under a comms-family view, and the flat `comms.html`
 * console still gets its own copy directly from {@link buildCommsConsoleState}.
 *
 * A world with no subjects publishes an empty list rather than nothing, so the
 * panel renders its own empty state instead of the console guessing.
 * @param {{ blackboards }} state
 */
function dossiersPayload(state) {
  const bb = (state.blackboards && state.blackboards['dossiers']) || {};
  return bb.subjects ?? [];
}

/**
 * Comms console. Returns JSON of {@link CommsConsolePayload}.
 * @param {{ blackboards, commsMessages, commsContacts }} state
 */
export function buildCommsConsoleState(state) {
  const bb = state.blackboards && state.blackboards['comms'];
  // Latest host rejection of an attempted response (#761 AC3); the comms
  // component flashes the matching button red.
  const rejection = state.commsRejection ?? null;
  if (bb) {
    return JSON.stringify({
      messages:   bb.messages   ?? [],
      objectives: bb.objectives ?? [],
      contacts:   bb.contacts   ?? [],
      on_screen:  state.currentView === 'Comms',
      own_hull:   aggregateStationHull('comms', state.consoleHull, state.stationSystems),
      // Dossiers ride their own blackboard (issue #1030), so they are read from
      // their own key on BOTH arms: a comms blackboard that has not arrived says
      // nothing about whether a dossier one has.
      dossiers:   dossiersPayload(state),
      // The fine System's live source is authoritative even when its complete
      // Comms Station is visiting a differently named host (issue #1098,
      // mirroring navigation_auto below). Older snapshots may not carry
      // controlSources, so retain the lobby-rating fallback for compatibility.
      comms_auto: state.controlSources?.['comms'] != null
        ? state.controlSources['comms'] === 'Ai'
        : state.stationRatings?.['comms'] === 'Backfill',
      rejection,
    });
  }
  // Legacy fallback.
  return JSON.stringify({
    messages:  state.commsMessages || [],
    contacts:  state.commsContacts || [],
    on_screen: state.currentView === 'Comms',
    own_hull:  aggregateStationHull('comms', state.consoleHull, state.stationSystems),
    dossiers:  dossiersPayload(state),
    comms_auto: state.controlSources?.['comms'] != null
      ? state.controlSources['comms'] === 'Ai'
      : state.stationRatings?.['comms'] === 'Backfill',
    rejection,
  });
}

// ── Navigation radar range ──────────────────────────────────────────────────

export const NAVIGATION_RADAR_RANGE = 5000.0;

/**
 * Payload contract for the Navigation console iframe (issue #827). Rendered
 * by gui/battleship/navigation.html.
 *
 * @typedef {{ blips: RadarBlip[], waypoint: {x: number, z: number}|null,
 *             ship_x: number, ship_z: number, ship_heading: number,
 *             own_hull: StationHullAggregate, ship_speed: number,
 *             impulse_charge_progress: number, cancel_visible: boolean,
 *             on_screen: boolean, radar_range: number,
 *             regions: RadarRegion[], civilians: Array,
 *             navigation_auto: boolean }} NavigationConsolePayload
 */

/**
 * Navigation console state builder (issue #458). Returns JSON of
 * {@link NavigationConsolePayload}.
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
  // Filter to entities the hull author elected to show. Objective markers and
  // objective regions are purposeful chart annotations. Other objective
  // targets (including hostile ships) must not bypass the authored filter.
  const navEntities = entities.filter(e => {
    const tags = (e.tags || e.entity_tags || []).map(t => String(t).toLowerCase());
    if (tags.includes('objective_marker')) return !!e.objective_target;
    if (e.objective_target && e.region_colour) return true;
    return tags.some(t => navShowsLower.includes(t));
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
    // Civilian traffic (issue #1028): who is on which lane, and who is not
    // doing as asked. Server-derived — the client never infers compliance from
    // watching a contact move, because "it has not started turning yet" and
    // "it has decided not to" look identical on a chart.
    civilians:               (bb && bb.civilians) || [],
    // The fine System's live source is authoritative even when its complete
    // Navigation Station is visiting a differently named host. Older snapshots
    // may not carry controlSources, so retain the lobby-rating fallback for
    // compatibility rather than briefly presenting manual control as AUTO.
    navigation_auto:         state.controlSources?.['navigation'] != null
      ? state.controlSources['navigation'] === 'Ai'
      : state.stationRatings?.['navigation'] === 'Backfill',
  });
}

/**
 * Map a fine (TOML) system id to the console family that renders it, or null
 * for ids no console view covers.
 *
 * Single source of truth for system-id → console-family matching: used here
 * to pick which owned systems feed each aggregate view, and by
 * gui/dirty-consoles.js to derive which station's console a BlackboardUpdate
 * dirties. Keep the two consumers in sync by editing only this function.
 *
 * @param {string} id  fine system id (e.g. 'helm-throttle', 'phaser-bank-1')
 * @returns {string|null}  console family name ('captain', 'helm', 'tactical',
 *   'sensors', 'navigation', 'comms', 'shields', 'power', 'repair') or null
 */
export function consoleForSystemId(id) {
  if (id === 'captain' || id === 'viewscreen' || id === 'red-alert') return 'captain';
  if (id.startsWith('helm-')) return 'helm';
  if (id === 'tactical-radar' || id === 'phaser-control' || id.startsWith('phaser-')
      || id.startsWith('torpedo-') || id.startsWith('blaster-')) return 'tactical';
  if (id === 'sensors' || id === 'sensor-radar') return 'sensors';
  if (id === 'navigation') return 'navigation';
  if (id === 'comms') return 'comms';
  if (id === 'shields-system' || id.startsWith('shield-arc-')) return 'shields';
  if (id === 'power-reactor' || id === 'power-battery') return 'power';
  if (id === 'repair') return 'repair';
  if (id === 'tractor') return 'tractor';
  if (id === 'umbilical') return 'umbilical';
  return null;
}

/**
 * Tractor console family (issue #1156). Reads the raw tractor blackboard the
 * engineering-owned `tractor` system publishes under its own system id and
 * returns JSON of the small view the engineering console's tractor control
 * renders: the authored reach, whether the beam is engaged, the coupled
 * target's uuid/name id, and the `strings.csv` id of the last refusal (which the
 * console resolves through `t()` — no English crosses the wire). A hull with no
 * tractor publishes no such blackboard, so this returns the idle shape and the
 * control renders its own "no beam" state.
 * @param {{ blackboards }} state
 */
export function buildTractorConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['tractor']) || {};
  return JSON.stringify({
    range: bb.range ?? 0,
    engaged: !!bb.engaged,
    coupled_target: bb.coupled_target ?? null,
    coupled_target_name: bb.coupled_target_name ?? null,
    refusal: bb.refusal ?? null,
  });
}

/**
 * Umbilical console family (issue #1160). Reads the raw umbilical blackboard the
 * engineering-owned `umbilical` system publishes under its own system id and
 * returns JSON of the small view the engineering console's umbilical control
 * renders: the authored capacity id, rate and direction, whether the flow is
 * running, both docked ends' current levels, and the `strings.csv` id of the last
 * refusal (which the console resolves through `t()` — no English crosses the
 * wire). A hull with no umbilical publishes no such blackboard, so this returns
 * the idle shape and the control renders its own "no umbilical" state.
 * @param {{ blackboards }} state
 */
export function buildUmbilicalConsoleState(state) {
  const bb = (state.blackboards && state.blackboards['umbilical']) || {};
  return JSON.stringify({
    capacity: bb.capacity ?? null,
    rate: bb.rate ?? 0,
    direction: bb.direction ?? null,
    running: !!bb.running,
    operator_level: bb.operator_level ?? null,
    partner_level: bb.partner_level ?? null,
    refusal: bb.refusal ?? null,
  });
}

/**
 * Console family → the flat (plain, non-system-keyed) builder that produces its
 * payload. Used by buildConsoleStateInner to dispatch a single-family station by
 * the family it OWNS rather than by its station-id string (issue #925), so a
 * flat console renders correctly regardless of what the seat is named. Every
 * family here has a matching flat `gui/battleship/*.html` console.
 */
const FAMILY_BUILDERS = {
  captain: buildCaptainConsoleState,
  helm: buildHelmConsoleState,
  tactical: buildWeaponsConsoleState,
  sensors: buildSensorsConsoleState,
  navigation: buildNavigationConsoleState,
  comms: buildCommsConsoleState,
  shields: buildShieldsConsoleState,
  power: buildPowerConsoleState,
  repair: buildRepairConsoleState,
};

/**
 * Payload contract for system-composed station consoles (issues #825, #827):
 * any station whose TOML-owned fine systems span more than one console
 * family. `systems` holds one per-family view (a *ConsolePayload above)
 * under EACH owning fine-system id — a console reads
 * `s.systems['power-reactor']`, never a station-role key. Rendered by
 * gui/cruiser/{comms,engineering,science}.html,
 * gui/destroyer/{captain,engineering,tactical}.html and
 * gui/courier/{captain,pilot,tactical}.html.
 *
 * @typedef {{ station_id: string, system_ids: string[],
 *             systems: Object<string, object>,
 *             dossiers: Array }} SystemStationConsolePayload
 */

/**
 * Build a console payload from the fine systems owned by a station.
 * Returns JSON of {@link SystemStationConsolePayload}.
 *
 * Each entry in `systems` is keyed by its actual SystemId. A console therefore
 * receives a view only when its station owns the corresponding fine system;
 * the station's display role never selects data. This lets TOML move systems
 * between stations without growing another per-hull composite builder.
 *
 * Multiple fine systems can share an aggregate view (for example all helm
 * axes need the current flight state). The aggregate is deliberately copied
 * only under the ids which own it, never under a station-role key.
 *
 * @param {string} stationId
 * @param {object} state
 */
export function buildSystemStationConsoleState(stationId, state) {
  const ids = state.stationSystems?.[stationId] || [];
  const systems = {};
  const controlSources = state.controlSources || {};
  const add = (group, build, adjust) => {
    const view = JSON.parse(build(state));
    const owned = ids.filter(id => consoleForSystemId(id) === group);
    if (owned.length === 0) return;
    if (adjust) adjust(view, owned);
    owned.forEach(id => { systems[id] = view; });
  };
  const allAi = idsToCheck => idsToCheck.length > 0
    && idsToCheck.every(id => controlSources[id] === 'Ai');

  add('captain', buildCaptainConsoleState,
    (view) => {
      view.red_alert_auto = controlSources['red-alert'] === 'Ai';
      view.viewscreen_auto = controlSources['viewscreen'] === 'Ai';
    });
  add('helm', buildHelmConsoleState,
    (view, owned) => { view.helm_auto = allAi(owned); view.lateral_auto = controlSources['helm-lateral-thrust'] === 'Ai'; });
  add('tactical', buildWeaponsConsoleState,
    (view, owned) => { view.tactical_auto = allAi(owned); });
  add('sensors', buildSensorsConsoleState,
    (view, owned) => { view.sensors_auto = allAi(owned); });
  add('navigation', buildNavigationConsoleState,
    (view) => { view.navigation_auto = controlSources['navigation'] === 'Ai'; });
  add('comms', buildCommsConsoleState,
    (view) => { view.comms_auto = controlSources['comms'] === 'Ai'; });
  add('shields', buildShieldsConsoleState,
    (view) => { view.shields_auto = controlSources['shields-system'] === 'Ai'; });
  add('power', buildPowerConsoleState,
    (view) => { view.power_auto = controlSources['power-reactor'] === 'Ai'; });
  add('repair', buildRepairConsoleState,
    (view) => { view.repair_auto = controlSources['repair'] === 'Ai'; });
  add('tractor', buildTractorConsoleState,
    (view) => { view.tractor_auto = controlSources['tractor'] === 'Ai'; });
  add('umbilical', buildUmbilicalConsoleState,
    (view) => { view.umbilical_auto = controlSources['umbilical'] === 'Ai'; });

  // Dossiers (issue #1030) ride this top-level key rather than under
  // `systems['comms']`. That used to be enough because the destroyer's Intel
  // panel and its Comms system lived on the same station, but Comms is now
  // its own complete Station (issue #1098) and can visit anywhere, so a
  // system-composed station that never owns a comms-family system (Tactical,
  // once Comms left it) would otherwise lose the feed. Cross-cutting like
  // `own_hull` below, for the same reason: every system-composed station gets
  // it, whether or not it renders it.
  return JSON.stringify({
    station_id: stationId,
    system_ids: ids,
    systems,
    dossiers: dossiersPayload(state),
  });
}

// ── Window dispatch (for non-module inline scripts in client.html) ──────────

/**
 * Compute the per-station footer damage aggregate for `consoleName` and merge
 * it into the built console JSON as `own_hull`. `consoleName` is the station id
 * (issue #12), so `aggregateStationHull` gives the console operator's own
 * systems — the footer `ph-station-damage` bar reads this.
 */
function withStationDamage(consoleName, state, json) {
  try {
    const obj = JSON.parse(json);
    obj.own_hull = aggregateStationHull(consoleName, state.consoleHull, state.stationSystems);
    return JSON.stringify(obj);
  } catch (_) {
    return json;
  }
}

/**
 * Merge the contextual tutorial block (issue #916) into the built console
 * JSON as `tutorial`, evaluating this station's TOML-authored overlay
 * definitions (`state.stationTutorials`, from Welcome) against the
 * client-local progress (`state.tutorialProgress`) and the payload itself —
 * so `state`-kind triggers reference exactly the fields the console renders.
 * Cross-cutting like `withStationDamage` above: every console gets it, and
 * a station that authored no overlays gets `tutorial: null`. `consoleName`
 * also scopes every progress lookup (`<station>/<id>` keys — see
 * gui/tutorial-state.js), so stations never share dismissal state.
 */
/**
 * Where each human-seeking system is hosted right now, keyed by system id
 * (issue #984, pasm decision `console-complexity-human-seeking-systems`).
 *
 * Read off the blackboards themselves rather than from a list of system ids
 * kept here: a blackboard that carries a `host_station` IS a seeking system's
 * blackboard, so a hull (or a scenario, through the scenario-floor vocabulary)
 * that makes another system seek needs no edit on this side.
 *
 * A `null`/absent host means "nobody is hosting this", which the wire uses for
 * both "does not seek" and "seeks and found no human" — see the field's doc
 * comment in src/core/messages.rs. Both want the pre-#984 rendering, and
 * leaving the id out of this map is what produces it.
 *
 * @param {object} state  simState
 * @returns {Object<string, string>}  system id → hosting station id
 */
export function soughtSystemHosts(state) {
  const hosts = {};
  const blackboards = state.blackboards || {};
  for (const id of Object.keys(blackboards)) {
    const bb = blackboards[id];
    const host = bb && bb.host_station;
    if (typeof host === 'string' && host !== '') hosts[id] = host;
  }
  return hosts;
}

/**
 * Answer, on every console's payload, "which systems is this station holding
 * right now" — the client half of issue #984.
 *
 * Two things go on:
 *
 * **`hosted_systems`** is the station's live holding: its authored systems
 * MINUS any the seek has moved elsewhere, PLUS any the seek has parked here.
 * This is what decides whether a console offers a system's controls, and it is
 * a separate list from `systems` on purpose. A view that stops being OFFERED
 * has not stopped being READ: the destroyer's Intel panel renders dossiers out
 * of the comms view, and Intel does not move when Comms does. Hiding a button
 * is a presentation decision, so it is expressed as one.
 *
 * **`systems[<id>]`** grows a view for each VISITING system, so a console can
 * render what it has just been handed. This is cross-cutting like
 * `withStationDamage` and for the same reason: it must work on BOTH payload
 * shapes. A system-composed station already has a `systems` map and the visitor
 * joins it; a flat single-family station (the destroyer's Helm) grows one
 * holding the visitor alone, every field its own console reads untouched. "A
 * visiting system rides under `systems[id]`" is then one rule, and a console
 * asks one question whatever kind of console it is.
 *
 * With no seek information at all — a hull that authors no `human_seeking`, a
 * host too old to send `host_station`, the boot race before the first
 * blackboard — `hosted_systems` is exactly the authored list and nothing is
 * merged, which is precisely the pre-#984 rendering.
 *
 * Visiting views carry no `*_auto` flag, which is right rather than missing:
 * the seek only ever lands a system on a human-held station, so the flag would
 * be false by construction.
 *
 * @param {string} consoleName  station id
 * @param {object} state  simState
 * @param {string} json  the console payload built so far
 * @returns {string} json, with `hosted_systems` and any visiting views
 */
export function withVisitingSystems(consoleName, state, json) {
  try {
    const hosts = soughtSystemHosts(state);
    const authored = state.stationSystems?.[consoleName] || [];
    const kept = authored.filter(id => !(id in hosts) || hosts[id] === consoleName);
    // Sorted so two clients with the same state produce the same payload
    // regardless of blackboard key order.
    const visiting = Object.keys(hosts)
      .filter(id => hosts[id] === consoleName && !authored.includes(id))
      .sort();
    const obj = JSON.parse(json);
    obj.hosted_systems = kept.concat(visiting);
    if (visiting.length > 0) {
      const systems = obj.systems || {};
      for (const id of visiting) {
        const build = FAMILY_BUILDERS[consoleForSystemId(id)];
        if (build) systems[id] = JSON.parse(build(state));
      }
      obj.systems = systems;
    }
    return JSON.stringify(obj);
  } catch (_) {
    return json;
  }
}

export function withTutorialOverlay(consoleName, state, json) {
  try {
    const obj = JSON.parse(json);
    obj.tutorial = buildTutorialState(
      (state.stationTutorials || {})[consoleName] || [],
      state.tutorialProgress,
      obj,
      consoleName,
    );
    return JSON.stringify(obj);
  } catch (_) {
    return json;
  }
}

if (typeof window !== 'undefined') {
  // Station labels for the inline client.html script (lobby chips, console
  // title) — the tab-bar CONSOLE_LABEL map was deleted with the tab bar (#827).
  window.stationDisplayName = stationDisplayName;
  window.buildConsoleState = function buildConsoleState(consoleName, state) {
    const inner = buildConsoleStateInner(consoleName, state);
    // Visiting systems are merged BEFORE the tutorial pass, so a station's
    // authored `state`-kind triggers can reference a sought system's view the
    // same way they reference an owned one.
    return withTutorialOverlay(
      consoleName,
      state,
      withStationDamage(
        consoleName,
        state,
        withCommandAdvice(
          consoleName,
          state,
          withVisitingSystems(consoleName, state, inner),
        ),
      ),
    );
  };
  window.buildConsoleStateInner = function buildConsoleStateInner(consoleName, state) {
    // Post issue #618: `consoleName` is a lowercase station id (from each
    // per-console iframe's `initConsole({ name: '...' })` and from
    // `__updateConsole('...', ...)`). Pre-#618 these were PascalCase
    // Console enum names.
    //
    // Family-span rule (issue #825): a station whose TOML-owned fine systems
    // span more than one console family is a system-composed console and gets
    // the generic system-id-keyed payload. This is what lets a TOML move a
    // system between stations without any change here — no per-hull composite
    // builders remain. Single-family stations (every battleship station) keep
    // their flat plain-builder payloads.
    // AUTHORED ownership, deliberately not the seek-adjusted list (issue
    // #984): this decides the payload's SHAPE, and a console whose shape
    // changed under it mid-round would have to re-render as a different
    // console. A sought system arrives as `systems[<id>]` on whichever shape
    // the station already has — see `withVisitingSystems`.
    const owned = state.stationSystems?.[consoleName];
    if (owned) {
      const families = [...new Set(owned.map(consoleForSystemId).filter(f => f !== null))];
      if (families.length > 1) {
        return buildSystemStationConsoleState(consoleName, state);
      }
      // Single-family station: dispatch by the family it OWNS, not by its
      // station-id string (issue #925). On the battleship every single-family
      // station id equals its family name, so this is identical to the switch
      // below — but an NPC hull can name the seat anything (e.g. `engineering`
      // owning only the `power` family), and keying on the owned family gives
      // it the correct flat builder instead of the `default: '{}'` blank that a
      // station-id mismatch used to produce.
      if (families.length === 1) {
        const build = FAMILY_BUILDERS[families[0]];
        if (build) return build(state);
      }
    }
    // Pre-Welcome boot race (stationSystems not yet delivered): fall back to the
    // plain builder matching the station id ('{}' for ids with no single-family
    // builder, e.g. 'science', 'engineering' or 'pilot' — they render on the
    // next update once stationSystems arrives and the family dispatch above runs).
    switch (consoleName) {
      case 'tactical':    return buildWeaponsConsoleState(state);
      case 'captain':     return buildCaptainConsoleState(state);
      case 'helm':        return buildHelmConsoleState(state);
      case 'repair':      return buildRepairConsoleState(state);
      case 'power':       return buildPowerConsoleState(state);
      case 'shields':     return buildShieldsConsoleState(state);
      case 'sensors':     return buildSensorsConsoleState(state);
      case 'comms':       return buildCommsConsoleState(state);
      case 'navigation':  return buildNavigationConsoleState(state);
      case 'command':     return buildCommandConsoleState(state);
      default:            return '{}';
    }
  };
}
