import { describe, it, expect } from 'vitest';
import {
  ClientSimState, simState, modifierKey,
  helmRadarConfig, weaponsRadarConfig, scienceRadarConfig, systemChartConfig,
  HELM_RADAR_RANGE, WEAPONS_RADAR_RANGE, SCIENCE_RADAR_RANGE, SYSTEM_CHART_RANGE,
  redAlertToggleMessage, firePhaserMessage, fireTorpedoMessage,
  dispatchRepairTeamMessage, setTargetMessage, setScienceTargetMessage,
  setSensorsTargetMessage, setPhaserModeMessage, togglePhaserModeMessage,
  increasePowerMessage, decreasePowerMessage, setPhaserFrequencyMessage,
  nearestEntityToPoint,
  isFireButtonEnabled, isTubeLoaded, tubeReloadSecs, phaserModeLabel,
  shieldStatusView, powerTotal, canIncreasePower, canDecreasePower,
  batteryPercentage, isPowerLocked, isSciencePhaserPanelVisible,
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

  it('SimState updates entity position and hull IN PLACE without appending', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0), asteroid('b', 5, 5)];
    const ref = s.world.entities; // must stay the same array
    s.apply({ type: 'SimState', data: { snapshot: {
      console_hull: [{ console: 'Helm', current: 50, max_hp: 100 }],
      entity_states: [
        { uuid: 'a', position: [1, 0, 2], hull_fraction: 0.5 },
        { uuid: 'unknown', position: [9, 9, 9] }, // must NOT be appended
      ],
    } } });
    expect(s.world.entities).toBe(ref);
    expect(s.world.entities).toHaveLength(2);
    expect(s.world.entities[0].position).toEqual([1, 0, 2]);
    expect(s.world.entities[0].hull_fraction).toBe(0.5);
    expect(s.consoleHull).toEqual([{ console: 'Helm', current: 50, max_hp: 100 }]);
  });

  it('SimState leaves position untouched when the snapshot omits it', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 7, 8)];
    s.apply({ type: 'SimState', data: { snapshot: {
      console_hull: [],
      entity_states: [{ uuid: 'a', hull_fraction: 0.25 }],
    } } });
    expect(s.world.entities[0].position).toEqual([7, 0, 8]);
    expect(s.world.entities[0].hull_fraction).toBe(0.25);
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
    expect(s.bankStates).toHaveLength(1);
    expect(s.tubeStates).toHaveLength(1);
    expect(s.torpedoCount).toBe(7);
    expect(s.phaserMode).toBe('Manual');
  });

  it('WeaponsUpdate with null target clears the lock', () => {
    const s = new ClientSimState();
    s.currentTargetUuid = 'old';
    s.apply({ type: 'WeaponsUpdate', data: { target_uuid: null, banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto' } });
    expect(s.currentTargetUuid).toBeNull();
  });

  it('Science and Sensors target suggestions update independently', () => {
    const s = new ClientSimState();
    s.apply({ type: 'ScienceTargetSuggestion', data: { uuid: 'sci' } });
    s.apply({ type: 'SensorsTargetSuggestion', data: { uuid: 'sen' } });
    expect(s.scienceTargetSuggestion).toBe('sci');
    expect(s.sensorsTargetSuggestion).toBe('sen');
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
    s.apply({ type: 'TorpedoLaunched', data: { uuid: 'tp1', tube: 'fore', x: 1, z: 2, heading: 0.5 } });
    s.apply({ type: 'TorpedoLaunched', data: { uuid: 'tp2', tube: 'aft', x: 3, z: 4, heading: 1.0 } });
    expect(s.torpedoesInFlight).toHaveLength(2);
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

  it('PowerState stores the latest payload', () => {
    const s = new ClientSimState();
    s.apply({ type: 'PowerState', data: { helm: 3, weapons: 2, sensors: 1, battery_charge: 42.5, locked: true } });
    expect(s.powerStatePayload).toEqual({ helm: 3, weapons: 2, sensors: 1, battery_charge: 42.5, locked: true });
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

  it('FrequencyHint stores the hint and Welcome clears it', () => {
    const s = new ClientSimState();
    s.apply({ type: 'FrequencyHint', data: { frequency: 0.75 } });
    expect(s.frequencyHint).toBe(0.75);
    s.apply(welcome(null, {}));
    expect(s.frequencyHint).toBeNull();
  });

  it('unrelated messages do not disturb the state', () => {
    const s = new ClientSimState();
    s.world.entities = [asteroid('a', 0, 0)];
    const before = JSON.stringify({ ...s, modifiers: undefined });
    s.apply({ type: 'PlayerJoined', data: { player: { token: 'x', name: 'Y', consoles: ['Helm'], connected: true } } });
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
    expect(redAlertToggleMessage()).toEqual({ type: 'ToggleRedAlert' });
    expect(firePhaserMessage('port')).toEqual({ type: 'FirePhaser', data: { bank: 'port' } });
    expect(fireTorpedoMessage('fore', 'tgt')).toEqual({ type: 'FireTorpedo', data: { tube: 'fore', target_uuid: 'tgt' } });
    expect(fireTorpedoMessage('fore')).toEqual({ type: 'FireTorpedo', data: { tube: 'fore', target_uuid: null } });
    expect(dispatchRepairTeamMessage(1, 'Helm')).toEqual({ type: 'DispatchRepairTeam', data: { team_idx: 1, console: 'Helm' } });
    expect(setTargetMessage('u')).toEqual({ type: 'SetTarget', data: { uuid: 'u' } });
    expect(setScienceTargetMessage('u')).toEqual({ type: 'SetScienceTarget', data: { uuid: 'u' } });
    expect(setSensorsTargetMessage('u')).toEqual({ type: 'SetSensorsTarget', data: { uuid: 'u' } });
    expect(setPhaserModeMessage('Manual')).toEqual({ type: 'SetPhaserMode', data: { mode: 'Manual' } });
    expect(increasePowerMessage('Helm')).toEqual({ type: 'IncreasePower', data: { console: 'Helm' } });
    expect(decreasePowerMessage('Sensors')).toEqual({ type: 'DecreasePower', data: { console: 'Sensors' } });
  });

  it('togglePhaserModeMessage flips Auto <-> Manual', () => {
    expect(togglePhaserModeMessage('Auto').data.mode).toBe('Manual');
    expect(togglePhaserModeMessage('Manual').data.mode).toBe('Auto');
  });

  it('setPhaserFrequencyMessage clamps to [0, 1]', () => {
    expect(setPhaserFrequencyMessage(1.5).data.frequency).toBe(1.0);
    expect(setPhaserFrequencyMessage(-0.5).data.frequency).toBe(0.0);
    expect(setPhaserFrequencyMessage(0.25).data.frequency).toBe(0.25);
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

  it('power helpers enforce the 6+2 allocation rules', () => {
    expect(powerTotal([2, 2, 2])).toBe(6);
    expect(canIncreasePower([2, 2, 2], 'Helm', false)).toBe(true);
    expect(canIncreasePower([4, 2, 2], 'Helm', false)).toBe(false);  // console at cap
    expect(canIncreasePower([4, 2, 2], 'Tactical', false)).toBe(false); // total at 8
    expect(canIncreasePower([2, 2, 2], 'Helm', true)).toBe(false);   // locked
    expect(canIncreasePower([2, 2, 2], 'Comms', false)).toBe(false); // not a powered console
    expect(canDecreasePower([2, 2, 2], 'Sensors', false)).toBe(true);
    expect(canDecreasePower([2, 2, 1], 'Sensors', false)).toBe(false); // at floor
    expect(canDecreasePower([2, 2, 2], 'Sensors', true)).toBe(false);  // locked
  });

  it('batteryPercentage / isPowerLocked read the payload with defaults', () => {
    expect(batteryPercentage(null)).toBe(0);
    expect(isPowerLocked(null)).toBe(false);
    const payload = { helm: 2, weapons: 2, sensors: 2, battery_charge: 73.5, locked: true };
    expect(batteryPercentage(payload)).toBe(73.5);
    expect(isPowerLocked(payload)).toBe(true);
  });

  it('isSciencePhaserPanelVisible only when Tactical is Low', () => {
    expect(isSciencePhaserPanelVisible({ Tactical: 'Low' })).toBe(true);
    expect(isSciencePhaserPanelVisible({ Tactical: 'Std' })).toBe(false);
    expect(isSciencePhaserPanelVisible({})).toBe(false);
    expect(isSciencePhaserPanelVisible(undefined)).toBe(false);
  });
});
