import { describe, it, expect } from 'vitest';
import {
  ClientSimState, simState, modifierKey,
  helmRadarConfig, weaponsRadarConfig, scienceRadarConfig, systemChartConfig,
  HELM_RADAR_RANGE, WEAPONS_RADAR_RANGE, SCIENCE_RADAR_RANGE, SYSTEM_CHART_RANGE,
  redAlertSetMessage, firePhaserMessage, fireTorpedoMessage,
  setTargetMessage, setScienceTargetMessage,
  setSensorsTargetMessage, setPhaserModeMessage, togglePhaserModeMessage,
  setPhaserFrequencyMessage,
  nearestEntityToPoint,
  isFireButtonEnabled, isTubeLoaded, tubeReloadSecs, phaserModeLabel,
  shieldStatusView, powerTotal, canIncreasePower, canDecreasePower,
  isSciencePhaserPanelVisible,
} from '../../gui/sim-state.js';

function asteroid(uuid, x, z, radius = 1) {
  return { uuid, position: [x, 0, z], tags: ['asteroid'], radius, radar_icon: 'asteroid' };
}

function welcome(world, shipConfig) {
  return {
    type: 'Welcome',
    data: {
      state: { phase: world ? 'InProgress' : 'Lobby', players: [], complexity: {}, world },
      ship_stations: { configs: {}, min_players: 0, max_players: 0 },
      ship_config: shipConfig || {},
    },
  };
}

describe('defaults', () => {
  it('starts with an empty world and sane defaults', () => {
    const s = new ClientSimState();
    expect(s.world.entities).toEqual([]);
    expect(s.phaserMode).toBe('Auto');
    expect(s.torpedoCount).toBe(10);
    expect(s.phaserFrequency).toBe(0.5);
    expect(s.repairTeams).toEqual([]);
    expect(simState).toBeInstanceOf(ClientSimState);
  });
});

describe('apply WorldSetup / SimState', () => {
  it('WorldSetup replaces the world', () => {
    const s = new ClientSimState();
    const world = { entities: [asteroid('a', 3, 4)], scenario_title: '', scenario_description: '' };
    s.apply({ type: 'WorldSetup', data: { world } });
    expect(s.world).toEqual(world);
  });

  it('clears authority and placement projections across a real new-round sequence', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SimState', data: { snapshot: {
      station_hosts: [{ station: 'navigation', host: 'tactical', rating: 'Std' }],
      control_sources: { navigation: 'Human' },
    } } });

    s.apply({ type: 'ReturnedToLobby', data: {} });
    expect(s.stationHosts).toEqual({});
    expect(s.controlSources).toEqual({});

    s.apply({ type: 'GameStarted', data: {} });
    s.apply({ type: 'WorldSetup', data: { world: {
      entities: [], scenario_title: 'Next round', scenario_description: '',
    } } });
    expect(s.stationHosts).toEqual({});
    expect(s.controlSources).toEqual({});
  });

  it('SimState updates entity position and hull IN PLACE without appending', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0), asteroid('b', 5, 5)];
    const ref = s.world.entities; // must stay the same array
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [
        { uuid: 'a', position: [1, 0, 2], hull_fraction: 0.5 },
        { uuid: 'unknown', position: [9, 9, 9] }, // must NOT be appended
      ],
    } } });
    expect(s.world.entities).toBe(ref);
    expect(s.world.entities).toHaveLength(2);
    expect(s.world.entities[0].position).toEqual([1, 0, 2]);
    expect(s.world.entities[0].hull_fraction).toBe(0.5);
  });

  it('SimState stores generic complete-Station host projections by Station id', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SimState', data: { snapshot: {
      station_hosts: [
        { station: 'power', host: 'repair', rating: 'Std' },
        { station: 'shields', host: null, rating: 'Backfill' },
      ],
    } } });
    expect(s.stationHosts).toEqual({
      power: { station: 'power', host: 'repair', rating: 'Std' },
      shields: { station: 'shields', host: null, rating: 'Backfill' },
    });
  });

  it('SimState stores the authoritative fine-System control-source projection', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SimState', data: { snapshot: {
      control_sources: { navigation: 'Human', 'shields-system': 'Ai' },
    } } });
    expect(s.controlSources).toEqual({ navigation: 'Human', 'shields-system': 'Ai' });
  });

  it('SystemHullUpdate replaces consoleHull with SystemId-keyed entries', () => {
    // Post issue #618: publisher emits `SystemHullUpdate` not
    // `ConsoleHullUpdate`; entries carry `.system_id` (lowercase station id)
    // and `.display_name` instead of `.console` (PascalCase Console name).
    const s = new ClientSimState();
    const entries = [{ system_id: 'helm', display_name: 'Helm', current: 50, max_hp: 100 }];
    s.apply({ type: 'SystemHullUpdate', data: { entries } });
    expect(s.consoleHull).toEqual(entries);
  });

  it('SystemHullUpdate stores the authoritative ship-wide aggregate (issue #737)', () => {
    const s = new ClientSimState();
    expect(s.hullAggregate).toBeNull();
    // `entries` is only this client's projection; the ship as a whole is 0.42.
    s.apply({ type: 'SystemHullUpdate', data: {
      entries: [{ system_id: 'helm', display_name: 'Helm', current: 100, max_hp: 100 }],
      aggregate_fraction: 0.42,
    } });
    expect(s.hullAggregate).toBe(0.42);
  });

  it('SystemHullUpdate keeps the last known aggregate when a payload omits it', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SystemHullUpdate', data: { entries: [], aggregate_fraction: 0.6 } });
    s.apply({ type: 'SystemHullUpdate', data: { entries: [] } });
    expect(s.hullAggregate).toBe(0.6);
  });

  it('SystemHullUpdate stores the ship-wide destroyed share (issue #1014)', () => {
    const s = new ClientSimState();
    expect(s.hullDestroyed).toBeNull();
    // The destroyed system is not in `entries` — it belongs to a station this
    // client cannot see — so the share can only come from the host.
    s.apply({ type: 'SystemHullUpdate', data: {
      entries: [{ system_id: 'helm', display_name: 'Helm', current: 100, max_hp: 100 }],
      aggregate_fraction: 0.66,
      destroyed_fraction: 0.33,
    } });
    expect(s.hullDestroyed).toBe(0.33);
  });

  it('SystemHullUpdate keeps the last known destroyed share when a payload omits it', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SystemHullUpdate', data: { entries: [], destroyed_fraction: 0.2 } });
    s.apply({ type: 'SystemHullUpdate', data: { entries: [] } });
    expect(s.hullDestroyed).toBe(0.2);
  });

  it('SimState leaves position untouched when the snapshot omits it', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 7, 8)];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', hull_fraction: 0.25 }],
    } } });
    expect(s.world.entities[0].position).toEqual([7, 0, 8]);
    expect(s.world.entities[0].hull_fraction).toBe(0.25);
  });

  // ── shield_fraction merge (#473) ─────────────────────────────────────────

  it('SimState merges shield_fraction into the live entity in place', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0)];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', shield_fraction: 0.7 }],
    } } });
    expect(s.world.entities[0].shield_fraction).toBe(0.7);
  });

  it('SimState leaves shield_fraction untouched when the snapshot omits it', () => {
    const s = new ClientSimState();
    s.world.entities = [{ ...asteroid('a', 0, 0), shield_fraction: 0.5 }];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', position: [1, 0, 2] }],
    } } });
    expect(s.world.entities[0].position).toEqual([1, 0, 2]);
    expect(s.world.entities[0].shield_fraction).toBe(0.5);
  });

  it('SimState shield_fraction = 0 (broken shield) merges correctly', () => {
    const s = new ClientSimState();
    s.world.entities = [{ ...asteroid('a', 0, 0), shield_fraction: 0.5 }];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', shield_fraction: 0 }],
    } } });
    expect(s.world.entities[0].shield_fraction).toBe(0);
  });

  // ── shields / shield_freq merge (issue #927) ─────────────────────────────
  //
  // `EntityStateSnapshot.shields`/`.shield_freq` were always absent on the
  // wire before #927 (server_app::sim_state_broadcaster hardcoded
  // `shields: None` and had no shield_freq field at all), so this merge was
  // dead code — buildSensorsConsoleState has read `tgt.shields`/
  // `tgt.shield_freq` since #473/#870 but nothing upstream ever set them.

  it('SimState merges per-facing shields into the live entity in place', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0)];
    const facings = [
      { label: 'Fore', hp: 80, max_hp: 80, online: true, arc_id: 'fore', center_deg: 0, width_deg: 90, priority: 1, offline_remaining: 0, is_focused: false },
    ];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', shields: facings }],
    } } });
    expect(s.world.entities[0].shields).toEqual(facings);
  });

  it('SimState merges shield_freq into the live entity in place', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0)];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', shield_freq: 0.75 }],
    } } });
    expect(s.world.entities[0].shield_freq).toBe(0.75);
  });

  it('SimState leaves shields/shield_freq untouched when the snapshot omits them', () => {
    const s = new ClientSimState();
    const facings = [{ label: 'Fore', hp: 80, max_hp: 80, online: true }];
    s.world.entities = [{ ...asteroid('a', 0, 0), shields: facings, shield_freq: 0.5 }];
    s.apply({ type: 'SimState', data: { snapshot: {
      entity_states: [{ uuid: 'a', position: [1, 0, 2] }],
    } } });
    expect(s.world.entities[0].shields).toEqual(facings);
    expect(s.world.entities[0].shield_freq).toBe(0.5);
  });

  it('SimState mirrors the shared navigation waypoint', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SimState', data: { snapshot: {
      navigation_waypoint: { x: 120, z: -45 },
      entity_states: [],
    } } });
    expect(s.navigationWaypoint).toEqual({ x: 120, z: -45 });

    s.apply({ type: 'SimState', data: { snapshot: {
      navigation_waypoint: null,
      entity_states: [],
    } } });
    expect(s.navigationWaypoint).toBeNull();
  });
});

describe('BlackboardUpdate mirror', () => {
  it('starts with empty blackboards', () => {
    const s = new ClientSimState();
    expect(s.blackboards).toEqual({});
  });

  it('stores blackboard data keyed by systemId', () => {
    const s = new ClientSimState();
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['helm', { kind: 'Helm', data: { yaw: 1.5, forward_speed: 42.0, x: 10, z: -20,
                                       impulse_charge: 0.3, boost_battery: 0.8,
                                       boost_active: false, boost_enabled: true } }],
    ] } });
    expect(s.blackboards['helm']).toBeDefined();
    expect(s.blackboards['helm'].yaw).toBeCloseTo(1.5);
    expect(s.blackboards['helm'].forward_speed).toBeCloseTo(42.0);
    expect(s.blackboards['helm'].boost_enabled).toBe(true);
  });

  it('merges updates without clearing other systems', () => {
    const s = new ClientSimState();
    s.blackboards['other'] = { value: 99 };
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['helm', { kind: 'Helm', data: { yaw: 0, forward_speed: 0, x: 0, z: 0,
                                       impulse_charge: 0, boost_battery: 0,
                                       boost_active: false, boost_enabled: false } }],
    ] } });
    expect(s.blackboards['other']).toEqual({ value: 99 });
    expect(s.blackboards['helm']).toBeDefined();
  });

  it('ignores updates with malformed entries', () => {
    const s = new ClientSimState();
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['helm', null],
      ['helm', { kind: 'Helm' }],  // missing data
    ] } });
    expect(s.blackboards['helm']).toBeUndefined();
  });

  it('stores a blackboard kind it has never heard of without breaking (issue #1026)', () => {
    // The additive-on-the-wire claim, from the client's side. This fold never
    // matches `kind` against a known set, so a server that adds a variant — as
    // #1026 added `Operations` — reaches an older client as an inert entry it
    // simply carries, rather than as a decode failure that takes the whole
    // update with it. A client that switched on `kind` would have had to ship in
    // lockstep with every server that grew one.
    const s = new ClientSimState();
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['helm', { kind: 'Helm', data: { yaw: 1.0 } }],
      ['some-future-channel', { kind: 'SomethingNewEntirely', data: { anything: 1 } }],
    ] } });
    expect(s.blackboards['some-future-channel']).toEqual({ anything: 1 });
    expect(s.blackboards['helm'].yaw).toBeCloseTo(1.0);
  });

  it('resets blackboards on Welcome', () => {
    const s = new ClientSimState();
    s.blackboards['helm'] = { yaw: 1.0 };
    s.apply({ type: 'Welcome', data: {
      state: { phase: 'Lobby', players: [], complexity: {}, world: null },
      ship_stations: {},
      ship_config: {},
    } });
    expect(s.blackboards).toEqual({});
  });
});

describe('apply Welcome', () => {
  it('resets to defaults but preserves the world when present', () => {
    const s = new ClientSimState();
    s.phaserFrequency = 0.8;
    s.scienceTargetSuggestion = 'x';
    const world = { entities: [asteroid('c', 1, 2)], scenario_title: 't', scenario_description: 'd' };
    s.apply(welcome(world, { repair_team_count: 0 }));
    expect(s.world).toEqual(world);
    expect(s.phaserFrequency).toBe(0.5);
    expect(s.scienceTargetSuggestion).toBeNull();
  });

  it('without a world clears everything to defaults', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('d', 0, 0)];
    s.apply(welcome(null, { repair_team_count: 3 }));
    expect(s.world.entities).toEqual([]);
    expect(s.repairTeams).toEqual([]); // no pre-seed during lobby
  });

  it('keeps authority and placement projections through an in-progress reconnect Welcome', () => {
    const s = new ClientSimState();
    s.apply({ type: 'SimState', data: { snapshot: {
      station_hosts: [{ station: 'navigation', host: 'tactical', rating: 'Std' }],
      control_sources: { navigation: 'Human' },
    } } });

    s.apply(welcome({ entities: [], scenario_title: '', scenario_description: '' }));
    expect(s.stationHosts).toEqual({
      navigation: { station: 'navigation', host: 'tactical', rating: 'Std' },
    });
    expect(s.controlSources).toEqual({ navigation: 'Human' });

    s.apply(welcome(null));
    expect(s.stationHosts).toEqual({});
    expect(s.controlSources).toEqual({});
  });

  it('pre-seeds repair teams with Idle slots sized from ship_config when in-game', () => {
    const s = new ClientSimState();
    const world = { entities: [], scenario_title: '', scenario_description: '' };
    s.apply(welcome(world, { repair_team_count: 3 }));
    expect(s.repairTeams).toEqual(['Idle', 'Idle', 'Idle']);
  });
});

describe('apply weapons / targets / shields', () => {
  it('RepairState overwrites the team list', () => {
    const s = new ClientSimState();
    const teams = ['Idle', { Travelling: { console: 'Helm', elapsed: 1.5 } }];
    s.apply({ type: 'RepairState', data: { teams } });
    expect(s.repairTeams).toEqual(teams);
  });

  it('PhaserFired records the last target', () => {
    const s = new ClientSimState();
    s.apply({ type: 'PhaserFired', data: { bank: 'port', target_uuid: 'tgt-1' } });
    expect(s.lastPhaserTarget).toBe('tgt-1');
  });

  it('WeaponsUpdate replaces banks, tubes, count, mode and target', () => {
    const s = new ClientSimState();
    s.apply({ type: 'WeaponsUpdate', data: {
      target_uuid: 't1', target_name: 'Rock',
      banks: [{ id: 'port', fire_ready: true, on_cooldown: false, cooldown_remaining: 0 }],
      tubes: [{ id: 'fore', loaded: false, reload_secs: 2.5 }],
      torpedo_count: 7, phaser_mode: 'Manual',
    } });
    expect(s.currentTargetUuid).toBe('t1');
    expect(s.currentTargetName).toBe('Rock');
    expect(s.bankStates).toHaveLength(1);
    expect(s.tubeStates).toHaveLength(1);
    expect(s.torpedoCount).toBe(7);
    expect(s.phaserMode).toBe('Manual');
  });

  it('WeaponsUpdate with null target clears the lock', () => {
    const s = new ClientSimState();
    s.currentTargetUuid = 'old';
    s.currentTargetName = 'Old Target';
    s.apply({ type: 'WeaponsUpdate', data: { target_uuid: null, banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto' } });
    expect(s.currentTargetUuid).toBeNull();
    expect(s.currentTargetName).toBeNull();
  });

  it('WeaponsUpdate preserves missing target name only for the same target', () => {
    const s = new ClientSimState();
    s.apply({ type: 'WeaponsUpdate', data: {
      target_uuid: 't1', target_name: 'Rock',
      banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto',
    } });
    s.apply({ type: 'WeaponsUpdate', data: {
      target_uuid: 't1',
      banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto',
    } });
    expect(s.currentTargetName).toBe('Rock');

    s.apply({ type: 'WeaponsUpdate', data: {
      target_uuid: 't2',
      banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto',
    } });
    expect(s.currentTargetName).toBeNull();
  });

  it('ShieldStatus replaces facings', () => {
    const s = new ClientSimState();
    const facings = [{ label: 'Fore', hp: 80, max_hp: 100, online: true, offline_remaining: 0 }];
    s.apply({ type: 'ShieldStatus', data: { facings } });
    expect(s.shieldFacings).toEqual(facings);
  });
});

describe('apply torpedoes / modifiers / power', () => {
  it('TorpedoLaunched appends and TorpedoDestroyed removes by uuid', () => {
    const s = new ClientSimState();
    s.apply({ type: 'TorpedoLaunched', data: { uuid: 'tp1', tube: 'fore', x: 1, y: 7, z: 2, heading: 0.5 } });
    s.apply({ type: 'TorpedoLaunched', data: { uuid: 'tp2', tube: 'aft', x: 3, z: 4, heading: 1.0 } });
    expect(s.torpedoesInFlight).toHaveLength(2);
    // Vertical launch position is stored (issue #768); a message with no `y`
    // defaults to the play plane.
    expect(s.torpedoesInFlight[0].y).toBe(7);
    expect(s.torpedoesInFlight[1].y).toBe(0);
    s.apply({ type: 'TorpedoDestroyed', data: { uuid: 'tp1' } });
    expect(s.torpedoesInFlight.map(t => t.uuid)).toEqual(['tp2']);
  });

  it('ModifierAdded / ModifierRemoved track (source, slot) pairs', () => {
    const s = new ClientSimState();
    const source = { Console: 'Power' };
    s.apply({ type: 'ModifierAdded', data: { source, slot: 'MaxSpeed', bonus: 0.2 } });
    s.apply({ type: 'ModifierAdded', data: { source: 'ImpulseDrive', slot: 'MaxSpeed', bonus: 0.9 } });
    expect(s.modifierBonus(source, 'MaxSpeed')).toBe(0.2);
    expect(s.modifierBonus('ImpulseDrive', 'MaxSpeed')).toBe(0.9);
    s.apply({ type: 'ModifierRemoved', data: { source, slot: 'MaxSpeed' } });
    expect(s.modifierBonus(source, 'MaxSpeed')).toBeNull();
    expect(s.modifierBonus('ImpulseDrive', 'MaxSpeed')).toBe(0.9);
  });

  it('modifierKey distinguishes sources and slots', () => {
    expect(modifierKey({ Console: 'Helm' }, 'MaxSpeed'))
      .not.toBe(modifierKey({ Console: 'Power' }, 'MaxSpeed'));
    expect(modifierKey('ImpulseDrive', 'MaxSpeed'))
      .not.toBe(modifierKey('ImpulseDrive', 'MaxYawRate'));
  });

});

describe('apply entity lifecycle', () => {
  it('EntitySpawned appends only when absent (idempotent)', () => {
    const s = new ClientSimState();
    const snap = asteroid('e1', 1, 1);
    s.apply({ type: 'EntitySpawned', data: { snapshot: snap } });
    s.apply({ type: 'EntitySpawned', data: { snapshot: snap } });
    expect(s.world.entities).toHaveLength(1);
  });

  it('EntityDespawned removes in place, preserving the array reference', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('e1', 1, 1), asteroid('e2', 2, 2)];
    const ref = s.world.entities;
    s.apply({ type: 'EntityDespawned', data: { uuid: 'e1' } });
    expect(s.world.entities).toBe(ref);
    expect(s.world.entities.map(e => e.uuid)).toEqual(['e2']);
  });

  it('AsteroidSpawned builds an asteroid snapshot; AsteroidDestroyed removes it', () => {
    const s = new ClientSimState();
    s.apply({ type: 'AsteroidSpawned', data: { uuid: 'ast', x: 1, y: 0, z: 2, config_path: '', max_hp: 10, current_hp: 10, radius: 3 } });
    expect(s.world.entities[0]).toMatchObject({
      uuid: 'ast', position: [1, 0, 2], tags: ['asteroid'], radius: 3, radar_icon: 'asteroid',
    });
    s.apply({ type: 'AsteroidSpawned', data: { uuid: 'ast', x: 9, y: 9, z: 9, radius: 1 } });
    expect(s.world.entities).toHaveLength(1); // idempotent
    s.apply({ type: 'AsteroidDestroyed', data: { uuid: 'ast' } });
    expect(s.world.entities).toEqual([]);
  });

  it('CoordinationPopup stores target, payload and sender label', () => {
    const s = new ClientSimState();
    s.apply({ type: 'CoordinationPopup', data: { target: 'tactical', payload: { type: 'FrequencyHint', frequency: 0.75 }, sender_label: 'Sensors' } });
    expect(s.coordinationPopup).toMatchObject({ target: 'tactical', payload: { type: 'FrequencyHint', frequency: 0.75 }, senderLabel: 'Sensors' });
  });

  it('ObjectiveSummary stores mission objectives', () => {
    const s = new ClientSimState();
    const objectives = [{ id: 'obj-1', text: 'Find it', mandatory: true, status: 'Active', targets: ['Beacon'] }];
    s.apply({ type: 'ObjectiveSummary', data: { objectives } });
    expect(s.objectives).toEqual(objectives);
  });

  it('CommsState also refreshes mission objectives', () => {
    const s = new ClientSimState();
    const objectives = [{ id: 'obj-2', text: 'Hail it', mandatory: false, status: 'Active', targets: ['Station'] }];
    s.apply({ type: 'CommsState', data: { messages: [], contacts: [], objectives } });
    expect(s.objectives).toEqual(objectives);
  });

  it('CommsResponseRejected stamps commsRejection with a timestamp', () => {
    const s = new ClientSimState();
    expect(s.commsRejection).toBeNull();
    s.apply({ type: 'CommsResponseRejected', data: { message_id: 'm7', response_index: 2 } });
    expect(s.commsRejection).toMatchObject({ message_id: 'm7', response_index: 2 });
    expect(typeof s.commsRejection.ts).toBe('number');
  });

  it('unrelated messages do not disturb the state', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0)];
    const before = JSON.stringify({ ...s, modifiers: undefined });
    s.apply({ type: 'PlayerJoined', data: { player: { token: 'x', name: 'Y', station: 'helm', connected: true } } });
    expect(JSON.stringify({ ...s, modifiers: undefined })).toBe(before);
  });
});

describe('radar configs', () => {
  it('match the Rust ranges and tag filters', () => {
    expect(helmRadarConfig()).toEqual({ range: HELM_RADAR_RANGE, shows: ['asteroid', 'star', 'planet', 'ship'] });
    expect(weaponsRadarConfig()).toEqual({ range: WEAPONS_RADAR_RANGE, shows: ['asteroid', 'ship'] });
    expect(scienceRadarConfig().range).toBe(SCIENCE_RADAR_RANGE);
    expect(scienceRadarConfig().shows).toContain('region');
    expect(systemChartConfig().range).toBe(SYSTEM_CHART_RANGE);
    expect(systemChartConfig().shows).not.toContain('asteroid');
    expect(WEAPONS_RADAR_RANGE).toBeGreaterThan(HELM_RADAR_RANGE);
  });
});

describe('message builders', () => {
  it('build serde tag/content wire objects', () => {
    expect(redAlertSetMessage(true)).toEqual({
      type: 'ControlSystem',
      data: {
        target: 'red-alert',
        payload: { type: 'SetRedAlert', data: { active: true } },
      },
    });
    expect(redAlertSetMessage(false)).toEqual({
      type: 'ControlSystem',
      data: {
        target: 'red-alert',
        payload: { type: 'SetRedAlert', data: { active: false } },
      },
    });
    expect(firePhaserMessage('port')).toEqual({ type: 'FirePhaser', data: { bank: 'port' } });
    expect(fireTorpedoMessage('fore', 'tgt')).toEqual({ type: 'FireTorpedo', data: { tube: 'fore', target_uuid: 'tgt' } });
    expect(fireTorpedoMessage('fore')).toEqual({ type: 'FireTorpedo', data: { tube: 'fore', target_uuid: null } });
    // Post-#822 (short-form shim retired): system controls are full
    // ControlSystem envelopes addressing the owning fine system.
    expect(setTargetMessage('u')).toEqual({
      type: 'ControlSystem',
      data: { target: 'tactical-radar', payload: { type: 'SetTarget', data: { uuid: 'u' } } },
    });
    expect(setScienceTargetMessage('u')).toEqual({
      type: 'ControlSystem',
      data: { target: 'sensors', payload: { type: 'SetScienceTarget', data: { uuid: 'u' } } },
    });
    // The sensors alias emits the same wire message as a science target —
    // the old short-form SetSensorsTarget → SetScienceTarget rename now
    // lives in the builder, not the codec.
    expect(setSensorsTargetMessage('u')).toEqual(setScienceTargetMessage('u'));
    expect(setPhaserModeMessage('Manual')).toEqual({
      type: 'ControlSystem',
      data: { target: 'phaser-control', payload: { type: 'SetPhaserMode', data: { mode: 'Manual' } } },
    });
  });

  it('togglePhaserModeMessage flips Auto <-> Manual', () => {
    expect(togglePhaserModeMessage('Auto').data.payload.data.mode).toBe('Manual');
    expect(togglePhaserModeMessage('Manual').data.payload.data.mode).toBe('Auto');
  });

  it('setPhaserFrequencyMessage clamps to [0, 1]', () => {
    expect(setPhaserFrequencyMessage(1.5).data.payload.data.frequency).toBe(1.0);
    expect(setPhaserFrequencyMessage(-0.5).data.payload.data.frequency).toBe(0.0);
    expect(setPhaserFrequencyMessage(0.25).data.payload.data.frequency).toBe(0.25);
  });
});

describe('nearestEntityToPoint', () => {
  it('returns null for empty list', () => {
    expect(nearestEntityToPoint({ x: 0, y: 0 }, [])).toBeNull();
  });
  it('picks the closest entity', () => {
    const entities = [
      { uuid: 'far', x: 50, y: 50 },
      { uuid: 'near', x: 1, y: 1 },
      { uuid: 'mid', x: 10, y: 10 },
    ];
    expect(nearestEntityToPoint({ x: 0, y: 0 }, entities)).toBe('near');
  });
  it('breaks ties first-wins', () => {
    const entities = [{ uuid: 'first', x: 5, y: 5 }, { uuid: 'second', x: 5, y: 5 }];
    expect(nearestEntityToPoint({ x: 0, y: 0 }, entities)).toBe('first');
  });
});

describe('view helpers', () => {
  it('isFireButtonEnabled requires fire_ready and not on_cooldown', () => {
    const s = new ClientSimState();
    s.bankStates = [
      { id: 'port', fire_ready: true, on_cooldown: false },
      { id: 'star', fire_ready: true, on_cooldown: true },
    ];
    expect(isFireButtonEnabled(s, 'port')).toBe(true);
    expect(isFireButtonEnabled(s, 'star')).toBe(false);
    expect(isFireButtonEnabled(s, 'missing')).toBe(false);
  });

  it('isTubeLoaded / tubeReloadSecs read tube state with safe defaults', () => {
    const s = new ClientSimState();
    s.tubeStates = [{ id: 'fore', loaded: true, reload_secs: 0 }, { id: 'aft', loaded: false, reload_secs: 3.5 }];
    expect(isTubeLoaded(s, 'fore')).toBe(true);
    expect(isTubeLoaded(s, 'aft')).toBe(false);
    expect(tubeReloadSecs(s, 'aft')).toBe(3.5);
    expect(tubeReloadSecs(s, 'missing')).toBe(0);
  });

  it('phaserModeLabel maps modes to labels', () => {
    expect(phaserModeLabel('Auto')).toBe('AUTO');
    expect(phaserModeLabel('Manual')).toBe('MANUAL');
  });

  it('shieldStatusView builds equal pie slices with fill fractions', () => {
    const TAU = Math.PI * 2;
    const facings = [
      { label: 'Fore', hp: 100, max_hp: 100, online: true },
      { label: 'Port', hp: 50, max_hp: 100, online: true },
      { label: 'Aft', hp: 0, max_hp: 100, online: false },
      { label: 'Starboard', hp: 100, max_hp: 100, online: true },
    ];
    const view = shieldStatusView(facings);
    expect(view).toHaveLength(4);
    expect(view[0].fill_fraction).toBeCloseTo(1.0);
    expect(view[1].fill_fraction).toBeCloseTo(0.5);
    expect(view[2].fill_fraction).toBeCloseTo(0.0);
    // Facing 0 centred on top (forward).
    expect((view[0].start_angle + view[0].end_angle) / 2).toBeCloseTo(0);
    // Arcs tile seamlessly with span TAU/4.
    for (const arc of view) expect(arc.end_angle - arc.start_angle).toBeCloseTo(TAU / 4);
    for (let i = 0; i < 3; i++) expect(view[i].end_angle).toBeCloseTo(view[i + 1].start_angle);
    expect(shieldStatusView([])).toEqual([]);
  });

  // The third powered console is Shields, not Sensors — issue #952 swapped the
  // third power group over — and the `locked` argument is gone with the
  // server's brownout lock. The old assertions passed a third `true` and
  // expected a refusal; nothing on the server can produce that state any more,
  // so left as they were they would have gone on passing against a lock that
  // does not exist while the real third console went untested.
  it('power helpers enforce the 6+2 allocation rules', () => {
    expect(powerTotal([2, 2, 2])).toBe(6);
    expect(canIncreasePower([2, 2, 2], 'Helm')).toBe(true);
    expect(canIncreasePower([4, 2, 2], 'Helm')).toBe(false);  // console at cap
    expect(canIncreasePower([4, 2, 2], 'Tactical')).toBe(false); // total at 8
    expect(canIncreasePower([2, 2, 2], 'Comms')).toBe(false); // not a powered console
    expect(canDecreasePower([2, 2, 2], 'Shields')).toBe(true);
    expect(canDecreasePower([2, 2, 1], 'Shields')).toBe(false); // at its per-group minimum
    expect(canDecreasePower([2, 2, 2], 'Sensors')).toBe(false); // no longer a powered console
  });

  it('isSciencePhaserPanelVisible only when Tactical is Low', () => {
    expect(isSciencePhaserPanelVisible({ Tactical: 'Low' })).toBe(true);
    expect(isSciencePhaserPanelVisible({ Tactical: 'Std' })).toBe(false);
    expect(isSciencePhaserPanelVisible({})).toBe(false);
    expect(isSciencePhaserPanelVisible(undefined)).toBe(false);
  });
});

// ── Single-store fields moved from client.html (issue #819) ─────────────────
// client.html's hand-maintained mirror and flat blackboard scalars were
// deleted; the console builders now read window.simState directly. These
// tests pin the fields/getters that migration relies on.

describe('weapons state derived in apply (#819)', () => {
  // The derived weaponsFireReady / weaponsOnCooldown / weaponsReadyBankId /
  // weaponsFiring flags were deleted in issue #825 — no console read them.

  it('WeaponsUpdate stores bank state and blips', () => {
    const banks = [
      { id: 'port', fire_ready: false, on_cooldown: true },
      { id: 'star', fire_ready: true,  on_cooldown: false },
    ];
    const s = new ClientSimState();
    s.apply({ type: 'WeaponsUpdate', data: {
      target_uuid: 't1', banks, tubes: [], torpedo_count: 0, phaser_mode: 'Auto',
      blips: [{ uuid: 'b1' }],
    } });
    expect(s.currentTargetUuid).toBe('t1');
    expect(s.bankStates).toEqual(banks);
    expect(s.weaponsBlips).toEqual([{ uuid: 'b1' }]);
  });

  it('WeaponsUpdate keeps previous blips when the payload omits them', () => {
    const s = new ClientSimState();
    s.weaponsBlips = [{ uuid: 'old' }];
    s.apply({ type: 'WeaponsUpdate', data: { target_uuid: 't1', banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto' } });
    expect(s.weaponsBlips).toEqual([{ uuid: 'old' }]);
  });

  it('TargetLock locked sets the target; unlocked clears it', () => {
    const s = new ClientSimState();
    s.apply({ type: 'TargetLock', data: { uuid: 'tgt-9', locked: true } });
    expect(s.currentTargetUuid).toBe('tgt-9');
    s.apply({ type: 'TargetLock', data: { uuid: 'tgt-9', locked: false } });
    expect(s.currentTargetUuid).toBeNull();
    expect(s.currentTargetName).toBeNull();
  });
});

describe('shield focus + target clearing (#819)', () => {
  it('ShieldStatus derives shieldFocusedFacing from is_focused', () => {
    const s = new ClientSimState();
    s.apply({ type: 'ShieldStatus', data: { facings: [
      { label: 'Fore', hp: 1, max_hp: 1, online: true },
      { label: 'Aft', hp: 1, max_hp: 1, online: true, is_focused: true },
    ] } });
    expect(s.shieldFocusedFacing).toBe('Aft');
    s.apply({ type: 'ShieldStatus', data: { facings: [
      { label: 'Fore', hp: 1, max_hp: 1, online: true },
    ] } });
    expect(s.shieldFocusedFacing).toBeNull();
  });

  it('EntityDespawned clears a matching weapons target and sensors target', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('e1', 1, 1)];
    s.currentTargetUuid = 'e1';
    s.currentTargetName = 'Rock';
    s.sensorsTarget = 'e1';
    s.apply({ type: 'EntityDespawned', data: { uuid: 'e1' } });
    expect(s.currentTargetUuid).toBeNull();
    expect(s.currentTargetName).toBeNull();
    expect(s.sensorsTarget).toBeNull();
  });

  it('AsteroidDestroyed leaves unrelated targets alone', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a1', 1, 1)];
    s.currentTargetUuid = 'other';
    s.sensorsTarget = 'another';
    s.apply({ type: 'AsteroidDestroyed', data: { uuid: 'a1' } });
    expect(s.currentTargetUuid).toBe('other');
    expect(s.sensorsTarget).toBe('another');
  });
});

describe('console-state view aliases (#819)', () => {
  it('asteroids getter is the live world entity array (same reference)', () => {
    const s = new ClientSimState();
    expect(s.asteroids).toBe(s.world.entities);
    s.apply({ type: 'EntitySpawned', data: { snapshot: asteroid('e1', 1, 1) } });
    expect(s.asteroids.map(e => e.uuid)).toEqual(['e1']);
  });

  it('helm blackboard drives shipX/shipZ/shipYaw/forwardSpeed/impulseChargeProgress', () => {
    const s = new ClientSimState();
    expect(s.shipX).toBe(0);
    expect(s.shipYaw).toBe(0);
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['helm', { kind: 'Helm', data: { x: 10, z: -20, yaw: 1.5, forward_speed: 42, impulse_charge: 0.3 } }],
    ] } });
    expect(s.shipX).toBe(10);
    expect(s.shipZ).toBe(-20);
    expect(s.shipYaw).toBeCloseTo(1.5);
    expect(s.forwardSpeed).toBe(42);
    expect(s.impulseChargeProgress).toBeCloseTo(0.3);
  });

  it('captain blackboard drives currentView (Camera data / Cinematic / kind) and redAlert', () => {
    const s = new ClientSimState();
    expect(s.currentView).toBe('Fore');
    expect(s.redAlert).toBe(false);
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['captain', { kind: 'Captain', data: { view_mode: { kind: 'Camera', data: 'Aft' }, red_alert: true } }],
    ] } });
    expect(s.currentView).toBe('Aft');
    expect(s.redAlert).toBe(true);
    s.blackboards['captain'].view_mode = { kind: 'Cinematic' };
    expect(s.currentView).toBe('cinematic');
    s.blackboards['captain'].view_mode = { kind: 'Radar' };
    expect(s.currentView).toBe('Radar');
  });

  it('navigation blackboard mirrors the shared waypoint', () => {
    const s = new ClientSimState();
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['navigation', { kind: 'Navigation', data: { navigation_waypoint: { x: 5, z: 6 } } }],
    ] } });
    expect(s.navigationWaypoint).toEqual({ x: 5, z: 6 });
    s.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['navigation', { kind: 'Navigation', data: { navigation_waypoint: null } }],
    ] } });
    expect(s.navigationWaypoint).toBeNull();
  });

  it('weapons aliases mirror the underlying fields; weaponsTarget setter writes through', () => {
    const s = new ClientSimState();
    s.apply({ type: 'WeaponsUpdate', data: {
      target_uuid: 't1', target_name: 'Rock',
      banks: [{ id: 'port', fire_ready: true, on_cooldown: false }],
      tubes: [{ id: 'fore', loaded: true, reload_secs: 0 }],
      torpedo_count: 7, phaser_mode: 'Manual',
    } });
    expect(s.weaponsTarget).toBe('t1');
    expect(s.weaponsTargetName).toBe('Rock');
    expect(s.weaponsBanks).toBe(s.bankStates);
    expect(s.weaponsTubes).toBe(s.tubeStates);
    expect(s.weaponsTorpedoCount).toBe(7);
    expect(s.weaponsPhaserMode).toBe('Manual');
    // The action-map's optimistic mutate patch assigns weaponsTarget directly.
    Object.assign(s, { weaponsTarget: 'patched' });
    expect(s.currentTargetUuid).toBe('patched');
  });

  it('commsMessages / commsContacts delegate to window.commsState, empty when absent', () => {
    const s = new ClientSimState();
    expect(s.commsMessages).toEqual([]);
    expect(s.commsContacts).toEqual([]);
    const hadWindow = typeof globalThis.window !== 'undefined';
    const prev = hadWindow ? globalThis.window : undefined;
    try {
      globalThis.window = { commsState: { messages: [{ id: 'm1' }], contacts: [{ uuid: 'c1' }] } };
      expect(s.commsMessages).toEqual([{ id: 'm1' }]);
      expect(s.commsContacts).toEqual([{ uuid: 'c1' }]);
    } finally {
      if (hadWindow) globalThis.window = prev;
      else delete globalThis.window;
    }
  });
});
