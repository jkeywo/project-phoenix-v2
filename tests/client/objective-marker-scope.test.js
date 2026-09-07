import { describe, expect, it } from 'vitest';
import { ClientSimState } from '../../gui/sim-state.js';
import {
  buildHelmConsoleState, buildNavigationConsoleState, buildRadarRegions,
  buildSensorsConsoleState,
} from '../../gui/console-state.js';
import { buildGmStationConsoleInput } from '../../gui/gm-station-puppet.js';

// Shape emitted by reconcile_runtime_entities for an active Objective's target.
// Its initial, spawn and reconnect rows carry a global hint; the Rust producer
// regressions exercise actual ECS publication of these same fields.
function marker(id) {
  return { uuid: `uuid-${id}`, id, name: `display.${id}`, position: [20, 0, 10],
    tags: ['objective_marker'], radar_icon: 'waypoint', region_colour: [0.1, 0.2, 0.3],
    radius: 10, objective_target: true };
}
const assignment = { id: 'private', text: 'objective.test', mandatory: true,
  status: 'Active', targets: ['seeded', 'spawned'], source: 'Mission' };

function assertNoMarker(state) {
  const nav = JSON.parse(buildNavigationConsoleState(state));
  expect(nav.blips).toEqual([]);
  expect(nav.regions).toEqual([]);
  expect(buildRadarRegions(state.asteroids, state.objectives)).toEqual([]);
}

describe('recipient Objective marker presentation', () => {
  it('replaces global flags from WorldSetup, EntitySpawned and reconnect Welcome', () => {
    const state = new ClientSimState();
    state.apply({ type: 'WorldSetup', data: { world: { entities: [marker('seeded')] } } });
    state.apply({ type: 'ObjectiveSummary', data: { objectives: [] } });
    assertNoMarker(state);
    state.apply({ type: 'EntitySpawned', data: { snapshot: marker('spawned') } });
    assertNoMarker(state);
    state.apply({ type: 'Welcome', data: { state: { phase: 'InProgress',
      world: { entities: [marker('seeded'), marker('spawned')] } } } });
    expect(state.world.entities).toHaveLength(2);
    assertNoMarker(state);

    // A recipient gets both annotations from its scoped summary, and loses
    // them on resolution even though the shared metadata was not republished.
    state.apply({ type: 'ObjectiveSummary', data: { objectives: [assignment] } });
    const shown = JSON.parse(buildNavigationConsoleState(state));
    expect(shown.blips.map(row => row.uuid)).toEqual(['uuid-seeded', 'uuid-spawned']);
    expect(shown.regions).toHaveLength(2);
    state.apply({ type: 'ObjectiveSummary', data: { objectives: [{ ...assignment, status: 'Completed' }] } });
    assertNoMarker(state);
    expect(state.world.entities.every(row => row.objective_target)).toBe(true);
  });

  it('keeps ordinary contacts while removing another ship’s Objective annotations', () => {
    const entity = { ...marker('contact'), tags: ['ship'], radar_icon: 'ship' };
    const state = { asteroids: [entity], objectives: [], sensorsRadarShows: ['ship'],
      sensorsRadarRange: 100, helmRadarRange: 100 };
    for (const build of [buildHelmConsoleState, buildSensorsConsoleState]) {
      const contact = JSON.parse(build(state)).blips.find(row => row.uuid === entity.uuid);
      expect(contact).toBeDefined();
      expect(contact.objective_target).toBe(false);
    }
    expect(entity.objective_target).toBe(true);
  });

  it('uses the same scope rule for the GM authentic Navigation replica and its cached regions', () => {
    const ship = {
      ship_id: 'ship-b', name: 'B', stations: [{ station_id: 'navigation', operators: [] }],
      ship_config: { station_systems: {}, system_console_families: {},
        system_kinds: {}, blackboard_console_families: {}, nav_chart_shows: ['station'] },
      entities: [marker('seeded'), marker('spawned')], entity_states: [], objectives: [],
      blackboards: [], console_hull: [], ship_pose: {},
    };
    const projection = { ships: [ship], activity: [], results: [] };
    assertNoMarker(buildGmStationConsoleInput(projection, ship));
    ship.objectives = [assignment];
    const recipient = buildGmStationConsoleInput(projection, ship);
    expect(JSON.parse(buildNavigationConsoleState(recipient)).regions).toHaveLength(2);
    expect(ship.entities.every(row => row.objective_target)).toBe(true);
  });
});
