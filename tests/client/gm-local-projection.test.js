// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  buildGmMapState,
  createGmLocalProjection,
  parseGmEntityProjection,
} from '../../gui/gm-local-projection.js';
import '../../gui/components/ph-navigation-map.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');

const PLAYER_ID = '00000000-0000-4000-8000-000000000001';
const NPC_ID = '00000000-0000-4000-8000-000000000002';
const REGION_ID = '00000000-0000-4000-8000-000000000003';
const FIELD_ID = '00000000-0000-4000-8000-000000000004';

function entity(overrides = {}) {
  return {
    entity_id: PLAYER_ID,
    name: 'Axiom',
    kind: 'player_ship',
    position: [10, 2, -30],
    faction: {
      entity_id: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
      name: 'Alliance',
    },
    status: {
      hull_percent: 73,
      condition_percent: null,
      destroyed: false,
      hull_current_milli_hp: 73_000,
      hull_max_milli_hp: 100_000,
    },
    current_target: null,
    geometry: null,
    radar: {
      icon: 'playerShip',
      colour: [0.2, 0.8, 1],
      size: 4,
      region_colour: null,
    },
    ...overrides,
  };
}

function region(overrides = {}) {
  return entity({
    entity_id: REGION_ID,
    name: 'world.region.safe_harbour.display_name',
    kind: 'region',
    position: [100, 0, -20],
    faction: null,
    status: {
      hull_percent: null,
      condition_percent: null,
      destroyed: false,
      hull_current_milli_hp: null,
      hull_max_milli_hp: null,
    },
    geometry: { type: 'sphere', radius: 30 },
    radar: { icon: null, colour: null, size: null, region_colour: [0.1, 0.7, 0.5] },
    ...overrides,
  });
}

function payload(entities) {
  return JSON.stringify({ entities });
}

function mount({ onSelectionChanged } = {}) {
  document.body.innerHTML = `
    <p id="gm-entity-pending"></p>
    <div id="gm-entity-map"></div>
    <p id="gm-inspector-empty"></p>
    <article id="gm-entity-card" hidden>
      <h3 id="gm-entity-name"></h3>
      <span id="gm-entity-identity"></span>
      <span id="gm-entity-kind"></span>
      <span id="gm-entity-position"></span>
      <span id="gm-entity-faction"></span>
      <span id="gm-entity-status"></span>
      <meter id="gm-entity-hull" min="0" max="100"></meter>
      <button id="gm-entity-target"></button>
    </article>`;
  const map = document.getElementById('gm-entity-map');
  let mapState = null;
  let selected = null;
  Object.defineProperty(map, 'state', {
    configurable: true,
    get: () => mapState,
    set: (value) => { mapState = value; },
  });
  map.navigationSelectedUuid = () => selected;
  map.navigationSelect = vi.fn(({ uuid }) => {
    const candidate = [...mapState.blips, ...mapState.regions]
      .find((entry) => entry.uuid === uuid) || null;
    if (uuid != null && !candidate) return false;
    selected = candidate && candidate.uuid || null;
    map.dispatchEvent(new CustomEvent('navselect', { detail: candidate }));
    return true;
  });
  const projection = createGmLocalProjection({
    doc: document,
    t: (id, params = {}) => `${id}:${Object.values(params).join('/')}`,
    ...(onSelectionChanged ? { onSelectionChanged } : {}),
  });
  return { map, projection, getMapState: () => mapState };
}

describe('GM omniscient local projection', () => {
  let harness;

  beforeEach(() => { harness = mount(); });

  it('carries the absolute hull totals a direct-effect preview needs', () => {
    const parsed = parseGmEntityProjection({ entities: [entity()] });
    expect(parsed[0].status.hull_current_milli_hp).toBe(73_000);
    expect(parsed[0].status.hull_max_milli_hp).toBe(100_000);
    // Present exactly when the percentage is: a payload carrying one without
    // the other would leave a damage control unable to preview its lethality.
    expect(parseGmEntityProjection({
      entities: [entity({ status: {
        hull_percent: 73,
        condition_percent: null,
        destroyed: false,
        hull_current_milli_hp: null,
        hull_max_milli_hp: null,
      } })],
    })).toBeUndefined();
    expect(parseGmEntityProjection({
      entities: [entity({ status: {
        hull_percent: 73,
        condition_percent: null,
        destroyed: false,
        hull_current_milli_hp: -1,
        hull_max_milli_hp: 100_000,
      } })],
    })).toBeUndefined();
  });

  it('carries the per-System breakdown a scoped effect is picked from', () => {
    const systems = [
      {
        system_id: 'impulse-drive',
        station_id: 'helm',
        station_name: 'station.helm.name',
        name: 'system_hull.impulse_drive.display_name',
        current_milli_hp: 10_000,
        max_milli_hp: 40_000,
      },
      {
        system_id: 'core',
        station_id: null,
        station_name: null,
        name: 'system_hull.core.display_name',
        current_milli_hp: 32_000,
        max_milli_hp: 32_000,
      },
    ];
    const parsed = parseGmEntityProjection({
      entities: [entity({ status: { ...entity().status, systems } })],
    });
    expect(parsed[0].status.systems).toEqual(systems);
    // The owner's display name is optional on the wire — Rust omits it for an
    // unowned System and for a hull with no ship config — and absent reads as
    // `null`, which is what makes the picker fall back to the authoring key.
    expect(parseGmEntityProjection({
      entities: [entity({
        status: {
          ...entity().status,
          systems: [{ ...systems[0], station_name: undefined }],
        },
      })],
    })[0].status.systems[0].station_name).toBeNull();
    // An entity with no hull carries no breakdown, and the omission is empty
    // rather than missing — the shape every pre-#1311 payload already had.
    expect(parseGmEntityProjection({ entities: [region()] })[0].status.systems).toEqual([]);
    // A PRESENT but malformed breakdown rejects the whole entity: a Station
    // scope that quietly covered less of the ship than the picker claimed is
    // worse than no picker at all.
    for (const broken of [
      [{ ...systems[0], system_id: '' }],
      [{ ...systems[0], station_id: '' }],
      [{ ...systems[0], station_name: '' }],
      [{ ...systems[0], station_name: 7 }],
      [{ ...systems[0], current_milli_hp: -1 }],
      [{ ...systems[0], max_milli_hp: 1.5 }],
      [{ ...systems[0], name: 4 }],
      'helm',
    ]) {
      expect(parseGmEntityProjection({
        entities: [entity({ status: { ...entity().status, systems: broken } })],
      })).toBeUndefined();
    }
  });

  it('announces the current selection to surfaces that act on it', () => {
    const onSelectionChanged = vi.fn();
    harness = mount({ onSelectionChanged });
    harness.projection.update(payload([entity()]));
    expect(onSelectionChanged).toHaveBeenLastCalledWith(null);

    harness.projection.select(PLAYER_ID);
    expect(onSelectionChanged).toHaveBeenLastCalledWith(
      expect.objectContaining({ entity_id: PLAYER_ID }),
    );

    // An entity leaving the world clears the selection through the same seam,
    // so no surface can keep aiming at something that is gone.
    harness.projection.update(payload([]));
    expect(onSelectionChanged).toHaveBeenLastCalledWith(null);
  });

  it('strictly normalises the public map DTO and drops unknown detail', () => {
    const parsed = parseGmEntityProjection({
      entities: [entity({
        raw_components: ['Transform', 'Ship'],
        effect_tuning: { dps: 9000 },
        layer_path: 'assets/worlds/secret.toml',
      })],
      ecs_world: { entities: 99 },
    });
    // The per-System breakdown is absent from this payload and normalises to an
    // empty list, so the omission is the entity's own answer rather than a hole.
    expect(parsed).toEqual([entity({ status: { ...entity().status, systems: [] } })]);
    expect(parsed[0]).not.toHaveProperty('raw_components');
    expect(parsed[0]).not.toHaveProperty('effect_tuning');
    expect(parsed[0]).not.toHaveProperty('layer_path');
    expect(parseGmEntityProjection({ entities: [entity(), entity()] })).toBeUndefined();
    expect(parseGmEntityProjection({ entities: [entity({ position: [0, NaN, 2] })] }))
      .toBeUndefined();
    expect(parseGmEntityProjection({ entities: [region({ geometry: null })] })).toBeUndefined();
    expect(parseGmEntityProjection({
      entities: [region({ geometry: { type: 'torus', inner_radius: 50, outer_radius: 10 } })],
    })).toBeUndefined();
    expect(parseGmEntityProjection({
      entities: [entity({ radar: { ...entity().radar, colour: [2, 0, 0] } })],
    })).toBeUndefined();
  });

  it('exposes exact live-identity containment for cross-channel selection links', () => {
    harness.projection.update(payload([entity()]));
    expect(harness.projection.contains(PLAYER_ID)).toBe(true);
    expect(harness.projection.contains(NPC_ID)).toBe(false);
    harness.projection.update(payload([]));
    expect(harness.projection.contains(PLAYER_ID)).toBe(false);
  });

  it('projects player/NPC map markers with non-colour kind and destroyed state', () => {
    const npc = entity({
      entity_id: NPC_ID,
      name: 'Raider',
      kind: 'npc_ship',
      position: [100, 0, 40],
      status: {
        hull_percent: 0,
        condition_percent: null,
        destroyed: true,
        hull_current_milli_hp: 0,
        hull_max_milli_hp: 100_000,
      },
    });
    const state = buildGmMapState([entity(), npc]);
    expect(state).toMatchObject({ interaction: 'inspect', show_ship_marker: false });
    expect(state.blips).toEqual([
      expect.objectContaining({ uuid: PLAYER_ID, kind: 'player_ship', destroyed: false }),
      expect.objectContaining({ uuid: NPC_ID, kind: 'npc_ship', destroyed: true }),
    ]);
    expect(state.range).toBeGreaterThan(100);
  });

  it('partitions geometry into selectable Regions without duplicate blips and frames full extents', () => {
    const hazard = region({
      entity_id: '00000000-0000-4000-8000-000000000005',
      kind: 'hazard',
      position: [200, 0, 100],
      geometry: { type: 'box', half_extents: [20, 5, 40], yaw: Math.PI / 2 },
      radar: { icon: null, colour: null, size: null, region_colour: [1, 0.3, 0.1] },
    });
    const field = region({
      entity_id: FIELD_ID,
      kind: 'asteroid_field',
      position: [-400, 0, 0],
      geometry: { type: 'torus', inner_radius: 25, outer_radius: 100 },
      radar: { icon: null, colour: null, size: null, region_colour: [0.5, 0.5, 0.5] },
    });
    const structure = entity({
      entity_id: '00000000-0000-4000-8000-000000000006',
      kind: 'structure',
      status: {
        hull_percent: 80,
        condition_percent: 64,
        destroyed: false,
        hull_current_milli_hp: 80_000,
        hull_max_milli_hp: 100_000,
      },
      radar: { icon: 'station', colour: [0.3, 0.6, 0.9], size: 12, region_colour: null },
    });
    const state = buildGmMapState([entity(), structure, hazard, field]);
    expect(state.blips.map((entry) => entry.uuid)).toEqual([PLAYER_ID, structure.entity_id]);
    expect(state.regions.map((entry) => entry.uuid)).toEqual([hazard.entity_id, FIELD_ID]);
    expect(state.regions).toEqual(expect.arrayContaining([
      expect.objectContaining({
        uuid: hazard.entity_id,
        kind: 'hazard',
        selectable: true,
        shape: 'box',
        half_extents: [20, 40],
      }),
      expect.objectContaining({
        uuid: FIELD_ID,
        kind: 'asteroid_field',
        shape: 'torus',
        inner_radius: 25,
        outer_radius: 100,
      }),
    ]));
    expect(state.blips[1]).toMatchObject({
      kind: 'structure',
      icon: 'station',
      color: [0.3, 0.6, 0.9],
      radar_size: 12,
    });
    expect(state.range).toBe(600);
  });

  it('keeps selection by stable identity while absolute position and status refresh', () => {
    expect(harness.projection.update(payload([entity()]))).toBe(true);
    expect(harness.projection.select(PLAYER_ID)).toBe(true);
    expect(harness.projection.state().selectedId).toBe(PLAYER_ID);
    expect(document.getElementById('gm-entity-card').dataset.entityId).toBe(PLAYER_ID);

    harness.projection.update(payload([entity({
      position: [25, 3, -12],
      status: {
        hull_percent: 41,
        condition_percent: null,
        destroyed: false,
        hull_current_milli_hp: 41_000,
        hull_max_milli_hp: 100_000,
      },
    })]));
    expect(harness.projection.state().selectedId).toBe(PLAYER_ID);
    expect(harness.getMapState().blips[0]).toMatchObject({ world_x: 25, world_z: -12 });
    expect(document.getElementById('gm-entity-hull').value).toBe(41);
    expect(document.getElementById('gm-entity-position').textContent)
      .toContain('25.0/3.0/-12.0');
  });

  it('selects a Region in the shared inspector and hides an inapplicable hull meter', () => {
    harness.projection.update(payload([region()]));
    expect(harness.projection.select(REGION_ID)).toBe(true);
    expect(harness.projection.state().selectedId).toBe(REGION_ID);
    expect(document.getElementById('gm-entity-card').dataset.kind).toBe('region');
    expect(document.getElementById('gm-entity-hull').hidden).toBe(true);
    expect(document.getElementById('gm-entity-status').textContent)
      .toContain('server.gm.entity.active');
  });

  it('clears selection and stale inspector detail when the selected ship is removed', () => {
    harness.projection.update(payload([entity()]));
    harness.projection.select(PLAYER_ID);
    harness.projection.update(payload([]));
    expect(harness.projection.state()).toEqual({ entities: [], selectedId: null });
    expect(document.getElementById('gm-entity-card').hidden).toBe(true);
    expect(document.getElementById('gm-entity-identity').textContent).toBe('');
    expect(document.getElementById('gm-entity-map').hidden).toBe(true);
  });

  it('synchronously reconciles removal and reappearance of a selected Region on a real map element', () => {
    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    const originalRequestAnimationFrame = window.requestAnimationFrame;
    const originalCancelAnimationFrame = window.cancelAnimationFrame;
    const originalResizeObserver = window.ResizeObserver;
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
    window.requestAnimationFrame = vi.fn(() => 1);
    window.cancelAnimationFrame = vi.fn();
    window.ResizeObserver = class {
      observe() {}
      disconnect() {}
    };

    try {
      document.body.innerHTML = `
        <p id="gm-entity-pending"></p>
        <ph-navigation-map id="gm-entity-map"></ph-navigation-map>
        <p id="gm-inspector-empty"></p>
        <article id="gm-entity-card" hidden>
          <h3 id="gm-entity-name"></h3>
          <span id="gm-entity-identity"></span>
          <span id="gm-entity-kind"></span>
          <span id="gm-entity-position"></span>
          <span id="gm-entity-faction"></span>
          <span id="gm-entity-status"></span>
          <meter id="gm-entity-hull" min="0" max="100"></meter>
          <button id="gm-entity-target"></button>
        </article>`;
      const map = document.getElementById('gm-entity-map');
      const canvas = map.shadowRoot.querySelector('canvas');
      canvas.width = 0;
      canvas.height = 0;
      const projection = createGmLocalProjection({ doc: document });

      projection.update(payload([region()]));
      expect(projection.select(REGION_ID)).toBe(true);
      expect(projection.state().selectedId).toBe(REGION_ID);
      expect(map.navigationSelectedUuid()).toBe(REGION_ID);
      expect(map.dataset.selectedEntityId).toBe(REGION_ID);
      expect(map.hasAttribute('data-has-selection')).toBe(true);

      projection.update(payload([]));
      expect(map.hidden).toBe(true);
      expect(projection.state().selectedId).toBeNull();
      expect(map.navigationSelectedUuid()).toBeNull();
      expect(map.dataset.selectedEntityId).toBeUndefined();
      expect(map.hasAttribute('data-has-selection')).toBe(false);

      projection.update(payload([region({ position: [140, 0, -20] })]));
      expect(projection.state().selectedId).toBeNull();
      expect(map.navigationSelectedUuid()).toBeNull();
      expect(projection.select(REGION_ID)).toBe(true);
      expect(projection.state().selectedId).toBe(REGION_ID);
      expect(map.navigationSelectedUuid()).toBe(REGION_ID);
      expect(map.dataset.selectedEntityId).toBe(REGION_ID);
    } finally {
      document.body.innerHTML = '';
      HTMLCanvasElement.prototype.getContext = originalGetContext;
      window.requestAnimationFrame = originalRequestAnimationFrame;
      window.cancelAnimationFrame = originalCancelAnimationFrame;
      window.ResizeObserver = originalResizeObserver;
    }
  });

  it('links an inspector target back to map selection by stable identity', () => {
    const player = entity({
      current_target: { entity_id: NPC_ID, name: 'Raider' },
    });
    const npc = entity({ entity_id: NPC_ID, name: 'Raider', kind: 'npc_ship' });
    harness.projection.update(payload([player, npc]));
    harness.projection.select(PLAYER_ID);
    const target = document.getElementById('gm-entity-target');
    expect(target.dataset.targetId).toBe(NPC_ID);
    target.click();
    expect(harness.projection.state().selectedId).toBe(NPC_ID);
    expect(document.getElementById('gm-entity-name').textContent).toBe('Raider');
  });

  it('localises inspector entity, faction, and current-target display IDs only at render', () => {
    const targetName = 'entity.ship_harrow_destroyer.display_name';
    const player = entity({
      name: 'entity.alliance_destroyer.display_name',
      faction: {
        entity_id: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
        name: 'faction.alliance.display_name',
      },
      current_target: { entity_id: NPC_ID, name: targetName },
    });
    const npc = entity({ entity_id: NPC_ID, name: targetName, kind: 'npc_ship' });
    harness.projection.update(payload([player, npc]));
    harness.projection.select(PLAYER_ID);

    expect(document.getElementById('gm-entity-name').textContent)
      .toBe(t('entity.alliance_destroyer.display_name'));
    expect(document.getElementById('gm-entity-faction').textContent)
      .toBe(t('faction.alliance.display_name'));
    const target = document.getElementById('gm-entity-target');
    expect(target.textContent).toBe(t(targetName));
    expect(target.dataset.targetId).toBe(NPC_ID);
    expect(document.getElementById('gm-entity-identity').textContent).toBe(PLAYER_ID);

    target.click();
    expect(harness.projection.state().selectedId).toBe(NPC_ID);
    expect(document.getElementById('gm-entity-name').textContent).toBe(t(targetName));
    expect(document.getElementById('gm-entity-identity').textContent).toBe(NPC_ID);
  });

  it('rejects malformed payloads without changing the current map or inspector', () => {
    harness.projection.update(payload([entity()]));
    harness.projection.select(PLAYER_ID);
    expect(harness.projection.update('{bad')).toBe(false);
    expect(harness.projection.state().selectedId).toBe(PLAYER_ID);
    expect(harness.getMapState().blips).toHaveLength(1);
  });
});

describe('GM projection transport separation', () => {
  it('exists only in the Host Channel, never in peer or lockstep vocabularies', () => {
    for (const file of [
      'src/core/messages.rs',
      'src/lockstep/frame.rs',
      'src/server_app/components.rs',
    ]) {
      const source = read(file);
      expect(source).not.toContain('GmEntityProjection');
      expect(source).not.toContain('gm_entity');
    }

    const bridge = read('src/server/bridge.rs');
    const outbound = bridge.slice(
      bridge.indexOf('fn flush_outbound('),
      bridge.indexOf('fn flush_host_channels('),
    );
    expect(outbound).not.toContain('GmEntityProjection');
    expect(outbound).not.toContain('GM_ENTITY');
    expect(outbound).not.toContain('gm_entity');
    expect(bridge).toContain('host_channels::GM_ENTITY');
    expect(bridge).toContain('HOST_CHANNEL_CB');
  });
});
