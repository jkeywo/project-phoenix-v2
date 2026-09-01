// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
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
    status: { hull_percent: 73, destroyed: false },
    current_target: null,
    ...overrides,
  };
}

function payload(entities) {
  return JSON.stringify({ entities });
}

function mount() {
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
    const blip = mapState.blips.find((entry) => entry.uuid === uuid) || null;
    if (uuid != null && !blip) return false;
    selected = blip && blip.uuid || null;
    map.dispatchEvent(new CustomEvent('navselect', { detail: blip }));
    return true;
  });
  const projection = createGmLocalProjection({
    doc: document,
    t: (id, params = {}) => `${id}:${Object.values(params).join('/')}`,
  });
  return { map, projection, getMapState: () => mapState };
}

describe('GM omniscient local projection', () => {
  let harness;

  beforeEach(() => { harness = mount(); });

  it('strictly normalises the public map DTO and drops unknown detail', () => {
    const parsed = parseGmEntityProjection({
      entities: [entity({ raw_components: ['Transform', 'Ship'] })],
      ecs_world: { entities: 99 },
    });
    expect(parsed).toEqual([entity()]);
    expect(parsed[0]).not.toHaveProperty('raw_components');
    expect(parseGmEntityProjection({ entities: [entity(), entity()] })).toBeUndefined();
    expect(parseGmEntityProjection({ entities: [entity({ position: [0, NaN, 2] })] }))
      .toBeUndefined();
  });

  it('projects player/NPC map markers with non-colour kind and destroyed state', () => {
    const npc = entity({
      entity_id: NPC_ID,
      name: 'Raider',
      kind: 'npc_ship',
      position: [100, 0, 40],
      status: { hull_percent: 0, destroyed: true },
    });
    const state = buildGmMapState([entity(), npc]);
    expect(state).toMatchObject({ interaction: 'inspect', show_ship_marker: false });
    expect(state.blips).toEqual([
      expect.objectContaining({ uuid: PLAYER_ID, kind: 'player_ship', destroyed: false }),
      expect.objectContaining({ uuid: NPC_ID, kind: 'npc_ship', destroyed: true }),
    ]);
    expect(state.range).toBeGreaterThan(100);
  });

  it('keeps selection by stable identity while absolute position and status refresh', () => {
    expect(harness.projection.update(payload([entity()]))).toBe(true);
    expect(harness.projection.select(PLAYER_ID)).toBe(true);
    expect(harness.projection.state().selectedId).toBe(PLAYER_ID);
    expect(document.getElementById('gm-entity-card').dataset.entityId).toBe(PLAYER_ID);

    harness.projection.update(payload([entity({
      position: [25, 3, -12],
      status: { hull_percent: 41, destroyed: false },
    })]));
    expect(harness.projection.state().selectedId).toBe(PLAYER_ID);
    expect(harness.getMapState().blips[0]).toMatchObject({ world_x: 25, world_z: -12 });
    expect(document.getElementById('gm-entity-hull').value).toBe(41);
    expect(document.getElementById('gm-entity-position').textContent)
      .toContain('25.0/3.0/-12.0');
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

  it('synchronously clears a real map element when its sole selected ship disappears', () => {
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

      projection.update(payload([entity()]));
      expect(projection.select(PLAYER_ID)).toBe(true);
      expect(projection.state().selectedId).toBe(PLAYER_ID);
      expect(map.navigationSelectedUuid()).toBe(PLAYER_ID);
      expect(map.dataset.selectedEntityId).toBe(PLAYER_ID);
      expect(map.hasAttribute('data-has-selection')).toBe(true);

      projection.update(payload([]));
      expect(map.hidden).toBe(true);
      expect(projection.state().selectedId).toBeNull();
      expect(map.navigationSelectedUuid()).toBeNull();
      expect(map.dataset.selectedEntityId).toBeUndefined();
      expect(map.hasAttribute('data-has-selection')).toBe(false);

      projection.update(payload([entity()]));
      expect(projection.select(PLAYER_ID)).toBe(true);
      expect(projection.state().selectedId).toBe(PLAYER_ID);
      expect(map.navigationSelectedUuid()).toBe(PLAYER_ID);
      expect(map.dataset.selectedEntityId).toBe(PLAYER_ID);
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
