import { describe, it, expect } from 'vitest';
import {
  entityX, entityZ, entityRadius,
  buildBlips,
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

const NEAR = { uuid: 'a', x: 10, z: 0, radius: 2, tags: ['asteroid'] };
const FAR  = { uuid: 'b', x: 1000, z: 1000, radius: 2, tags: ['asteroid'] };
const SHIP = { uuid: 's', x: 5, z: 0, tags: ['ship'] };
const STATION = { uuid: 'st', x: 0, z: 5, tags: ['station'] };

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
    const blips = buildBlips([{ uuid: 'x', x: 1, z: 0, tags: ['region'] }], 0, 0, 0, 100);
    expect(blips[0].kind).toBe('asteroid');
  });

  it('supports entity_tags fallback', () => {
    const blips = buildBlips([{ uuid: 'q', x: 1, z: 0, entity_tags: ['ship'] }], 0, 0, 0, 100);
    expect(blips[0].kind).toBe('ship');
  });

  it('normalises scaled_radius = radius / range', () => {
    const blips = buildBlips([NEAR], 0, 0, 0, 100);
    expect(blips[0].scaled_radius).toBeCloseTo(2 / 100);
  });

  describe('rotate=true (ship-local frame, weapons/helm)', () => {
    it('at yaw=0 entity directly ahead (dz=-range) → radar_y ≈ 1', () => {
      const range = 100;
      const blips = buildBlips([{ uuid: 'f', x: 0, z: -range, radius: 1, tags: [] }], 0, 0, 0, range, { rotate: true });
      // dx=0, dz=-range → radar_x=(0·1+(-range)·0)/range=0, radar_y=(0·0-(-range)·1)/range=1
      expect(blips[0].radar_x).toBeCloseTo(0);
      expect(blips[0].radar_y).toBeCloseTo(1);
    });

    it('at yaw=0 entity to starboard (dx=+range) → radar_x ≈ 1', () => {
      const range = 100;
      const blips = buildBlips([{ uuid: 'r', x: range, z: 0, radius: 1, tags: [] }], 0, 0, 0, range, { rotate: true });
      expect(blips[0].radar_x).toBeCloseTo(1);
      expect(blips[0].radar_y).toBeCloseTo(0);
    });
  });

  describe('rotate=false (world-axis frame, sensors)', () => {
    it('radar_x = dx/range, radar_y = dz/range', () => {
      const range = 100;
      const blips = buildBlips([{ uuid: 'w', x: 30, z: 40, radius: 1, tags: [] }], 0, 0, 0, range, { rotate: false });
      expect(blips[0].radar_x).toBeCloseTo(30 / 100);
      expect(blips[0].radar_y).toBeCloseTo(40 / 100);
    });
  });

  it('merges extra fields from opts.extra', () => {
    const blips = buildBlips(
      [{ uuid: 'e', x: 1, z: 0, tags: [], name: 'Zeta', faction: 'pirate' }],
      0, 0, 0, 100,
      { extra: (a) => ({ name: a.name || null, faction: a.faction || null }) }
    );
    expect(blips[0].name).toBe('Zeta');
    expect(blips[0].faction).toBe('pirate');
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
      weaponsPhaserMode: 'Manual',
      weaponsTorpedoCount: 3,
    }));
    expect(s.target_uuid).toBe('tgt-1');
    expect(s.phaser_mode).toBe('Manual');
    expect(s.torpedo_count).toBe(3);
  });

  it('blips excludes entities outside WEAPONS_RADAR_RANGE', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      asteroids: [
        { uuid: 'close', x: 1, z: 0, tags: [] },
        { uuid: 'far', x: WEAPONS_RADAR_RANGE + 1, z: 0, tags: [] },
      ],
    };
    const s = parse(buildWeaponsConsoleState(state));
    expect(s.blips.map(b => b.uuid)).toEqual(['close']);
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

  it('blips include color, name, stance, faction extra fields', () => {
    const state = {
      shipX: 0, shipZ: 0, shipYaw: 0,
      asteroids: [{ uuid: 'p', x: 10, z: 0, tags: ['ship'], name: 'Raider', stance: 'hostile', faction: 'pirate' }],
    };
    const blips = parse(buildSensorsConsoleState(state)).blips;
    expect(blips[0].name).toBe('Raider');
    expect(blips[0].stance).toBe('hostile');
    expect(blips[0].faction).toBe('pirate');
    expect(blips[0].color).toBeNull();
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
        hull_pct: 75, heading: 45, speed: 10,
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
      asteroids: [{ uuid: 'st1', x: 100, z: 0, tags: ['station'], name: 'Starbase 1' }],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(1);
    expect(blips[0].kind).toBe('station');
    expect(blips[0].name).toBe('Starbase 1');
  });

  it('includes planet and star entities', () => {
    const state = {
      shipX: 0, shipZ: 0,
      asteroids: [
        { uuid: 'p1', x: 50, z: 0,  tags: ['planet'] },
        { uuid: 's1', x: 0,  z: 50, tags: ['star']   },
      ],
    };
    const blips = parse(buildNavigationConsoleState(state)).blips;
    expect(blips.length).toBe(2);
    expect(blips.map(b => b.kind).sort()).toEqual(['planet', 'star']);
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
      asteroids: [{ uuid: 'ps1', x: 5, z: 0, tags: ['player_ship'] }],
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

  it('blips use world-axis (north-up) projection — no ship yaw rotation', () => {
    // Entity directly east (x+) of ship → radar_x positive, radar_y ≈ 0
    const state = {
      shipX: 0, shipZ: 0,
      asteroids: [{ uuid: 'st', x: 500, z: 0, tags: ['station'] }],
    };
    const blip = parse(buildNavigationConsoleState(state)).blips[0];
    expect(blip.radar_x).toBeCloseTo(500 / NAVIGATION_RADAR_RANGE);
    expect(blip.radar_y).toBeCloseTo(0);
  });
});
