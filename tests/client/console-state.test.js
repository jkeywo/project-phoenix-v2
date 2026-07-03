import { describe, it, expect } from 'vitest';
import {
  entityX, entityZ, entityRadius,
  buildBlips,
  buildRadarRegions,
  buildWaypointBlip,
  buildTargetBlip,
  WEAPONS_RADAR_RANGE, HELM_RADAR_RANGE, SENSORS_RADAR_RANGE,
  NAVIGATION_RADAR_RANGE,
  buildWeaponsConsoleState,
  buildCaptainConsoleState,
  buildHelmConsoleState,
  buildRepairConsoleState,
  buildPowerConsoleState,
  buildShieldsConsoleState,
  buildSensorsConsoleState,
  buildCommsConsoleState,
  buildNavigationConsoleState,
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

  it('clears view_direction for non-camera views', () => {
    expect(parse(buildCaptainConsoleState({ currentView: 'Radar' })).view_direction).toBe('');
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

  it('prefers blackboard groups (post issue #618)', () => {
    const s = parse(buildPowerConsoleState({
      blackboards: {
        power: {
          groups: [
            { id: 'helm',    label: 'HELM',    level: 3, max_level: 4 },
            { id: 'weapons', label: 'WEAPONS', level: 1, max_level: 4 },
          ],
          consoles: [],
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

  it('falls back to legacy blackboard consoles when groups is empty', () => {
    const s = parse(buildPowerConsoleState({
      blackboards: {
        power: {
          groups: [],
          consoles: [
            { id: 'helm', label: 'HELM', level: 2, max_level: 4 },
          ],
          total: 2, total_max: 8, battery_charge: 0, battery_max: 100, locked: false,
        },
      },
    }));
    expect(s.consoles).toEqual([
      { id: 'helm', label: 'HELM', level: 2, max_level: 4 },
    ]);
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

  it('includes player_ship entities', () => {
    const state = {
      shipX: 0, shipZ: 0,
      navChartShows: ['player_ship'],
      asteroids: [{ uuid: 'ps1', x: 5, z: 0, tags: ['player_ship'], radar_icon: 'ship' }],
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

  // tactical_auto
  it('tactical_auto is true when stationRatings.tactical === Backfill', () => {
    expect(parse(buildWeaponsConsoleState({ stationRatings: { tactical: 'Backfill' } })).tactical_auto).toBe(true);
  });

  it('tactical_auto is false when stationRatings.tactical is a different rating', () => {
    expect(parse(buildWeaponsConsoleState({ stationRatings: { tactical: 'Full' } })).tactical_auto).toBe(false);
  });

  it('tactical_auto is false when stationRatings is absent', () => {
    expect(parse(buildWeaponsConsoleState(EMPTY)).tactical_auto).toBe(false);
  });

  // repair_auto
  it('repair_auto is true when stationRatings.repair === Backfill', () => {
    expect(parse(buildRepairConsoleState({ stationRatings: { repair: 'Backfill' } })).repair_auto).toBe(true);
  });

  it('repair_auto is false when stationRatings.repair is a different rating', () => {
    expect(parse(buildRepairConsoleState({ stationRatings: { repair: 'Full' } })).repair_auto).toBe(false);
  });

  it('repair_auto is false when stationRatings is absent', () => {
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
