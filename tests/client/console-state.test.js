import { describe, it, expect } from 'vitest';
import {
  entityX, entityZ, entityRadius,
  buildBlips,
  buildRadarRegions,
  buildWaypointBlip,
  buildTargetBlip,
  WEAPONS_RADAR_RANGE, HELM_RADAR_RANGE, SENSORS_RADAR_RANGE,
  NAVIGATION_RADAR_RANGE,
  aggregateStationHull,
  repairCoreAndTargets,
  torpSlotStates,
  buildWeaponsConsoleState,
  buildCaptainConsoleState,
  buildHelmConsoleState,
  buildRepairConsoleState,
  buildPowerConsoleState,
  buildShieldsConsoleState,
  buildSensorsConsoleState,
  buildCommsConsoleState,
  buildNavigationConsoleState,
  buildScienceConsoleState,
  buildEngineeringConsoleState,
  buildCruiserCommsConsoleState,
  buildDestroyerCaptainConsoleState,
  buildDestroyerTacticalConsoleState,
  buildDestroyerEngineeringConsoleState,
} from '../../gui/console-state.js';
import { ClientSimState } from '../../gui/sim-state.js';

// ── Entity helpers ────────────────────────────────────────────────────────────

describe('entityX', () => {
  it('reads e.x when present', () => expect(entityX({ x: 42 })).toBe(42));
  it('reads position[0] when x is absent', () => expect(entityX({ position: [7, 0, 3] })).toBe(7));
  it('returns 0 when neither x nor position is present', () => expect(entityX({})).toBe(0));
});

describe('entityZ', () => {
  it('reads e.z when present', () => expect(entityZ({ z: -5 })).toBe(-5));
  it('reads position[2] when z is absent', () => expect(entityZ({ position: [0, 0, 9] })).toBe(9));
  it('returns 0 when neither z nor position is present', () => expect(entityZ({})).toBe(0));
});

describe('entityRadius', () => {
  it('returns e.radius when present', () => expect(entityRadius({ radius: 10 })).toBe(10));
  it('returns 4 when radius is absent', () => expect(entityRadius({})).toBe(4));
  it('returns 4 when radius is null', () => expect(entityRadius({ radius: null })).toBe(4));
});

// ── buildBlips ────────────────────────────────────────────────────────────────

const NEAR = { uuid: 'a', x: 10, z: 0, radius: 2, tags: ['asteroid'], radar_icon: 'asteroid' };
const FAR  = { uuid: 'b', x: 1000, z: 1000, radius: 2, tags: ['asteroid'], radar_icon: 'asteroid' };
const SHIP = { uuid: 's', x: 5, z: 0, tags: ['ship'], radar_icon: 'ship' };
const STATION = { uuid: 'st', x: 0, z: 5, tags: ['station'], radar_icon: 'station' };

describe('buildBlips', () => {
  it('returns empty array for no entities', () => {
    expect(buildBlips([], 0, 0, 0, 100)).toEqual([]);
  });

  it('returns empty array for null entities', () => {
    expect(buildBlips(null, 0, 0, 0, 100)).toEqual([]);
  });

  it('includes entity within range', () => {
    const blips = buildBlips([NEAR], 0, 0, 0, 100);
    expect(blips).toHaveLength(1);
    expect(blips[0].uuid).toBe('a');
  });

  it('excludes entity beyond range', () => {
    const blips = buildBlips([FAR], 0, 0, 0, 100);
    expect(blips).toHaveLength(0);
  });

  it('classifies ship kind correctly', () => {
    const blips = buildBlips([SHIP], 0, 0, 0, 100);
    expect(blips[0].kind).toBe('ship');
  });

  it('classifies station kind correctly', () => {
    const blips = buildBlips([STATION], 0, 0, 0, 100);
    expect(blips[0].kind).toBe('station');
  });

  it('classifies entity with no recognised tag as asteroid', () => {
    const blips = buildBlips([{ uuid: 'x', x: 1, z: 0, tags: ['unknown'], radar_icon: 'asteroid' }], 0, 0, 0, 100);
    expect(blips[0].kind).toBe('asteroid');
  });

  it('uses explicit radar_icon as the authoritative blip kind', () => {
    const blips = buildBlips(
      [{ uuid: 'sun', x: 1, z: 0, tags: ['unknown'], radar_icon: 'star' }],
      0, 0, 0, 100
    );
    expect(blips[0].kind).toBe('star');
    expect(blips[0].icon).toBe('star');
  });

  it('supports entity_tags fallback', () => {
    const blips = buildBlips([{ uuid: 'q', x: 1, z: 0, entity_tags: ['ship'], radar_icon: 'ship' }], 0, 0, 0, 100);
    expect(blips[0].kind).toBe('ship');
  });

  it('normalises scaled_radius = radius / range', () => {
    const blips = buildBlips([NEAR], 0, 0, 0, 100);
    expect(blips[0].scaled_radius).toBeCloseTo(2 / 100);
  });

  describe('rotate=true (ship-local frame, weapons/helm)', () => {
    it('at yaw=0 entity directly ahead (dz=-range) → radar_y ≈ 1', () => {
      const range = 100;
      const blips = buildBlips([{ uuid: 'f', x: 0, z: -range, radius: 1, tags: [], radar_icon: 'asteroid' }], 0, 0, 0, range, { rotate: true });
      // dx=0, dz=-range → radar_x=(0·1+(-range)·0)/range=0, radar_y=(0·0-(-range)·1)/range=1
      expect(blips[0].radar_x).toBeCloseTo(0);
      expect(blips[0].radar_y).toBeCloseTo(1);
    });

    it('at yaw=0 entity to starboard (dx=+range) → radar_x ≈ 1', () => {
      const range = 100;
      const blips = buildBlips([{ uuid: 'r', x: range, z: 0, radius: 1, tags: [], radar_icon: 'asteroid' }], 0, 0, 0, range, { rotate: true });
      expect(blips[0].radar_x).toBeCloseTo(1);
      expect(blips[0].radar_y).toBeCloseTo(0);
    });
  });

  describe('rotate=false (world-axis frame, navigation)', () => {
    it('radar_x = dx/range, radar_y = dz/range', () => {
      const range = 100;
      const blips = buildBlips([{ uuid: 'w', x: 30, z: 40, radius: 1, tags: [], radar_icon: 'asteroid' }], 0, 0, 0, range, { rotate: false });
      expect(blips[0].radar_x).toBeCloseTo(30 / 100);
      expect(blips[0].radar_y).toBeCloseTo(40 / 100);
    });
  });

  it('merges extra fields from opts.extra', () => {
    const blips = buildBlips(
      [{ uuid: 'e', x: 1, z: 0, tags: [], name: 'Zeta', faction: 'pirate', radar_icon: 'asteroid' }],
      0, 0, 0, 100,
      { extra: (a) => ({ name: a.name || null, faction: a.faction || null }) }
    );
    expect(blips[0].name).toBe('Zeta');
    expect(blips[0].faction).toBe('pirate');
  });

  it('filters visible blips by opts.shows', () => {
    const blips = buildBlips(
      [
        { uuid: 'ship-1', x: 1, z: 0, tags: ['ship'], radar_icon: 'ship' },
        { uuid: 'planet-1', x: 2, z: 0, tags: ['planet'], radar_icon: 'planet' },
      ],
      0, 0, 0, 100,
      { shows: ['ship'] }
    );
    expect(blips.map(b => b.uuid)).toEqual(['ship-1']);
  });

  it('marks blips selectable from target_tags and opts.selects', () => {
    const blips = buildBlips(
      [
        { uuid: 'ship-1', x: 1, z: 0, tags: ['ship'], target_tags: ['ship'], radar_icon: 'ship' },
        { uuid: 'rock-1', x: 2, z: 0, tags: ['asteroid'], target_tags: ['asteroid'], radar_icon: 'asteroid' },
      ],
      0, 0, 0, 100,
      { shows: ['ship', 'asteroid'], selects: ['ship'] }
    );
    expect(blips.find(b => b.uuid === 'ship-1').selectable).toBe(true);
    expect(blips.find(b => b.uuid === 'rock-1').selectable).toBe(false);
  });

  it('allows active objective targets through the show filter and marks them', () => {
    const blips = buildBlips(
      [{ uuid: 'beacon-1', name: 'Patrol Zone', x: 25, z: 0, tags: ['objective_marker'], radar_icon: 'waypoint' }],
      0, 0, 0, 100,
      { shows: ['ship'] }
    );
    expect(blips).toEqual([]);

    const objectiveBlips = buildBlips(
      [{ uuid: 'beacon-1', name: 'Patrol Zone', x: 25, z: 0, tags: ['objective_marker'], objective_target: true, radar_icon: 'waypoint' }],
      0, 0, 0, 100,
      { shows: ['ship'] }
    );
    expect(objectiveBlips).toHaveLength(1);
    expect(objectiveBlips[0].objective_target).toBe(true);
    expect(objectiveBlips[0].kind).toBe('waypoint');
  });
});

describe('buildRadarRegions', () => {
  it('builds shape overlays and marks active objective regions by name', () => {
    const regions = buildRadarRegions(
      [{
        uuid: 'nebula-1',
        name: 'Kaleth Nebula',
        x: 100,
        z: -50,
        tags: ['region'],
        shape: 'sphere',
        radius: 80,
        region_colour: [0.2, 0.4, 0.8],
      }],
      [{ id: 'obj', text: 'Survey', mandatory: true, status: 'Active', targets: ['Kaleth Nebula'] }]
    );
    expect(regions).toHaveLength(1);
    expect(regions[0]).toMatchObject({
      uuid: 'nebula-1',
      x: 100,
      z: -50,
      shape: 'sphere',
      radius: 80,
      objective_target: true,
    });
  });

  it('builds asteroid field overlays from field radii when shape is omitted', () => {
    const regions = buildRadarRegions([{
      uuid: 'field-1',
      name: 'Main Belt',
      x: 0,
      z: 0,
      tags: ['asteroid_field'],
      radius: 350,
      inner_radius: 300,
      region_colour: [0.52, 0.32, 0.18],
    }]);
    expect(regions).toHaveLength(1);
    expect(regions[0]).toMatchObject({
      uuid: 'field-1',
      shape: 'torus',
      radius: 350,
      inner_radius: 300,
      outer_radius: 350,
      color: [0.52, 0.32, 0.18],
    });
  });
});

describe('buildWaypointBlip', () => {
  it('returns null when waypoint is absent', () => {
    expect(buildWaypointBlip(null, 0, 0, 0, 100)).toBeNull();
  });

  it('projects in-range waypoint without edge flag', () => {
    const blip = buildWaypointBlip({ x: 50, z: 0 }, 0, 0, 0, 100, { rotate: true, edgeClamp: true });
    expect(blip.kind).toBe('waypoint');
    expect(blip.radar_x).toBeCloseTo(0.5);
    expect(blip.radar_y).toBeCloseTo(0);
    expect(blip.edge).toBe(false);
  });

  it('clamps out-of-range waypoint to the edge when requested', () => {
    const blip = buildWaypointBlip({ x: 500, z: 0 }, 0, 0, 0, 100, { rotate: true, edgeClamp: true });
    expect(Math.hypot(blip.radar_x, blip.radar_y)).toBeCloseTo(0.96);
    expect(blip.edge).toBe(true);
  });
});

// ── buildTargetBlip ───────────────────────────────────────────────────────────

const TARGET_ENTITY = { uuid: 'tgt-1', x: 80, z: 0, name: 'Kobayashi Maru' };
const ENTITIES = [TARGET_ENTITY];

describe('buildTargetBlip', () => {
  it('returns null when targetUuid is absent', () => {
    expect(buildTargetBlip(null, ENTITIES, 0, 0, 0, 100)).toBeNull();
  });

  it('returns null when entities list is empty', () => {
    expect(buildTargetBlip('tgt-1', [], 0, 0, 0, 100)).toBeNull();
  });

  it('returns null when target entity is not found', () => {
    expect(buildTargetBlip('unknown-uuid', ENTITIES, 0, 0, 0, 100)).toBeNull();
  });

  it('projects in-range target without edge flag', () => {
    const blip = buildTargetBlip('tgt-1', ENTITIES, 0, 0, 0, 100, { edgeClamp: true });
    expect(blip.uuid).toBe('tgt-1');
    expect(blip.radar_x).toBeCloseTo(0.8);
    expect(blip.radar_y).toBeCloseTo(0);
    expect(blip.edge).toBe(false);
    expect(blip.kind).toBe('target-marker');
  });

  it('clamps out-of-range target to the edge when requested', () => {
    const blip = buildTargetBlip('tgt-1', ENTITIES, 0, 0, 0, 10, { edgeClamp: true });
    expect(Math.hypot(blip.radar_x, blip.radar_y)).toBeCloseTo(0.96);
    expect(blip.edge).toBe(true);
  });

  it('accepts custom kind and color', () => {
    const blip = buildTargetBlip('tgt-1', ENTITIES, 0, 0, 0, 100, {
      kind: 'tactical-target',
      color: [1.0, 0.2, 0.2],
      label: 'TACTICAL TARGET',
    });
    expect(blip.kind).toBe('tactical-target');
    expect(blip.color).toEqual([1.0, 0.2, 0.2]);
    expect(blip.name).toBe('TACTICAL TARGET');
  });

  it('preserves world_x/world_z for canvas rendering', () => {
    const blip = buildTargetBlip('tgt-1', ENTITIES, 0, 0, 0, 100);
    expect(blip.world_x).toBe(80);
    expect(blip.world_z).toBe(0);
  });

  it('falls back to target.name when no label is provided', () => {
    const blip = buildTargetBlip('tgt-1', ENTITIES, 0, 0, 0, 100);
    expect(blip.name).toBe('Kobayashi Maru');
  });

  it('supports rotate=false for world-axis projection', () => {
    const blip = buildTargetBlip('tgt-1', ENTITIES, 10, 20, 0, 100, { rotate: false });
    expect(blip.radar_x).toBeCloseTo(0.7);
    expect(blip.radar_y).toBeCloseTo(-0.2);
  });
});

// ── State builders — all return valid JSON ─────────────────────────────────────

function parse(jsonStr) {
  return JSON.parse(jsonStr);
}

const EMPTY = {};

describe('buildWeaponsConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildWeaponsConsoleState(EMPTY))).not.toThrow();
  });

  it('includes required keys', () => {
    const s = parse(buildWeaponsConsoleState(EMPTY));
    expect(s).toHaveProperty('target_uuid');
    expect(s).toHaveProperty('banks');
    expect(s).toHaveProperty('tubes');
    expect(s).toHaveProperty('blips');
    expect(s).toHaveProperty('phaser_mode');
    expect(s).toHaveProperty('torpedo_max');
  });

  it('torpedo_max falls back to torpedo_count when torpedo-magazine blackboard absent', () => {
    const s = parse(buildWeaponsConsoleState({ weaponsTorpedoCount: 6 }));
    expect(s.torpedo_max).toBe(6);
  });

  it('torpedo_max reads capacity from torpedo-magazine blackboard', () => {
    const s = parse(buildWeaponsConsoleState({
      weaponsTorpedoCount: 4,
      blackboards: { 'torpedo-magazine': { capacity: 12 } },
    }));
    expect(s.torpedo_max).toBe(12);
  });

  it('uses state values when present', () => {
    const s = parse(buildWeaponsConsoleState({
      weaponsTarget: 'tgt-1',
      weaponsTargetName: 'Harrow Patrol',
      weaponsPhaserMode: 'Manual',
      weaponsTorpedoCount: 3,
    }));
    expect(s.target_uuid).toBe('tgt-1');
    expect(s.target_name).toBe('Harrow Patrol');
    expect(s.phaser_mode).toBe('Manual');
    expect(s.torpedo_count).toBe(3);
  });

  it('derives target_name from the locked server blip when no explicit name is stored', () => {
    const s = parse(buildWeaponsConsoleState({
      weaponsTarget: 'srv-1',
      weaponsBlips: [
        { uuid: 'srv-1', radar_x: 0.2, radar_y: 0.1, scaled_radius: 0.02, kind: 'ship', selectable: true, name: 'KSV Nemesis' },
      ],
    }));
    expect(s.target_name).toBe('KSV Nemesis');
  });

  it('blips excludes entities outside WEAPONS_RADAR_RANGE', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      asteroids: [
        { uuid: 'close', x: 1, z: 0, tags: ['asteroid'], radar_icon: 'asteroid' },
        { uuid: 'far', x: WEAPONS_RADAR_RANGE + 1, z: 0, tags: ['asteroid'], radar_icon: 'asteroid' },
      ],
    };
    const s = parse(buildWeaponsConsoleState(state));
    expect(s.blips.map(b => b.uuid)).toEqual(['close']);
  });

  it('uses authoritative server blips when WeaponsUpdate provided them', () => {
    const serverBlips = [
      { uuid: 'srv-1', radar_x: 0.2, radar_y: 0.1, scaled_radius: 0.02, kind: 'ship', selectable: true },
    ];
    const s = parse(buildWeaponsConsoleState({
      weaponsBlips: serverBlips,
      asteroids: [{ uuid: 'fallback-1', x: 1, z: 0, tags: ['asteroid'] }],
    }));
    expect(s.blips).toEqual(serverBlips);
  });

  it('surfaces volley fields from tubes (issue #632)', () => {
    const tubeWithVolley = {
      id: 'fore_port',
      loaded: true,
      reload_secs: 0,
      state: 'loading',
      progress: 0.5,
      load_time: 10,
      volley_max: 4,
      loaded_count: 2,
      target_count: 3,
      load_progress: 0.5,
    };
    const s = parse(buildWeaponsConsoleState({ weaponsTubes: [tubeWithVolley] }));
    expect(s.tubes).toHaveLength(1);
    const t = s.tubes[0];
    expect(t.volley_max).toBe(4);
    expect(t.loaded_count).toBe(2);
    expect(t.target_count).toBe(3);
    expect(t.load_progress).toBe(0.5);
  });

  it('surfaces volley fields from blackboard tubes (issue #632)', () => {
    const tubeWithVolley = {
      id: 'aft',
      loaded: false,
      reload_secs: 5,
      state: 'unloaded',
      progress: 0,
      load_time: 10,
      volley_max: 2,
      loaded_count: 0,
      target_count: 2,
      load_progress: 0,
    };
    const s = parse(buildWeaponsConsoleState({
      blackboards: { tactical: { tubes: [tubeWithVolley] } },
    }));
    expect(s.tubes).toHaveLength(1);
    expect(s.tubes[0].volley_max).toBe(2);
    expect(s.tubes[0].target_count).toBe(2);
  });

  it('blasters defaults to empty array when absent', () => {
    const s = parse(buildWeaponsConsoleState(EMPTY));
    expect(s.blasters).toEqual([]);
  });

  it('passes blasters through from state.blasterBanks', () => {
    const bank = { id: 'fore', fire_ready: true, on_cooldown: false, cooldown_remaining_secs: 0 };
    const s = parse(buildWeaponsConsoleState({ blasterBanks: [bank] }));
    expect(s.blasters).toHaveLength(1);
    expect(s.blasters[0].id).toBe('fore');
    expect(s.blasters[0].fire_ready).toBe(true);
  });

  it('passes blasters through from blackboard', () => {
    const bank = { id: 'aft', fire_ready: false, on_cooldown: true, cooldown_remaining_secs: 1.5 };
    const s = parse(buildWeaponsConsoleState({
      blackboards: { tactical: { blasters: [bank] } },
    }));
    expect(s.blasters).toHaveLength(1);
    expect(s.blasters[0].id).toBe('aft');
    expect(s.blasters[0].on_cooldown).toBe(true);
    expect(s.blasters[0].cooldown_remaining_secs).toBe(1.5);
  });

  it('blackboard blasters take priority over state.blasterBanks', () => {
    const bbBank  = { id: 'bb-bank',  fire_ready: true,  on_cooldown: false, cooldown_remaining_secs: 0 };
    const stBank  = { id: 'st-bank',  fire_ready: false, on_cooldown: true,  cooldown_remaining_secs: 2 };
    const s = parse(buildWeaponsConsoleState({
      blackboards: { tactical: { blasters: [bbBank] } },
      blasterBanks: [stBank],
    }));
    expect(s.blasters).toHaveLength(1);
    expect(s.blasters[0].id).toBe('bb-bank');
  });

  it('passes charge_progress through from blackboard (issue #636)', () => {
    const bank = { id: 'heavy', fire_ready: false, on_cooldown: false, charge_progress: 0.75, has_charge: true };
    const s = parse(buildWeaponsConsoleState({
      blackboards: { tactical: { blasters: [bank] } },
    }));
    expect(s.blasters[0].charge_progress).toBeCloseTo(0.75, 3);
  });

  it('has_charge true surfaces in blaster bank state (issue #636)', () => {
    const bank = { id: 'heavy', fire_ready: true, on_cooldown: false, charge_progress: 0.0, has_charge: true };
    const s = parse(buildWeaponsConsoleState({
      blackboards: { tactical: { blasters: [bank] } },
    }));
    expect(s.blasters[0].has_charge).toBe(true);
  });

  it('has_charge false for instant-fire bank (issue #636)', () => {
    const bank = { id: 'fore', fire_ready: true, on_cooldown: false, charge_progress: 0.0, has_charge: false };
    const s = parse(buildWeaponsConsoleState({
      blackboards: { tactical: { blasters: [bank] } },
    }));
    expect(s.blasters[0].has_charge).toBe(false);
  });

  it('charge_progress defaults to 0 when absent (issue #636)', () => {
    const bank = { id: 'fore', fire_ready: true, on_cooldown: false };
    const s = parse(buildWeaponsConsoleState({ blasterBanks: [bank] }));
    // charge_progress not present → passes through as undefined; treat as falsy
    expect(s.blasters[0].charge_progress == null || s.blasters[0].charge_progress === 0).toBe(true);
  });
});

describe('buildCaptainConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildCaptainConsoleState(EMPTY))).not.toThrow();
  });

  it('red_alert false by default', () => {
    const s = parse(buildCaptainConsoleState(EMPTY));
    expect(s.red_alert).toBe(false);
    expect(s.game_status).toMatch(/nominal/i);
  });

  it('red_alert true changes game_status', () => {
    const s = parse(buildCaptainConsoleState({ redAlert: true }));
    expect(s.red_alert).toBe(true);
    expect(s.game_status).toMatch(/RED ALERT/);
  });

  it('hull_integrity_pct defaults to 100', () => {
    expect(parse(buildCaptainConsoleState(EMPTY)).hull_integrity_pct).toBe(100);
  });

  it('passes through objectives', () => {
    const s = parse(buildCaptainConsoleState({ objectives: ['obj-A'] }));
    expect(s.objectives).toEqual(['obj-A']);
  });

  it('passes currentView as view_direction for all views', () => {
    expect(parse(buildCaptainConsoleState({ currentView: 'Radar' })).view_direction).toBe('Radar');
  });
});

describe('buildHelmConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildHelmConsoleState(EMPTY))).not.toThrow();
  });

  it('heading is in degrees [0, 360)', () => {
    const cases = [
      { yaw: 0,           expectedHeading: 0 },
      { yaw: Math.PI,     expectedHeading: 180 },
      { yaw: Math.PI / 2, expectedHeading: 90 },  // yaw=+90 rad → heading=90°
    ];
    for (const { yaw, expectedHeading } of cases) {
      const s = parse(buildHelmConsoleState({ shipYaw: yaw }));
      expect(s.heading).toBeCloseTo(expectedHeading, 3);
    }
  });

  it('on_screen true when currentView is Radar', () => {
    expect(parse(buildHelmConsoleState({ currentView: 'Radar' })).on_screen).toBe(true);
  });

  it('on_screen false for other views', () => {
    expect(parse(buildHelmConsoleState({ currentView: 'Fore' })).on_screen).toBe(false);
  });

  it('includes active waypoint as a helm radar blip', () => {
    const s = parse(buildHelmConsoleState({
      shipX: 0, shipZ: 0, shipYaw: 0, helmRadarRange: 100,
      navigationWaypoint: { x: 50, z: 0 },
    }));
    const waypoint = s.blips.find(b => b.kind === 'waypoint');
    expect(waypoint).toBeDefined();
    expect(waypoint.edge).toBe(false);
    expect(s.waypoint).toEqual({ x: 50, z: 0 });
  });

  it('edge-clamps active waypoint when outside helm range', () => {
    const s = parse(buildHelmConsoleState({
      shipX: 0, shipZ: 0, shipYaw: 0, helmRadarRange: 100,
      navigationWaypoint: { x: 500, z: 0 },
    }));
    const waypoint = s.blips.find(b => b.kind === 'waypoint');
    expect(waypoint.edge).toBe(true);
    expect(Math.hypot(waypoint.radar_x, waypoint.radar_y)).toBeCloseTo(0.96);
  });

  it('reads yaw and speed from blackboard mirror when present', () => {
    const state = {
      blackboards: {
        helm: { yaw: Math.PI, forward_speed: 99.0, x: 0, z: 0,
                impulse_charge: 0, boost_battery: 0, boost_active: false, boost_enabled: false },
      },
      // legacy props should be ignored when blackboard is present
      shipYaw: 0, forwardSpeed: 0,
    };
    const s = parse(buildHelmConsoleState(state));
    expect(s.heading).toBeCloseTo(180, 2);
    expect(s.speed).toBeCloseTo(99.0, 3);
  });

  it('reads boost state from blackboard mirror', () => {
    const state = {
      blackboards: {
        helm: { yaw: 0, forward_speed: 0, x: 0, z: 0,
                impulse_charge: 0.5, boost_battery: 0.75, boost_active: true, boost_enabled: true },
      },
    };
    const s = parse(buildHelmConsoleState(state));
    expect(s.impulse_charge_progress).toBeCloseTo(0.5, 3);
    expect(s.boost_battery).toBeCloseTo(0.75, 3);
    expect(s.boost_active).toBe(true);
    expect(s.boost_enabled).toBe(true);
  });

  it('falls back to legacy props when blackboard absent', () => {
    const s = parse(buildHelmConsoleState({ shipYaw: Math.PI / 2, forwardSpeed: 33 }));
    expect(s.heading).toBeCloseTo(90, 2);
    expect(s.speed).toBeCloseTo(33, 3);
  });
});

describe('helm engine fields', () => {
  // engine_port_thrust
  it('engine_port_thrust is 0 when no blackboard present', () => {
    expect(parse(buildHelmConsoleState(EMPTY)).engine_port_thrust).toBe(0);
  });

  it('engine_port_thrust reads from helm-engine-port blackboard thrust_fraction', () => {
    const state = {
      blackboards: { 'helm-engine-port': { thrust_fraction: 0.72 } },
    };
    expect(parse(buildHelmConsoleState(state)).engine_port_thrust).toBeCloseTo(0.72);
  });

  // engine_stbd_thrust
  it('engine_stbd_thrust is 0 when no blackboard present', () => {
    expect(parse(buildHelmConsoleState(EMPTY)).engine_stbd_thrust).toBe(0);
  });

  it('engine_stbd_thrust reads from helm-engine-starboard blackboard thrust_fraction', () => {
    const state = {
      blackboards: { 'helm-engine-starboard': { thrust_fraction: 0.55 } },
    };
    expect(parse(buildHelmConsoleState(state)).engine_stbd_thrust).toBeCloseTo(0.55);
  });

  // engine_port_auto — derived from coarse 'helm' station rating
  it('engine_port_auto is true when stationRatings.helm === Backfill', () => {
    expect(parse(buildHelmConsoleState({ stationRatings: { helm: 'Backfill' } })).engine_port_auto).toBe(true);
  });

  it('engine_port_auto is false when stationRatings.helm is a different rating', () => {
    expect(parse(buildHelmConsoleState({ stationRatings: { helm: 'Full' } })).engine_port_auto).toBe(false);
  });

  it('engine_port_auto is false when stationRatings is absent', () => {
    expect(parse(buildHelmConsoleState(EMPTY)).engine_port_auto).toBe(false);
  });

  // engine_stbd_auto — derived from coarse 'helm' station rating
  it('engine_stbd_auto is true when stationRatings.helm === Backfill', () => {
    expect(parse(buildHelmConsoleState({ stationRatings: { helm: 'Backfill' } })).engine_stbd_auto).toBe(true);
  });

  it('engine_stbd_auto is false when stationRatings.helm is a different rating', () => {
    expect(parse(buildHelmConsoleState({ stationRatings: { helm: 'Full' } })).engine_stbd_auto).toBe(false);
  });

  it('engine_stbd_auto is false when stationRatings is absent', () => {
    expect(parse(buildHelmConsoleState(EMPTY)).engine_stbd_auto).toBe(false);
  });

  // both AUTO badges light up together when helm goes to Backfill
  it('engine_port_auto and engine_stbd_auto are both true together on Backfill', () => {
    const s = parse(buildHelmConsoleState({ stationRatings: { helm: 'Backfill' } }));
    expect(s.engine_port_auto).toBe(true);
    expect(s.engine_stbd_auto).toBe(true);
  });
});

describe('buildRepairConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildRepairConsoleState(EMPTY))).not.toThrow();
  });

  it('travel_duration_secs is always 5', () => {
    expect(parse(buildRepairConsoleState(EMPTY)).travel_duration_secs).toBe(5.0);
  });

  it('passes repair teams through', () => {
    const teams = [{ id: 1, location: 'Helm' }];
    expect(parse(buildRepairConsoleState({ repairTeams: teams })).teams).toEqual(teams);
  });

  it('damageable_systems derives from consoleHull (SystemId-keyed post issue #618)', () => {
    // Post issue #618 hull entries carry `.system_id` (lowercase station id).
    // Post issue #619 the legacy `damageable_consoles` Console-keyed wire
    // field is gone entirely.
    const hull = [
      { system_id: 'helm',     current: 14, max_hp: 25 },
      { system_id: 'tactical', current: 25, max_hp: 25 },
      { system_id: 'power',    current:  6, max_hp: 25 },
    ];
    const s = parse(buildRepairConsoleState({ consoleHull: hull }));
    expect(s.damageable_systems).toEqual(['helm', 'tactical', 'power']);
  });

  it('damageable_systems is empty when consoleHull is empty', () => {
    const s = parse(buildRepairConsoleState({ consoleHull: [] }));
    expect(s.damageable_systems).toEqual([]);
  });

  it('damageable_systems is empty when consoleHull is absent', () => {
    const s = parse(buildRepairConsoleState({}));
    expect(s.damageable_systems).toEqual([]);
  });
});

describe('buildPowerConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildPowerConsoleState(EMPTY))).not.toThrow();
  });

  it('all allocations default to 0', () => {
    const s = parse(buildPowerConsoleState(EMPTY));
    expect(s.helm).toBe(0);
    expect(s.weapons).toBe(0);
    expect(s.sensors).toBe(0);
    expect(s.battery_charge).toBe(0);
    expect(s.locked).toBe(false);
  });

  it('passes power values through', () => {
    const s = parse(buildPowerConsoleState({
      powerHelm: 2, powerWeapons: 3, powerSensors: 1, powerBattery: 50, powerLocked: true,
    }));
    expect(s.helm).toBe(2);
    expect(s.weapons).toBe(3);
    expect(s.sensors).toBe(1);
    expect(s.battery_charge).toBe(50);
    expect(s.locked).toBe(true);
  });

  it('reads blackboard groups (PowerGroupId-keyed)', () => {
    const s = parse(buildPowerConsoleState({
      blackboards: {
        power: {
          groups: [
            { id: 'helm',    label: 'HELM',    level: 3, max_level: 4 },
            { id: 'weapons', label: 'WEAPONS', level: 1, max_level: 4 },
          ],
          total: 4, total_max: 8, battery_charge: 25, battery_max: 100, locked: false,
        },
      },
    }));
    expect(s.consoles).toEqual([
      { id: 'helm',    label: 'HELM',    level: 3, max_level: 4 },
      { id: 'weapons', label: 'WEAPONS', level: 1, max_level: 4 },
    ]);
    expect(s.total).toBe(4);
    expect(s.battery_charge).toBe(25);
  });

  it('falls back to empty consoles when groups is missing', () => {
    // A blackboard object without a `groups` field (legacy or upstream bug)
    // must still produce a valid, non-throwing panel state.
    const s = parse(buildPowerConsoleState({
      blackboards: {
        power: {
          total: 2, total_max: 8, battery_charge: 0, battery_max: 100, locked: false,
        },
      },
    }));
    expect(s.consoles).toEqual([]);
  });
});

describe('buildShieldsConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildShieldsConsoleState(EMPTY))).not.toThrow();
  });

  it('grid_status GRID OFFLINE when no shield facings', () => {
    expect(parse(buildShieldsConsoleState(EMPTY)).grid_status).toBe('GRID OFFLINE');
  });

  it('grid_status GRID NOMINAL when facings present', () => {
    const s = parse(buildShieldsConsoleState({ shieldFacings: ['fore', 'aft'] }));
    expect(s.grid_status).toBe('GRID NOMINAL');
  });

  it('target_bearing null when no weaponsTarget', () => {
    expect(parse(buildShieldsConsoleState(EMPTY)).target_bearing).toBeNull();
  });

  it('computes target_bearing from entity position', () => {
    // Target is directly to starboard (+X) from ship at origin
    // atan2(dx=10, -dz=0) = atan2(10,0) = 90°
    const s = parse(buildShieldsConsoleState({
      shipX: 0, shipZ: 0,
      weaponsTarget: 'tgt',
      asteroids: [{ uuid: 'tgt', x: 10, z: 0 }],
    }));
    expect(s.target_bearing).toBeCloseTo(90);
  });

  it('passes priority field through from blackboard facings', () => {
    const state = {
      blackboards: {
        shields: {
          facings: [
            { label: 'Fore', hp: 100, max_hp: 100, online: true, offline_remaining: 0, arc_id: 'fore', center_deg: 0, width_deg: 90, priority: 3 },
            { label: 'Aft',  hp: 80,  max_hp: 100, online: true, offline_remaining: 0, arc_id: 'aft',  center_deg: 180, width_deg: 90, priority: 1 },
          ],
          hull_integrity_pct: 100,
          focused_facing: null,
          target_bearing: null,
          grid_status: 'GRID NOMINAL',
        },
      },
    };
    const s = parse(buildShieldsConsoleState(state));
    expect(s.facings[0].priority).toBe(3);
    expect(s.facings[1].priority).toBe(1);
  });

  it('passes priority field through from legacy shieldFacings path', () => {
    const state = {
      shieldFacings: [
        { label: 'Fore', hp: 100, max_hp: 100, online: true, offline_remaining: 0, arc_id: 'fore', center_deg: 0, width_deg: 90, priority: 2 },
      ],
    };
    const s = parse(buildShieldsConsoleState(state));
    expect(s.facings[0].priority).toBe(2);
  });
});

describe('buildSensorsConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildSensorsConsoleState(EMPTY))).not.toThrow();
  });

  it('scan_range matches SENSORS_RADAR_RANGE constant', () => {
    expect(parse(buildSensorsConsoleState(EMPTY)).scan_range).toBe(SENSORS_RADAR_RANGE);
  });

  it('on_screen is true when currentView is SensorsRadar', () => {
    expect(parse(buildSensorsConsoleState({ currentView: 'SensorsRadar' })).on_screen).toBe(true);
  });

  it('on_screen is false for other views', () => {
    expect(parse(buildSensorsConsoleState({ currentView: 'NavigationChart' })).on_screen).toBe(false);
  });

  it('blips include color, name, stance, faction extra fields', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      asteroids: [{ uuid: 'p', x: 10, z: 0, tags: ['ship'], name: 'Raider', stance: 'hostile', faction: 'pirate', radar_icon: 'ship' }],
    };
    const blips = parse(buildSensorsConsoleState(state)).blips;
    expect(blips[0].name).toBe('Raider');
    expect(blips[0].stance).toBe('hostile');
    expect(blips[0].faction).toBe('pirate');
    expect(blips[0].color).toBeNull();
  });

  it('marks sensor-visible ships selectable and regions untargetable by default', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsRadarShows: ['ship', 'region', 'asteroid_field'],
      sensorsRadarSelects: ['ship'],
      asteroids: [
        { uuid: 'ship-1', x: 10, z: 0, tags: ['ship'], target_tags: ['ship'], radar_icon: 'ship' },
        { uuid: 'region-1', x: 20, z: 0, tags: ['region'], target_tags: ['region'], radar_icon: 'region' },
      ],
    };
    const blips = parse(buildSensorsConsoleState(state)).blips;
    expect(blips.find(b => b.uuid === 'ship-1').selectable).toBe(true);
    expect(blips.find(b => b.uuid === 'region-1').selectable).toBe(false);
  });

  it('projects blips in the same ship-local frame as helm and weapons', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: Math.PI / 2, sensorsRadarRange: 100,
      asteroids: [{ uuid: 'ahead-after-turn', x: 100, z: 0, radius: 1, tags: ['ship'], target_tags: ['ship'], radar_icon: 'ship' }],
    };
    const blip = parse(buildSensorsConsoleState(state)).blips[0];
    expect(blip.radar_x).toBeCloseTo(0);
    expect(blip.radar_y).toBeCloseTo(1);
  });

  it('projects asteroid-field overlays with the same ship-local frame as blips', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: Math.PI / 2, sensorsRadarRange: 100,
      asteroids: [{
        uuid: 'field-1',
        x: 100,
        z: 0,
        tags: ['asteroid_field'],
        radius: 40,
        inner_radius: 20,
        radar_icon: 'field',
        region_colour: [0.52, 0.32, 0.18],
      }],
    };
    const s = parse(buildSensorsConsoleState(state));
    const blip = s.blips.find(b => b.uuid === 'field-1');
    const region = s.regions.find(r => r.uuid === 'field-1');
    expect(region.radar_x).toBeCloseTo(blip.radar_x);
    expect(region.radar_y).toBeCloseTo(blip.radar_y);
    expect(region.scaled_outer_radius).toBeCloseTo(0.4);
    expect(region.scaled_inner_radius).toBeCloseTo(0.2);
  });

  it('excludes objective_marker entities from the sensors radar', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsRadarShows: ['ship'],
      objectives: [{ id: 'obj-1', text: 'Reach patrol zone', mandatory: true, status: 'Active', targets: ['Patrol Zone'] }],
      asteroids: [{ uuid: 'beacon-1', name: 'Patrol Zone', x: 10, z: 0, tags: ['objective_marker'] }],
    };
    const blips = parse(buildSensorsConsoleState(state)).blips;
    // Objective markers are intentionally excluded from the sensors radar —
    // they only appear on the navigation console's system chart.
    expect(blips).toHaveLength(0);
  });

  it('target_uuid and derived fields are null when no sensorsTarget', () => {
    const s = parse(buildSensorsConsoleState(EMPTY));
    expect(s.target_uuid).toBeNull();
    expect(s.target_name).toBeNull();
    expect(s.target_bearing).toBeNull();
  });

  it('resolves target fields when sensorsTarget matches an asteroid uuid', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'target-1',
      asteroids: [{
        uuid: 'target-1', x: 0, z: -100,
        tags: ['ship'], name: 'Patrol', stance: 'neutral', faction: 'harrow',
        hull_pct: 75, yaw: 45 * Math.PI / 180, speed: 10,
      }],
    };
    const s = parse(buildSensorsConsoleState(state));
    expect(s.target_uuid).toBe('target-1');
    expect(s.target_name).toBe('Patrol');
    expect(s.target_kind).toBe('ship');
    expect(s.target_stance).toBe('neutral');
    expect(s.target_faction).toBe('harrow');
    expect(s.target_hull_pct).toBe(75);
    expect(s.target_heading).toBe(45);
    expect(s.target_speed).toBe(10);
    // atan2(dx=0, -dz=100) = 0° (directly ahead = north)
    expect(s.target_bearing).toBeCloseTo(0);
    expect(s.target_range).toBeCloseTo(100);
  });

  it('target_threat is high when stance is hostile', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'e1',
      asteroids: [{ uuid: 'e1', x: 5, z: 0, tags: ['ship'], stance: 'hostile' }],
    };
    expect(parse(buildSensorsConsoleState(state)).target_threat).toBe('high');
  });

  // ── target_shield_fraction (#473) ────────────────────────────────────────

  it('target_shield_fraction is null when target has no shield_fraction field', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'a1',
      asteroids: [{ uuid: 'a1', x: 0, z: 0, tags: ['ship'] }],
    };
    expect(parse(buildSensorsConsoleState(state)).target_shield_fraction).toBeNull();
  });

  it('target_shield_fraction is null when target has shield_fraction = null', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'a1',
      asteroids: [{ uuid: 'a1', x: 0, z: 0, tags: ['ship'], shield_fraction: null }],
    };
    expect(parse(buildSensorsConsoleState(state)).target_shield_fraction).toBeNull();
  });

  it('target_shield_fraction passes through when target has a shield', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'a1',
      asteroids: [{ uuid: 'a1', x: 0, z: 0, tags: ['ship'], shield_fraction: 0.42 }],
    };
    expect(parse(buildSensorsConsoleState(state)).target_shield_fraction).toBe(0.42);
  });

  it('target_shield_fraction is 0 for broken shield', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'a1',
      asteroids: [{ uuid: 'a1', x: 0, z: 0, tags: ['ship'], shield_fraction: 0 }],
    };
    expect(parse(buildSensorsConsoleState(state)).target_shield_fraction).toBe(0);
  });

  it('target_shield_fraction is null when sensorsTarget is unset', () => {
    const state = { shipX: 0, shipZ: 0, shipYaw: 0 };
    expect(parse(buildSensorsConsoleState(state)).target_shield_fraction).toBeNull();
  });
});

describe('buildScienceConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildScienceConsoleState(EMPTY))).not.toThrow();
  });

  it('contains sensors and shields sub-objects', () => {
    const s = parse(buildScienceConsoleState(EMPTY));
    expect(s).toHaveProperty('sensors');
    expect(s).toHaveProperty('shields');
    expect(s.sensors).toHaveProperty('blips');
    expect(s.shields).toHaveProperty('grid_status');
  });

  it('science_auto is true when stationRatings.science === Backfill', () => {
    expect(parse(buildScienceConsoleState({ stationRatings: { science: 'Backfill' } })).science_auto).toBe(true);
  });

  it('science_auto is false when stationRatings.science is a different rating', () => {
    expect(parse(buildScienceConsoleState({ stationRatings: { science: 'Full' } })).science_auto).toBe(false);
  });

  it('science_auto is false when stationRatings is absent', () => {
    expect(parse(buildScienceConsoleState(EMPTY)).science_auto).toBe(false);
  });

  it('passes sensors target state through the nested sensors sub-object', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'tgt-1',
      asteroids: [{ uuid: 'tgt-1', x: 0, z: -100, tags: ['ship'], name: 'Raider', stance: 'hostile', faction: 'pirate' }],
    };
    const s = parse(buildScienceConsoleState(state));
    expect(s.sensors.target_uuid).toBe('tgt-1');
    expect(s.sensors.target_name).toBe('Raider');
    expect(s.sensors.target_stance).toBe('hostile');
    expect(s.sensors.target_faction).toBe('pirate');
  });

  it('passes shields facings through the nested shields sub-object', () => {
    const s = parse(buildScienceConsoleState({ shieldFacings: ['fore', 'port', 'aft', 'starboard'] }));
    expect(s.shields.grid_status).toBe('GRID NOMINAL');
    expect(s.shields.facings).toEqual(['fore', 'port', 'aft', 'starboard']);
  });
});

describe('buildCommsConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildCommsConsoleState({}))).not.toThrow();
  });

  it('messages defaults to empty array', () => {
    expect(parse(buildCommsConsoleState({})).messages).toEqual([]);
  });

  it('contacts defaults to empty array', () => {
    expect(parse(buildCommsConsoleState({})).contacts).toEqual([]);
  });

  it('passes messages through', () => {
    const msgs = [{ id: 'msg-1', sender_name: 'Starbase', subject: 'Hello', body: 'Hi' }];
    expect(parse(buildCommsConsoleState({ commsMessages: msgs })).messages).toEqual(msgs);
  });

  it('passes contacts through', () => {
    const contacts = [{ uuid: 'npc-1', name: 'Station Alpha', in_range: true }];
    expect(parse(buildCommsConsoleState({ commsContacts: contacts })).contacts).toEqual(contacts);
  });

  it('on_screen is true when currentView is Comms', () => {
    expect(parse(buildCommsConsoleState({ currentView: 'Comms' })).on_screen).toBe(true);
  });
});

// ── buildNavigationConsoleState ───────────────────────────────────────────────

describe('buildNavigationConsoleState', () => {
  const EMPTY = {};

  it('returns valid JSON', () => {
    expect(() => parse(buildNavigationConsoleState(EMPTY))).not.toThrow();
  });

  it('radar_range matches NAVIGATION_RADAR_RANGE constant', () => {
    expect(parse(buildNavigationConsoleState(EMPTY)).radar_range).toBe(NAVIGATION_RADAR_RANGE);
  });

  it('blips is empty when no asteroids', () => {
    expect(parse(buildNavigationConsoleState(EMPTY)).blips).toEqual([]);
  });

  it('includes station entities', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['station'],
      asteroids: [{ uuid: 'st1', x: 100, z: 0, tags: ['station'], name: 'Starbase 1', radar_icon: 'station' }],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(1);
    expect(blips[0].kind).toBe('station');
    expect(blips[0].name).toBe('Starbase 1');
  });

  it('includes planet and star entities', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['planet', 'star'],
      asteroids: [
        { uuid: 'p1', x: 50, z: 0,  tags: ['planet'], radar_icon: 'planet' },
        { uuid: 's1', x: 0,  z: 50, tags: ['star'],   radar_icon: 'star'   },
      ],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(2);
    expect(blips.map(b => b.kind).sort()).toEqual(['planet', 'star']);
  });

  it('draws a radar_icon star as a star on the navigation chart', () => {
    const state = {
      shipX: 0, shipZ: 0,
      asteroids: [
        { uuid: 'sun', name: 'Sun', x: 0, z: 50, tags: ['unknown'], radar_icon: 'star', objective_target: true },
      ],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips).toHaveLength(1);
    expect(blips[0].kind).toBe('star');
    expect(blips[0].icon).toBe('star');
  });

  it('excludes bare asteroid entities', () => {
    const state = {
      shipX: 0, shipZ: 0,
      asteroids: [{ uuid: 'a1', x: 10, z: 0, tags: ['asteroid'] }],
    };
    expect(parse(buildNavigationConsoleState(state)).blips).toEqual([]);
  });

  it('excludes NPC ship entities (ship tag only)', () => {
    const state = {
      shipX: 0, shipZ: 0,
      asteroids: [{ uuid: 'npc1', x: 10, z: 0, tags: ['ship'] }],
    };
    expect(parse(buildNavigationConsoleState(state)).blips).toEqual([]);
  });

  it('includes alliance_cruiser entities', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['alliance_cruiser'],
      asteroids: [{ uuid: 'ps1', x: 5, z: 0, tags: ['alliance_cruiser'], radar_icon: 'ship' }],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(1);
    expect(blips[0].kind).toBe('ship');
  });

  it('cancel_visible is true when impulse_charge_progress > 0', () => {
    const s = parse(buildNavigationConsoleState({ impulseChargeProgress: 0.5 }));
    expect(s.cancel_visible).toBe(true);
    expect(s.impulse_charge_progress).toBeCloseTo(0.5);
  });

  it('cancel_visible is false when charge is 0', () => {
    expect(parse(buildNavigationConsoleState(EMPTY)).cancel_visible).toBe(false);
  });

  it('on_screen is true when currentView is NavigationChart', () => {
    expect(parse(buildNavigationConsoleState({ currentView: 'NavigationChart' })).on_screen).toBe(true);
  });

  it('on_screen is false for other views', () => {
    expect(parse(buildNavigationConsoleState({ currentView: 'Radar' })).on_screen).toBe(false);
  });

  it('passes waypoint through and adds a waypoint blip', () => {
    const s = parse(buildNavigationConsoleState({
      shipX: 0, shipZ: 0,
      navigationWaypoint: { x: 250, z: 500 },
    }));
    expect(s.waypoint).toEqual({ x: 250, z: 500 });
    expect(s.blips.find(b => b.kind === 'waypoint')).toBeDefined();
  });

  it('blips include world_x and world_z for canvas rendering', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['station', 'planet'],
      asteroids: [
        { uuid: 'st1', x: 500, z: -300, tags: ['station'], radar_icon: 'station' },
        { uuid: 'pl1', x: -200, z: 400, tags: ['planet'], radar_icon: 'planet' },
      ],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(2);
    expect(blips[0].world_x).toBe(500);
    expect(blips[0].world_z).toBe(-300);
    expect(blips[1].world_x).toBe(-200);
    expect(blips[1].world_z).toBe(400);
  });

  it('blips include world_x and world_z coordinates', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['station', 'planet'],
      asteroids: [
        { uuid: 'st1', x: 500, z: 300, tags: ['station'], name: 'Starbase 1', radar_icon: 'station' },
        { uuid: 'p1',  x: 200, z: 800, tags: ['planet'],  name: 'Alderaan',  radar_icon: 'planet'  },
      ],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(2);
    expect(blips[0].world_x).toBe(500);
    expect(blips[0].world_z).toBe(300);
    expect(blips[1].world_x).toBe(200);
    expect(blips[1].world_z).toBe(800);
  });

  it('blips use world-axis (north-up) projection — no ship yaw rotation', () => {
    // Entity directly east (x+) of ship → radar_x positive, radar_y ≈ 0
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['station'],
      asteroids: [{ uuid: 'st', x: 500, z: 0, tags: ['station'], radar_icon: 'station' }],
    };
    const blip = parse(buildNavigationConsoleState(state)).blips[0];
    expect(blip.radar_x).toBeCloseTo(500 / NAVIGATION_RADAR_RANGE);
    expect(blip.radar_y).toBeCloseTo(0);
  });

  it('waypoint blip carries world_x/world_z for canvas rendering', () => {
    const state = {
      shipX: 100, shipZ: 200,
      navigationWaypoint: { x: 800, z: -400 },
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    const wp = blips.find(b => b.kind === 'waypoint');
    expect(wp).toBeDefined();
    expect(wp.world_x).toBe(800);
    expect(wp.world_z).toBe(-400);
  });

  it('free waypoint is non-selectable and source_uuid is null', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navigationWaypoint: { x: 500, z: -300 },
    };
    const out = parse(buildNavigationConsoleState(state));
    const wp = out.blips.find(b => b.kind === 'waypoint');
    expect(wp).toBeDefined();
    expect(wp.selectable).toBe(false);
    expect(wp.source_uuid).toBeNull();
  });

  it('anchored waypoint forwards source_uuid and is selectable', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navigationWaypoint: { x: 500, z: -300, source_uuid: 'station-alpha' },
    };
    const out = parse(buildNavigationConsoleState(state));
    expect(out.waypoint).toEqual({ x: 500, z: -300, source_uuid: 'station-alpha' });
    const wp = out.blips.find(b => b.kind === 'waypoint');
    expect(wp).toBeDefined();
    expect(wp.source_uuid).toBe('station-alpha');
    expect(wp.selectable).toBe(true);
  });

  it('includes active objective marker beacons and hides inactive ones', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['station'],
      objectives: [{ id: 'obj-1', text: 'Find zone', mandatory: true, status: 'Active', targets: ['Patrol Zone'] }],
      asteroids: [
        { uuid: 'beacon-1', name: 'Patrol Zone', x: 100, z: 0, tags: ['objective_marker'], radar_icon: 'station' },
        { uuid: 'beacon-2', name: 'Quiet Zone', x: 200, z: 0, tags: ['objective_marker'], radar_icon: 'station' },
      ],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.map(b => b.uuid)).toEqual(['beacon-1']);
    expect(blips[0].objective_target).toBe(true);
    expect(blips[0].kind).toBe('station');
    expect(blips[0].icon).toBe('station');
  });

  it('emits region overlays for active objective regions', () => {
    const state = {
      shipX: 0, shipZ: 0,
      objectives: [{ id: 'obj-1', text: 'Survey nebula', mandatory: true, status: 'Active', targets: ['Kaleth Nebula'] }],
      asteroids: [{
        uuid: 'region-1',
        name: 'Kaleth Nebula',
        x: 100,
        z: 100,
        tags: ['region'],
        shape: 'sphere',
        radius: 50,
        region_colour: [0.3, 0.6, 0.9],
      }],
    };
    const s = parse(buildNavigationConsoleState(state));
    expect(s.regions).toHaveLength(1);
    expect(s.regions[0].objective_target).toBe(true);
  });

  it('emits asteroid field and nebula region overlays on the navigation screen', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['asteroid_field', 'nebula'],
      asteroids: [
        {
          uuid: 'field-1',
          name: 'Main Belt',
          x: 1200,
          z: -200,
          tags: ['field', 'asteroid_field'],
          radius: 350,
          inner_radius: 300,
          region_colour: [0.52, 0.32, 0.18],
        },
        {
          uuid: 'nebula-1',
          name: 'Kaleth Nebula',
          x: 680,
          z: -440,
          tags: ['region', 'nebula'],
          shape: 'sphere',
          radius: 220,
          region_colour: [0.2, 0.4, 0.8],
        },
      ],
    };
    const s = parse(buildNavigationConsoleState(state));
    expect(s.regions.map(r => r.uuid).sort()).toEqual(['field-1', 'nebula-1']);
    expect(s.regions.find(r => r.uuid === 'field-1')).toMatchObject({
      shape: 'torus',
      inner_radius: 300,
      outer_radius: 350,
    });
    expect(s.regions.find(r => r.uuid === 'nebula-1')).toMatchObject({
      shape: 'sphere',
      radius: 220,
    });
  });

  // ── Production-path regression: Welcome → client.html mirror → builder ───
  //
  // The Navigation builder has a two-stage filter (outer entity filter on
  // `navChartShows`, then the inner `buildBlips` filter). If `client.html`'s
  // Welcome-handler mirror block forgets to copy `navChartShows` /
  // `navChartSelects` / `navChartRange` from `window.simState` onto the
  // plain `state` object passed into the builder, the outer filter sees
  // `navChartShows === undefined` and silently drops every non-objective
  // entity — leaving the navigation chart blank.
  //
  // This test exercises that exact pipeline end-to-end (sans the iframe
  // transport): it applies a real Welcome payload via `ClientSimState`,
  // then mirrors only the keys `client.html` actually copies, and asserts
  // that asteroids / stations / planets land in the blip list. Setting
  // `state.navChartShows` directly (as every other test in this file does)
  // would hide the very gap this test exists to catch.
  it('blips arrive when state is built via the client.html Welcome mirror path', () => {
    const sim = new ClientSimState();
    const shipConfig = {
      nav_chart_range: 800,
      nav_chart_shows:   ['asteroid', 'station', 'planet', 'star'],
      nav_chart_selects: ['station', 'planet'],
    };
    const world = {
      entities: [
        { uuid: 'st1', position: [500, 0, -300], tags: ['station'], name: 'Starbase 1', radar_icon: 'station' },
        { uuid: 'pl1', position: [-200, 0, 400], tags: ['planet'],  name: 'Sol III',    radar_icon: 'planet'  },
      ],
      scenario_title: '',
      scenario_description: '',
    };
    sim.apply({
      type: 'Welcome',
      data: {
        state: { phase: 'InProgress', players: [], complexity: {}, world },
        ship_stations: { configs: {}, min_players: 0, max_players: 0 },
        ship_config: shipConfig,
      },
    });

    // Mirror EXACTLY what client.html's Welcome handler copies onto `state`.
    // Keep this in lock-step with the mirror block in client.html — if a new
    // ShipClientConfig field is added there, mirror it here too.
    const state = {
      asteroids:           sim.world.entities,
      repairTeams:         sim.repairTeams,
      weaponsRadarRange:   sim.weaponsRadarRange,
      helmRadarRange:      sim.helmRadarRange,
      sensorsRadarRange:   sim.sensorsRadarRange,
      tacticalRadarShows:  sim.tacticalRadarShows,
      tacticalRadarSelects: sim.tacticalRadarSelects,
      sensorsRadarShows:   sim.sensorsRadarShows,
      sensorsRadarSelects: sim.sensorsRadarSelects,
      navChartShows:       sim.navChartShows,
      navChartSelects:     sim.navChartSelects,
      navChartRange:       sim.navChartRange,
      phaserArcConfigs:    sim.phaserArcConfigs,
      torpedoArcConfigs:   sim.torpedoArcConfigs,
      navigationWaypoint:  sim.navigationWaypoint,
      shipX: 0, shipZ: 0,
    };

    const out = parse(buildNavigationConsoleState(state));
    expect(out.blips.length).toBeGreaterThan(0);
    expect(out.blips.map(b => b.uuid).sort()).toEqual(['pl1', 'st1']);
    expect(out.radar_range).toBe(800);
  });
});

// ── auto fields ───────────────────────────────────────────────────────────────

describe('auto fields', () => {
  // helm_auto
  it('helm_auto is true when stationRatings.helm === Backfill', () => {
    expect(parse(buildHelmConsoleState({ stationRatings: { helm: 'Backfill' } })).helm_auto).toBe(true);
  });

  it('helm_auto is false when stationRatings.helm is a different rating', () => {
    expect(parse(buildHelmConsoleState({ stationRatings: { helm: 'Full' } })).helm_auto).toBe(false);
  });

  it('helm_auto is false when stationRatings is absent', () => {
    expect(parse(buildHelmConsoleState(EMPTY)).helm_auto).toBe(false);
  });

  // tactical_auto — per-system: every phaser system id (from stationSystems.tactical)
  // must be controlled by 'Ai' in controlSources.
  it('tactical_auto is true when every phaser system is Ai-controlled', () => {
    const state = {
      stationSystems: { tactical: ['phaser-fore', 'phaser-aft'] },
      controlSources: { 'phaser-fore': 'Ai', 'phaser-aft': 'Ai' },
    };
    expect(parse(buildWeaponsConsoleState(state)).tactical_auto).toBe(true);
  });

  it('tactical_auto is false when only some phaser systems are Ai-controlled', () => {
    const state = {
      stationSystems: { tactical: ['phaser-fore', 'phaser-aft'] },
      controlSources: { 'phaser-fore': 'Ai', 'phaser-aft': 'Human' },
    };
    expect(parse(buildWeaponsConsoleState(state)).tactical_auto).toBe(false);
  });

  it('tactical_auto is false when stationSystems/controlSources are absent', () => {
    expect(parse(buildWeaponsConsoleState(EMPTY)).tactical_auto).toBe(false);
  });

  // repair_auto — per-system: the literal 'repair' system id must be Ai-controlled.
  it('repair_auto is true when controlSources.repair === Ai', () => {
    expect(parse(buildRepairConsoleState({ controlSources: { repair: 'Ai' } })).repair_auto).toBe(true);
  });

  it('repair_auto is false when controlSources.repair is a different value', () => {
    expect(parse(buildRepairConsoleState({ controlSources: { repair: 'Human' } })).repair_auto).toBe(false);
  });

  it('repair_auto is false when controlSources is absent', () => {
    expect(parse(buildRepairConsoleState(EMPTY)).repair_auto).toBe(false);
  });

  // power_auto
  it('power_auto is true when stationRatings.power === Backfill', () => {
    expect(parse(buildPowerConsoleState({ stationRatings: { power: 'Backfill' } })).power_auto).toBe(true);
  });

  it('power_auto is false when stationRatings.power is a different rating', () => {
    expect(parse(buildPowerConsoleState({ stationRatings: { power: 'Full' } })).power_auto).toBe(false);
  });

  it('power_auto is false when stationRatings is absent', () => {
    expect(parse(buildPowerConsoleState(EMPTY)).power_auto).toBe(false);
  });

  // shields_auto
  it('shields_auto is true when stationRatings.shields === Backfill', () => {
    expect(parse(buildShieldsConsoleState({ stationRatings: { shields: 'Backfill' } })).shields_auto).toBe(true);
  });

  it('shields_auto is false when stationRatings.shields is a different rating', () => {
    expect(parse(buildShieldsConsoleState({ stationRatings: { shields: 'Full' } })).shields_auto).toBe(false);
  });

  it('shields_auto is false when stationRatings is absent', () => {
    expect(parse(buildShieldsConsoleState(EMPTY)).shields_auto).toBe(false);
  });

  // sensors_auto
  it('sensors_auto is true when stationRatings.sensors === Backfill', () => {
    expect(parse(buildSensorsConsoleState({ stationRatings: { sensors: 'Backfill' } })).sensors_auto).toBe(true);
  });

  it('sensors_auto is false when stationRatings.sensors is a different rating', () => {
    expect(parse(buildSensorsConsoleState({ stationRatings: { sensors: 'Full' } })).sensors_auto).toBe(false);
  });

  it('sensors_auto is false when stationRatings is absent', () => {
    expect(parse(buildSensorsConsoleState(EMPTY)).sensors_auto).toBe(false);
  });

  // navigation_auto
  it('navigation_auto is true when stationRatings.navigation === Backfill', () => {
    expect(parse(buildNavigationConsoleState({ stationRatings: { navigation: 'Backfill' } })).navigation_auto).toBe(true);
  });

  it('navigation_auto is false when stationRatings.navigation is a different rating', () => {
    expect(parse(buildNavigationConsoleState({ stationRatings: { navigation: 'Full' } })).navigation_auto).toBe(false);
  });

  it('navigation_auto is false when stationRatings is absent', () => {
    expect(parse(buildNavigationConsoleState(EMPTY)).navigation_auto).toBe(false);
  });

  // comms_auto
  it('comms_auto is true when stationRatings.comms === Backfill', () => {
    expect(parse(buildCommsConsoleState({ stationRatings: { comms: 'Backfill' } })).comms_auto).toBe(true);
  });

  it('comms_auto is false when stationRatings.comms is a different rating', () => {
    expect(parse(buildCommsConsoleState({ stationRatings: { comms: 'Full' } })).comms_auto).toBe(false);
  });

  it('comms_auto is false when stationRatings is absent', () => {
    expect(parse(buildCommsConsoleState(EMPTY)).comms_auto).toBe(false);
  });
});

// ── buildEngineeringConsoleState (issue #627) ─────────────────────────────────

describe('buildEngineeringConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildEngineeringConsoleState(EMPTY))).not.toThrow();
  });

  it('contains power and repair sub-objects', () => {
    const s = parse(buildEngineeringConsoleState(EMPTY));
    expect(s).toHaveProperty('power');
    expect(s).toHaveProperty('repair');
  });

  it('power sub-object has expected fields', () => {
    const s = parse(buildEngineeringConsoleState(EMPTY));
    // buildPowerConsoleState returns either consoles+total or helm+weapons+sensors depending on state
    expect(s.power).toBeDefined();
  });

  it('repair sub-object has teams field', () => {
    const s = parse(buildEngineeringConsoleState(EMPTY));
    expect(s.repair).toHaveProperty('teams');
    expect(Array.isArray(s.repair.teams)).toBe(true);
  });

  it('engineering_auto is true when stationRatings.engineering === Backfill', () => {
    const s = parse(buildEngineeringConsoleState({ stationRatings: { engineering: 'Backfill' } }));
    expect(s.engineering_auto).toBe(true);
  });

  it('engineering_auto is false when stationRatings.engineering is a different rating', () => {
    const s = parse(buildEngineeringConsoleState({ stationRatings: { engineering: 'Full' } }));
    expect(s.engineering_auto).toBe(false);
  });

  it('engineering_auto is false when stationRatings is absent', () => {
    expect(parse(buildEngineeringConsoleState(EMPTY)).engineering_auto).toBe(false);
  });

  it('passes power blackboard state through the nested power sub-object', () => {
    const state = {
      blackboards: {
        power: {
          groups: [{ id: 'helm', label: 'HELM', level: 3, max_level: 4 }],
          total: 3,
          total_max: 8,
          battery_charge: 75,
          battery_max: 100,
          locked: false,
        },
      },
    };
    const s = parse(buildEngineeringConsoleState(state));
    expect(s.power.consoles).toHaveLength(1);
    expect(s.power.consoles[0].id).toBe('helm');
    expect(s.power.battery_charge).toBe(75);
  });

  it('passes repair blackboard state through the nested repair sub-object', () => {
    const state = {
      blackboards: {
        repair: {
          teams: ['Idle', 'Idle'],
          system_hull: [{ system_id: 'helm', current: 20, max_hp: 25 }],
          damageable_systems: ['helm'],
          travel_duration_secs: 5.0,
        },
      },
    };
    const s = parse(buildEngineeringConsoleState(state));
    expect(s.repair.teams).toHaveLength(2);
    expect(s.repair.system_hull[0].system_id).toBe('helm');
  });
});

// ── buildCruiserCommsConsoleState (issue #627) ────────────────────────────────

describe('buildCruiserCommsConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildCruiserCommsConsoleState(EMPTY))).not.toThrow();
  });

  it('contains navigation and comms sub-objects', () => {
    const s = parse(buildCruiserCommsConsoleState(EMPTY));
    expect(s).toHaveProperty('navigation');
    expect(s).toHaveProperty('comms');
  });

  it('navigation sub-object has blips and waypoint fields', () => {
    const s = parse(buildCruiserCommsConsoleState(EMPTY));
    expect(s.navigation).toHaveProperty('blips');
    expect(s.navigation).toHaveProperty('waypoint');
  });

  it('comms sub-object has messages and contacts fields', () => {
    const s = parse(buildCruiserCommsConsoleState(EMPTY));
    expect(s.comms).toHaveProperty('messages');
    expect(s.comms).toHaveProperty('contacts');
    expect(Array.isArray(s.comms.messages)).toBe(true);
    expect(Array.isArray(s.comms.contacts)).toBe(true);
  });

  it('comms_auto is true when stationRatings.comms === Backfill', () => {
    expect(parse(buildCruiserCommsConsoleState({ stationRatings: { comms: 'Backfill' } })).comms_auto).toBe(true);
  });

  it('comms_auto is false when stationRatings.comms is a different rating', () => {
    expect(parse(buildCruiserCommsConsoleState({ stationRatings: { comms: 'Full' } })).comms_auto).toBe(false);
  });

  it('comms_auto is false when stationRatings is absent', () => {
    expect(parse(buildCruiserCommsConsoleState(EMPTY)).comms_auto).toBe(false);
  });

  it('passes navigation blackboard through the nested navigation sub-object', () => {
    const state = {
      blackboards: {
        navigation: {
          nav_chart_range: 1000,
          nav_chart_shows: ['ship', 'station'],
          nav_chart_selects: ['station'],
        },
      },
      shipX: 10,
      shipZ: 20,
    };
    const s = parse(buildCruiserCommsConsoleState(state));
    expect(s.navigation.ship_x).toBe(10);
    expect(s.navigation.ship_z).toBe(20);
    expect(s.navigation.radar_range).toBe(1000);
  });

  it('passes comms blackboard messages through the nested comms sub-object', () => {
    const msgs = [{ id: 'msg-1', sender_name: 'Starbase', subject: 'Hello', body: 'Hi' }];
    const state = {
      blackboards: {
        comms: { messages: msgs, contacts: [], objectives: [] },
      },
    };
    const s = parse(buildCruiserCommsConsoleState(state));
    expect(s.comms.messages).toHaveLength(1);
    expect(s.comms.messages[0].id).toBe('msg-1');
  });
});

// ── aggregateStationHull (issue #625) ────────────────────────────────────────

describe('aggregateStationHull', () => {
  const hull = [
    { system_id: 'helm', display_name: 'Helm', current: 20, max_hp: 25, tier: 'Operational' },
    { system_id: 'helm-engine-port', display_name: 'Port Engine', current: 15, max_hp: 25, tier: 'Damaged' },
  ];
  const stationSystems = { helm: ['helm', 'helm-engine-port'], tactical: ['tactical'] };

  it('returns entries for the given station', () => {
    const agg = aggregateStationHull('helm', hull, stationSystems);
    expect(agg.entries).toHaveLength(2);
    expect(agg.entries.map(e => e.system_id)).toContain('helm');
    expect(agg.entries.map(e => e.system_id)).toContain('helm-engine-port');
  });

  it('computes totalCurrent and totalMax correctly', () => {
    const agg = aggregateStationHull('helm', hull, stationSystems);
    expect(agg.totalCurrent).toBe(35);
    expect(agg.totalMax).toBe(50);
  });

  it('computes pct as current/max', () => {
    const agg = aggregateStationHull('helm', hull, stationSystems);
    expect(agg.pct).toBeCloseTo(0.7);
  });

  it('computes damagePct as 1 - pct', () => {
    const agg = aggregateStationHull('helm', hull, stationSystems);
    expect(agg.damagePct).toBeCloseTo(0.3);
  });

  it('damaged system reduces pct but remains in denominator', () => {
    const hullWithDamaged = [
      { system_id: 'helm', display_name: 'Helm', current: 25, max_hp: 25, tier: 'Operational' },
      { system_id: 'helm-engine-port', display_name: 'Port Engine', current: 5, max_hp: 25, tier: 'Damaged' },
    ];
    const agg = aggregateStationHull('helm', hullWithDamaged, stationSystems);
    // max_hp: 50 (both systems still in denominator)
    expect(agg.totalMax).toBe(50);
    expect(agg.totalCurrent).toBe(30);
    expect(agg.pct).toBeCloseTo(0.6);
  });

  it('destroyed system current=0 but max_hp still in denominator', () => {
    const hullWithDestroyed = [
      { system_id: 'helm', display_name: 'Helm', current: 25, max_hp: 25, tier: 'Operational' },
      { system_id: 'helm-engine-port', display_name: 'Port Engine', current: 0, max_hp: 25, tier: 'Destroyed' },
    ];
    const agg = aggregateStationHull('helm', hullWithDestroyed, stationSystems);
    expect(agg.totalMax).toBe(50);
    expect(agg.totalCurrent).toBe(25);
    expect(agg.pct).toBeCloseTo(0.5);
  });

  it('returns pct=1 when stationSystems is empty for that station', () => {
    const agg = aggregateStationHull('comms', hull, stationSystems);
    expect(agg.entries).toHaveLength(0);
    expect(agg.totalMax).toBe(0);
    expect(agg.pct).toBe(1);
    expect(agg.damagePct).toBe(0);
  });

  it('falls back gracefully when stationSystems is null/undefined', () => {
    const agg = aggregateStationHull('helm', hull, null);
    expect(agg.entries).toHaveLength(0);
    expect(agg.pct).toBe(1);
  });

  it('falls back gracefully when consoleHull is null/undefined', () => {
    const agg = aggregateStationHull('helm', null, stationSystems);
    expect(agg.entries).toHaveLength(0);
    expect(agg.totalMax).toBe(0);
    expect(agg.pct).toBe(1);
  });
});

// ── buildDestroyerCaptainConsoleState (issue #628) ────────────────────────────

describe('buildDestroyerCaptainConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildDestroyerCaptainConsoleState(EMPTY))).not.toThrow();
  });

  it('contains captain and sensors sub-objects', () => {
    const s = parse(buildDestroyerCaptainConsoleState(EMPTY));
    expect(s).toHaveProperty('captain');
    expect(s).toHaveProperty('sensors');
  });

  it('captain sub-object has red_alert field', () => {
    const s = parse(buildDestroyerCaptainConsoleState(EMPTY));
    expect(s.captain).toHaveProperty('red_alert');
    expect(s.captain.red_alert).toBe(false);
  });

  it('sensors sub-object has blips field', () => {
    const s = parse(buildDestroyerCaptainConsoleState(EMPTY));
    expect(s.sensors).toHaveProperty('blips');
    expect(Array.isArray(s.sensors.blips)).toBe(true);
  });

  it('captain_auto is true when stationRatings.captain === Backfill', () => {
    const s = parse(buildDestroyerCaptainConsoleState({ stationRatings: { captain: 'Backfill' } }));
    expect(s.captain_auto).toBe(true);
  });

  it('captain_auto is false when stationRatings.captain is a different rating', () => {
    const s = parse(buildDestroyerCaptainConsoleState({ stationRatings: { captain: 'Full' } }));
    expect(s.captain_auto).toBe(false);
  });

  it('captain_auto is false when stationRatings is absent', () => {
    expect(parse(buildDestroyerCaptainConsoleState(EMPTY)).captain_auto).toBe(false);
  });

  it('passes captain state through the nested captain sub-object', () => {
    const s = parse(buildDestroyerCaptainConsoleState({ redAlert: true }));
    expect(s.captain.red_alert).toBe(true);
    expect(s.captain.game_status).toMatch(/RED ALERT/);
  });

  it('passes sensors target state through the nested sensors sub-object', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      sensorsTarget: 'tgt-1',
      asteroids: [{ uuid: 'tgt-1', x: 0, z: -100, tags: ['ship'], name: 'Raider', stance: 'hostile', faction: 'pirate', radar_icon: 'ship' }],
    };
    const s = parse(buildDestroyerCaptainConsoleState(state));
    expect(s.sensors.target_uuid).toBe('tgt-1');
    expect(s.sensors.target_name).toBe('Raider');
  });
});

// ── buildDestroyerTacticalConsoleState (issue #628) ───────────────────────────

describe('buildDestroyerTacticalConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildDestroyerTacticalConsoleState(EMPTY))).not.toThrow();
  });

  it('contains weapons, navigation, and comms sub-objects', () => {
    const s = parse(buildDestroyerTacticalConsoleState(EMPTY));
    expect(s).toHaveProperty('weapons');
    expect(s).toHaveProperty('navigation');
    expect(s).toHaveProperty('comms');
  });

  it('weapons sub-object has banks, tubes, and blips fields', () => {
    const s = parse(buildDestroyerTacticalConsoleState(EMPTY));
    expect(s.weapons).toHaveProperty('banks');
    expect(s.weapons).toHaveProperty('tubes');
    expect(s.weapons).toHaveProperty('blips');
  });

  it('navigation sub-object has blips and waypoint fields', () => {
    const s = parse(buildDestroyerTacticalConsoleState(EMPTY));
    expect(s.navigation).toHaveProperty('blips');
    expect(s.navigation).toHaveProperty('waypoint');
  });

  it('comms sub-object has messages and contacts fields', () => {
    const s = parse(buildDestroyerTacticalConsoleState(EMPTY));
    expect(s.comms).toHaveProperty('messages');
    expect(s.comms).toHaveProperty('contacts');
    expect(Array.isArray(s.comms.messages)).toBe(true);
    expect(Array.isArray(s.comms.contacts)).toBe(true);
  });

  it('tactical_auto is true when the phaser system is Ai-controlled', () => {
    const s = parse(buildDestroyerTacticalConsoleState({
      stationSystems: { tactical: ['phaser-omni'] },
      controlSources: { 'phaser-omni': 'Ai' },
    }));
    expect(s.tactical_auto).toBe(true);
  });

  it('tactical_auto is false when the phaser system is not Ai-controlled', () => {
    const s = parse(buildDestroyerTacticalConsoleState({
      stationSystems: { tactical: ['phaser-omni'] },
      controlSources: { 'phaser-omni': 'Human' },
    }));
    expect(s.tactical_auto).toBe(false);
  });

  it('tactical_auto is false when stationRatings is absent', () => {
    expect(parse(buildDestroyerTacticalConsoleState(EMPTY)).tactical_auto).toBe(false);
  });

  it('passes weapons blackboard through the nested weapons sub-object', () => {
    const state = {
      blackboards: {
        tactical: {
          banks: [{ id: 'fore', ready: true }],
          tubes: [],
          torpedo_count: 4,
          phaser_mode: 'Manual',
          blips: [],
          regions: [],
          phaser_arcs: [],
          torpedo_arcs: [],
          target_uuid: null,
          target_name: null,
        },
      },
    };
    const s = parse(buildDestroyerTacticalConsoleState(state));
    expect(s.weapons.banks).toHaveLength(1);
    expect(s.weapons.banks[0].id).toBe('fore');
    expect(s.weapons.torpedo_count).toBe(4);
    expect(s.weapons.phaser_mode).toBe('Manual');
  });

  it('passes comms messages through the nested comms sub-object', () => {
    const msgs = [{ id: 'msg-1', sender_name: 'Starbase', subject: 'Hello', body: 'Hi' }];
    const state = {
      blackboards: {
        comms: { messages: msgs, contacts: [], objectives: [] },
      },
    };
    const s = parse(buildDestroyerTacticalConsoleState(state));
    expect(s.comms.messages).toHaveLength(1);
    expect(s.comms.messages[0].id).toBe('msg-1');
  });

  it('passes navigation blackboard through the nested navigation sub-object', () => {
    const state = {
      blackboards: {
        navigation: {
          nav_chart_range: 800,
          nav_chart_shows: ['station', 'planet'],
          nav_chart_selects: ['station'],
        },
      },
      shipX: 10,
      shipZ: 20,
    };
    const s = parse(buildDestroyerTacticalConsoleState(state));
    expect(s.navigation.ship_x).toBe(10);
    expect(s.navigation.ship_z).toBe(20);
    expect(s.navigation.radar_range).toBe(800);
  });
});

// ── buildDestroyerEngineeringConsoleState (issue #628) ────────────────────────

describe('buildDestroyerEngineeringConsoleState', () => {
  it('returns valid JSON', () => {
    expect(() => parse(buildDestroyerEngineeringConsoleState(EMPTY))).not.toThrow();
  });

  it('contains shields, power, and repair sub-objects', () => {
    const s = parse(buildDestroyerEngineeringConsoleState(EMPTY));
    expect(s).toHaveProperty('shields');
    expect(s).toHaveProperty('power');
    expect(s).toHaveProperty('repair');
  });

  it('shields sub-object has grid_status field', () => {
    const s = parse(buildDestroyerEngineeringConsoleState(EMPTY));
    expect(s.shields).toHaveProperty('grid_status');
  });

  it('power sub-object is defined', () => {
    const s = parse(buildDestroyerEngineeringConsoleState(EMPTY));
    expect(s.power).toBeDefined();
  });

  it('repair sub-object has teams field', () => {
    const s = parse(buildDestroyerEngineeringConsoleState(EMPTY));
    expect(s.repair).toHaveProperty('teams');
    expect(Array.isArray(s.repair.teams)).toBe(true);
  });

  it('engineering_auto is true when stationRatings.engineering === Backfill', () => {
    const s = parse(buildDestroyerEngineeringConsoleState({ stationRatings: { engineering: 'Backfill' } }));
    expect(s.engineering_auto).toBe(true);
  });

  it('engineering_auto is false when stationRatings.engineering is a different rating', () => {
    const s = parse(buildDestroyerEngineeringConsoleState({ stationRatings: { engineering: 'Full' } }));
    expect(s.engineering_auto).toBe(false);
  });

  it('engineering_auto is false when stationRatings is absent', () => {
    expect(parse(buildDestroyerEngineeringConsoleState(EMPTY)).engineering_auto).toBe(false);
  });

  it('passes shields blackboard state through the nested shields sub-object', () => {
    const state = {
      blackboards: {
        shields: {
          facings: [
            { arc_id: 'fore', label: 'Fore', hp: 80, max_hp: 80, online: true, center_deg: 0, width_deg: 90 },
            { arc_id: 'aft', label: 'Aft', hp: 40, max_hp: 80, online: true, center_deg: 180, width_deg: 90 },
          ],
          hull_integrity_pct: 75,
          focused_facing: null,
          target_bearing: null,
          grid_status: 'GRID NOMINAL',
        },
      },
    };
    const s = parse(buildDestroyerEngineeringConsoleState(state));
    expect(s.shields.grid_status).toBe('GRID NOMINAL');
    expect(s.shields.facings).toHaveLength(2);
    expect(s.shields.hull_integrity_pct).toBe(75);
  });

  it('passes power blackboard state through the nested power sub-object', () => {
    const state = {
      blackboards: {
        power: {
          groups: [{ id: 'helm', label: 'HELM', level: 3, max_level: 4 }],
          total: 3,
          total_max: 8,
          battery_charge: 75,
          battery_max: 100,
          locked: false,
        },
      },
    };
    const s = parse(buildDestroyerEngineeringConsoleState(state));
    expect(s.power.consoles).toHaveLength(1);
    expect(s.power.battery_charge).toBe(75);
  });

  it('passes repair blackboard state through the nested repair sub-object', () => {
    const state = {
      blackboards: {
        repair: {
          teams: ['Idle', 'Idle'],
          system_hull: [{ system_id: 'helm', current: 20, max_hp: 25 }],
          damageable_systems: ['helm'],
          travel_duration_secs: 5.0,
        },
      },
    };
    const s = parse(buildDestroyerEngineeringConsoleState(state));
    expect(s.repair.teams).toHaveLength(2);
    expect(s.repair.system_hull[0].system_id).toBe('helm');
  });

  it('shields sub-object has GRID OFFLINE when no facings', () => {
    const s = parse(buildDestroyerEngineeringConsoleState(EMPTY));
    expect(s.shields.grid_status).toBe('GRID OFFLINE');
  });

  it('shields sub-object has GRID NOMINAL when facings present', () => {
    const s = parse(buildDestroyerEngineeringConsoleState({ shieldFacings: ['fore', 'aft'] }));
    expect(s.shields.grid_status).toBe('GRID NOMINAL');
  });
});

// ── torpSlotStates (issue #637) ───────────────────────────────────────────────

describe('torpSlotStates', () => {
  it('returns vollMax slots', () => {
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 0, target_count: 0, load_progress: 0 });
    expect(slots).toHaveLength(4);
  });

  it('defaults to 1 slot when volley_max absent', () => {
    const slots = torpSlotStates({ loaded: false });
    expect(slots).toHaveLength(1);
    expect(slots[0].state).toBe('empty');
  });

  it('marks slots below loaded_count as filled', () => {
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 3, target_count: 3, load_progress: 0 });
    expect(slots[0].state).toBe('filled');
    expect(slots[1].state).toBe('filled');
    expect(slots[2].state).toBe('filled');
    expect(slots[3].state).toBe('empty');
  });

  it('marks slots queued-to-fill (loaded < i < target_count)', () => {
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 1, target_count: 3, load_progress: 0 });
    expect(slots[0].state).toBe('filled');
    expect(slots[1].state).toBe('queued-to-fill');
    expect(slots[2].state).toBe('queued-to-fill');
    expect(slots[3].state).toBe('empty');
  });

  it('marks slots queued-to-empty (target_count <= i < loaded_count)', () => {
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 3, target_count: 1, load_progress: 0 });
    expect(slots[0].state).toBe('filled');
    expect(slots[1].state).toBe('queued-to-empty');
    expect(slots[2].state).toBe('queued-to-empty');
    expect(slots[3].state).toBe('empty');
  });

  it('loading state: active slot is loaded_count index, progress bar filled to load_progress', () => {
    // loading: loaded_count=1, next slot (index 1) is being filled; load_progress=0.6
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 1, target_count: 3,
                                   state: 'loading', load_progress: 0.6 });
    expect(slots[1].progress).toBeCloseTo(0.6);
    // Other slots have no progress
    expect(slots[0].progress).toBe(0);
    expect(slots[2].progress).toBe(0);
    expect(slots[3].progress).toBe(0);
  });

  it('unloading state: active slot is loaded_count-1, progress bar = 1 - load_progress', () => {
    // unloading: loaded_count=3, top slot (index 2) is being drained; load_progress=0.7
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 3, target_count: 1,
                                   state: 'unloading', load_progress: 0.7 });
    expect(slots[2].progress).toBeCloseTo(0.3);
    expect(slots[0].progress).toBe(0);
    expect(slots[1].progress).toBe(0);
    expect(slots[3].progress).toBe(0);
  });

  it('no active slot when state is loaded (all loaded, no transition)', () => {
    const slots = torpSlotStates({ volley_max: 2, loaded_count: 2, target_count: 2,
                                   state: 'loaded', load_progress: 0 });
    expect(slots.every(s => s.progress === 0)).toBe(true);
  });

  it('Cruiser-style single slot: fully loaded', () => {
    const slots = torpSlotStates({ volley_max: 1, loaded_count: 1, target_count: 1,
                                   state: 'loaded', load_progress: 0 });
    expect(slots).toHaveLength(1);
    expect(slots[0].state).toBe('filled');
    expect(slots[0].progress).toBe(0);
  });

  it('Destroyer fore tube: volley_max=4, partially filled, partially queued', () => {
    // 2 loaded, targeting 4 (all will fill), none queued-to-empty
    const slots = torpSlotStates({ volley_max: 4, loaded_count: 2, target_count: 4, load_progress: 0 });
    expect(slots[0].state).toBe('filled');
    expect(slots[1].state).toBe('filled');
    expect(slots[2].state).toBe('queued-to-fill');
    expect(slots[3].state).toBe('queued-to-fill');
  });

  it('Battleship tube: volley_max=3, all queued-to-empty when target_count=0', () => {
    const slots = torpSlotStates({ volley_max: 3, loaded_count: 3, target_count: 0, load_progress: 0 });
    expect(slots.every(s => s.state === 'queued-to-empty')).toBe(true);
  });

  it('uses loaded boolean as fallback for loaded_count when field absent', () => {
    const slots = torpSlotStates({ volley_max: 1, loaded: true, target_count: 1 });
    expect(slots[0].state).toBe('filled');
  });
});

describe('repairCoreAndTargets', () => {
  const stationSystems = {
    helm: ['helm', 'helm-engine-port'],
    tactical: ['tactical', 'phaser-omni'],
    engineering: ['power-reactor', 'repair'],
  };

  it('classifies ownerless systems as core', () => {
    const hull = [
      { system_id: 'helm', display_name: 'Helm', current: 10, max_hp: 10 },
      { system_id: 'core', display_name: 'Core', current: 3, max_hp: 10 },
    ];
    const { coreSystems } = repairCoreAndTargets(hull, stationSystems);
    expect(coreSystems.map(s => s.system_id)).toEqual(['core']);
  });

  it('lists a dispatch target for every damageable station, with a damage fraction', () => {
    const hull = [
      { system_id: 'helm', display_name: 'Helm', current: 4, max_hp: 10 },        // damaged
      { system_id: 'helm-engine-port', display_name: 'Engine', current: 10, max_hp: 10 },
      { system_id: 'tactical', display_name: 'Tactical', current: 10, max_hp: 10 }, // healthy
      { system_id: 'phaser-omni', display_name: 'Phaser', current: 10, max_hp: 10 },
    ];
    const { targets } = repairCoreAndTargets(hull, stationSystems);
    const helm = targets.find(t => t.id === 'helm');
    expect(helm).toBeTruthy();
    expect(helm.label).toBe('Helm');
    expect(helm.damage_pct).toBeCloseTo(0.3, 5); // 6 of 20 hp lost

    // Healthy stations still get a dispatch target — repair teams can be
    // pre-positioned before damage occurs — just with damage_pct 0.
    const tactical = targets.find(t => t.id === 'tactical');
    expect(tactical).toBeTruthy();
    expect(tactical.damage_pct).toBe(0);
  });

  it('adds a core bucket when a core system is damaged', () => {
    const hull = [
      { system_id: 'core', display_name: 'Core', current: 5, max_hp: 10 },
    ];
    const { targets } = repairCoreAndTargets(hull, stationSystems);
    expect(targets.map(t => t.id)).toContain('core');
  });

  it('still lists every damageable station when nothing is damaged', () => {
    const hull = [
      { system_id: 'helm', display_name: 'Helm', current: 10, max_hp: 10 },
      { system_id: 'helm-engine-port', display_name: 'Engine', current: 10, max_hp: 10 },
      { system_id: 'tactical', display_name: 'Tactical', current: 10, max_hp: 10 },
      { system_id: 'phaser-omni', display_name: 'Phaser', current: 10, max_hp: 10 },
      { system_id: 'power-reactor', display_name: 'Reactor', current: 10, max_hp: 10 },
      { system_id: 'repair', display_name: 'Repair', current: 10, max_hp: 10 },
    ];
    const { targets } = repairCoreAndTargets(hull, stationSystems);
    expect(targets.map(t => t.id).sort()).toEqual(['engineering', 'helm', 'tactical']);
    expect(targets.every(t => t.damage_pct === 0)).toBe(true);
  });
});
