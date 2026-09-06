import { createEntityInspector } from './entity-inspector.js';

/** Peer-local omniscient world-map presenter for the browser GM (#1295/#1296). */

const ENTITY_KINDS = new Set([
  'player_ship',
  'npc_ship',
  'structure',
  'hazard',
  'region',
  'asteroid_field',
  'authored_asteroid',
]);
const REGION_KINDS = new Set(['hazard', 'region', 'asteroid_field']);

function normaliseMilliHp(value) {
  if (value === null) return null;
  if (!Number.isSafeInteger(value) || value < 0) return undefined;
  return value;
}

function normalisePercent(value) {
  if (value === null) return null;
  if (!Number.isInteger(value) || value < 0 || value > 100) return undefined;
  return value;
}

function normaliseColour(value) {
  if (value === null) return null;
  if (!Array.isArray(value) || value.length !== 3
      || !value.every((channel) => Number.isFinite(channel) && channel >= 0 && channel <= 1)) {
    return undefined;
  }
  return [...value];
}

function normaliseRadar(value) {
  if (!value || typeof value !== 'object') return undefined;
  const icon = value.icon === null ? null
    : typeof value.icon === 'string' && value.icon.length > 0 ? value.icon : undefined;
  const colour = normaliseColour(value.colour);
  const regionColour = normaliseColour(value.region_colour);
  const size = value.size === null ? null
    : Number.isFinite(value.size) && value.size >= 0 ? value.size : undefined;
  if (icon === undefined || colour === undefined || size === undefined || regionColour === undefined) {
    return undefined;
  }
  return { icon, colour, size, region_colour: regionColour };
}

function normaliseGeometry(value) {
  if (value === null) return null;
  if (!value || typeof value !== 'object' || typeof value.type !== 'string') return undefined;
  if (value.type === 'sphere') {
    if (!Number.isFinite(value.radius) || value.radius < 0) return undefined;
    return { type: 'sphere', radius: value.radius };
  }
  if (value.type === 'torus') {
    if (!Number.isFinite(value.inner_radius) || value.inner_radius < 0
        || !Number.isFinite(value.outer_radius) || value.outer_radius < value.inner_radius) {
      return undefined;
    }
    return {
      type: 'torus',
      inner_radius: value.inner_radius,
      outer_radius: value.outer_radius,
    };
  }
  if (value.type === 'box') {
    if (!Array.isArray(value.half_extents) || value.half_extents.length !== 3
        || !value.half_extents.every((extent) => Number.isFinite(extent) && extent >= 0)
        || !Number.isFinite(value.yaw)) return undefined;
    return { type: 'box', half_extents: [...value.half_extents], yaw: value.yaw };
  }
  return undefined;
}

function normaliseReference(value) {
  if (!value || typeof value !== 'object'
      || typeof value.entity_id !== 'string' || value.entity_id.length === 0
      || typeof value.name !== 'string') return undefined;
  return { entity_id: value.entity_id, name: value.name };
}

function normaliseEntity(value) {
  const status = value && value.status;
  const hullPercent = status && normalisePercent(status.hull_percent);
  const conditionPercent = status && normalisePercent(status.condition_percent);
  // Absolute hull totals (issue #1310). Present exactly when the percentage
  // is, so a payload that carries one and not the other is rejected outright
  // rather than leaving a damage control unable to preview its own lethality.
  const hullCurrent = status && normaliseMilliHp(status.hull_current_milli_hp);
  const hullMax = status && normaliseMilliHp(status.hull_max_milli_hp);
  const geometry = normaliseGeometry(value && value.geometry);
  const radar = normaliseRadar(value && value.radar);
  if (!value || typeof value !== 'object'
      || typeof value.entity_id !== 'string' || value.entity_id.length === 0
      || typeof value.name !== 'string'
      || !ENTITY_KINDS.has(value.kind)
      || !Array.isArray(value.position) || value.position.length !== 3
      || !value.position.every(Number.isFinite)
      || !status || typeof status !== 'object'
      || hullPercent === undefined
      || conditionPercent === undefined
      || hullCurrent === undefined
      || hullMax === undefined
      || (hullPercent === null) !== (hullCurrent === null)
      || (hullPercent === null) !== (hullMax === null)
      || geometry === undefined
      || radar === undefined
      || REGION_KINDS.has(value.kind) !== (geometry !== null)
      || typeof status.destroyed !== 'boolean') return undefined;
  const faction = value.faction === null ? null : normaliseReference(value.faction);
  const currentTarget = value.current_target === null
    ? null : normaliseReference(value.current_target);
  if (faction === undefined || currentTarget === undefined) return undefined;
  // Pick the public DTO fields explicitly. A future producer adding detail
  // cannot accidentally turn this M1/M6 shell into a raw inspector.
  return {
    entity_id: value.entity_id,
    name: value.name,
    kind: value.kind,
    position: [...value.position],
    faction,
    status: {
      hull_percent: hullPercent,
      condition_percent: conditionPercent,
      destroyed: status.destroyed,
      hull_current_milli_hp: hullCurrent,
      hull_max_milli_hp: hullMax,
    },
    current_target: currentTarget,
    geometry,
    radar,
  };
}

export function parseGmEntityProjection(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object' || !Array.isArray(value.entities)) return undefined;
  const seen = new Set();
  const entities = [];
  for (const source of value.entities) {
    const entity = normaliseEntity(source);
    if (!entity || seen.has(entity.entity_id)) return undefined;
    seen.add(entity.entity_id);
    entities.push(entity);
  }
  entities.sort((left, right) => left.entity_id.localeCompare(right.entity_id));
  return entities;
}

export function buildGmMapState(entities) {
  const extent = entities.reduce((largest, entity) => {
    let extentX = entity.radar.size || 0;
    let extentZ = entity.radar.size || 0;
    if (entity.geometry && entity.geometry.type === 'sphere') {
      extentX = entity.geometry.radius;
      extentZ = entity.geometry.radius;
    } else if (entity.geometry && entity.geometry.type === 'torus') {
      extentX = entity.geometry.outer_radius;
      extentZ = entity.geometry.outer_radius;
    } else if (entity.geometry && entity.geometry.type === 'box') {
      const [halfX, , halfZ] = entity.geometry.half_extents;
      const sinYaw = Math.abs(Math.sin(entity.geometry.yaw));
      const cosYaw = Math.abs(Math.cos(entity.geometry.yaw));
      extentX = halfX * cosYaw + halfZ * sinYaw;
      extentZ = halfX * sinYaw + halfZ * cosYaw;
    }
    return Math.max(
      largest,
      Math.abs(entity.position[0]) + extentX,
      Math.abs(entity.position[2]) + extentZ,
    );
  }, 1);
  const regions = entities.filter((entity) => entity.geometry !== null).map((entity) => ({
    uuid: entity.entity_id,
    kind: entity.kind,
    name: entity.name,
    x: entity.position[0],
    z: entity.position[2],
    selectable: true,
    color: entity.radar.region_colour,
    shape: entity.geometry.type,
    radius: entity.geometry.type === 'sphere' ? entity.geometry.radius : null,
    inner_radius: entity.geometry.type === 'torus' ? entity.geometry.inner_radius : null,
    outer_radius: entity.geometry.type === 'torus' ? entity.geometry.outer_radius : null,
    half_extents: entity.geometry.type === 'box'
      ? [entity.geometry.half_extents[0], entity.geometry.half_extents[2]] : null,
    yaw: entity.geometry.type === 'box' ? entity.geometry.yaw : null,
  }));
  const blips = entities.filter((entity) => entity.geometry === null).map((entity) => ({
    uuid: entity.entity_id,
    kind: entity.kind,
    name: entity.name,
    world_x: entity.position[0],
    world_z: entity.position[2],
    stance: entity.kind === 'player_ship' ? 'friendly' : 'unknown',
    destroyed: entity.status.destroyed,
    hull_percent: entity.status.hull_percent,
    condition_percent: entity.status.condition_percent,
    icon: entity.radar.icon,
    color: entity.radar.colour,
    radar_size: entity.radar.size,
  }));
  return {
    interaction: 'inspect',
    show_ship_marker: false,
    range: extent * 1.2,
    regions,
    blips,
  };
}

export function createGmLocalProjection({
  doc = document,
  t = (id) => id,
  // Absolute selection push for surfaces that act ON the selected entity
  // (issue #1310). It fires wherever the inspector re-renders — a click, a
  // projection refresh, an entity leaving the world — so a consumer never has
  // to poll `state()` or listen to the map component itself.
  onSelectionChanged = () => {},
} = {}) {
  const pending = doc.getElementById('gm-entity-pending');
  const map = doc.getElementById('gm-entity-map');
  let entities = [];
  let selectedId = null;

  const inspector = createEntityInspector({
    doc,
    t,
    onSelect: (id) => select(id),
  });

  function selectedEntity() {
    return entities.find((entity) => entity.entity_id === selectedId) || null;
  }

  function announceSelection(entity) {
    try {
      onSelectionChanged(entity);
    } catch (_) {
      // A consumer that throws must not take the map down with it.
    }
  }

  function setSelected(id) {
    const next = id == null ? null : entities.find((entity) => entity.entity_id === id) || null;
    selectedId = next ? next.entity_id : null;
    inspector.render(next);
    announceSelection(next);
    return id == null || !!next;
  }

  function select(id) {
    if (id != null && !entities.some((entity) => entity.entity_id === id)) return false;
    if (map && typeof map.navigationSelectedUuid === 'function'
        && typeof map.navigationSelect === 'function'
        && map.navigationSelectedUuid() !== id) {
      return map.navigationSelect({ uuid: id });
    }
    return setSelected(id);
  }

  if (map) {
    map.addEventListener('navselect', (event) => {
      setSelected(event.detail && event.detail.uuid || null);
    });
  }

  function update(payload) {
    const next = parseGmEntityProjection(payload);
    if (next === undefined) return false;
    const selectionRemoved = selectedId !== null
      && !next.some((entity) => entity.entity_id === selectedId);
    entities = next;
    if (pending) pending.hidden = entities.length > 0;
    if (map) {
      map.hidden = entities.length === 0;
      map.state = buildGmMapState(entities);
    }
    if (selectionRemoved) {
      selectedId = null;
      // Rendering is deferred and a hidden/zero-size canvas does not reconcile
      // its retained private blip. Clear through the component's semantic seam
      // now so its UUID and observable attributes cannot outlive the entity.
      if (map && typeof map.navigationSelect === 'function') {
        map.navigationSelect({ uuid: null });
      }
    }
    const current = selectedEntity();
    inspector.render(current);
    announceSelection(current);
    return true;
  }

  function clear() {
    return update({ entities: [] });
  }

  return {
    update,
    clear,
    select,
    contains: (id) => entities.some((entity) => entity.entity_id === id),
    state: () => ({ entities: [...entities], selectedId }),
  };
}
