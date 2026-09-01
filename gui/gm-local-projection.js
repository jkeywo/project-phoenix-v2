import { createEntityInspector } from './entity-inspector.js';

/** Peer-local omniscient ship-map presenter for the browser GM (#1295). */

const ENTITY_KINDS = new Set(['player_ship', 'npc_ship']);

function normaliseReference(value) {
  if (!value || typeof value !== 'object'
      || typeof value.entity_id !== 'string' || value.entity_id.length === 0
      || typeof value.name !== 'string') return undefined;
  return { entity_id: value.entity_id, name: value.name };
}

function normaliseEntity(value) {
  const status = value && value.status;
  if (!value || typeof value !== 'object'
      || typeof value.entity_id !== 'string' || value.entity_id.length === 0
      || typeof value.name !== 'string'
      || !ENTITY_KINDS.has(value.kind)
      || !Array.isArray(value.position) || value.position.length !== 3
      || !value.position.every(Number.isFinite)
      || !status || typeof status !== 'object'
      || !Number.isInteger(status.hull_percent)
      || status.hull_percent < 0 || status.hull_percent > 100
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
      hull_percent: status.hull_percent,
      destroyed: status.destroyed,
    },
    current_target: currentTarget,
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
  const extent = entities.reduce((largest, entity) => Math.max(
    largest,
    Math.abs(entity.position[0]),
    Math.abs(entity.position[2]),
  ), 1);
  return {
    interaction: 'inspect',
    show_ship_marker: false,
    range: extent * 1.2,
    regions: [],
    blips: entities.map((entity) => ({
      uuid: entity.entity_id,
      kind: entity.kind,
      name: entity.name,
      world_x: entity.position[0],
      world_z: entity.position[2],
      stance: entity.kind === 'player_ship' ? 'friendly' : 'unknown',
      destroyed: entity.status.destroyed,
      hull_percent: entity.status.hull_percent,
    })),
  };
}

export function createGmLocalProjection({ doc = document, t = (id) => id } = {}) {
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

  function setSelected(id) {
    const next = id == null ? null : entities.find((entity) => entity.entity_id === id) || null;
    selectedId = next ? next.entity_id : null;
    inspector.render(next);
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
    inspector.render(selectedEntity());
    return true;
  }

  function clear() {
    return update({ entities: [] });
  }

  return {
    update,
    clear,
    select,
    state: () => ({ entities: [...entities], selectedId }),
  };
}
