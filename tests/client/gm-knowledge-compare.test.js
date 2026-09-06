// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  buildKnowledgeCompare,
  commsContactRows,
  commsMessageRows,
  createGmKnowledgeCompare,
  crewContactRows,
  diffByIdentity,
  objectiveRows,
  truthContactRows,
} from '../../gui/gm-knowledge-compare.js';
import { buildGmStationConsoleInput } from '../../gui/gm-station-puppet.js';
// The vitest setup file (tests/client/setup-strings.js) already loads the
// real assets/strings/strings.csv table into this module before any test
// runs, so importing `t` here calls the exact production resolver — no
// buildTable/setTable boilerplate needed to catch a real double-resolve
// (issue #1318 review, finding 2).
import { t as realT } from '../../gui/strings.js';

// ── Fixtures ─────────────────────────────────────────────────────────────

function truthEntity(overrides = {}) {
  return {
    entity_id: 'ship-alpha',
    name: 'Alpha',
    kind: 'npc_ship',
    position: [10, 0, 0],
    faction: null,
    status: { hull_percent: 80, condition_percent: null, destroyed: false },
    current_target: null,
    geometry: null,
    radar: { icon: 'ship', colour: null, size: null, region_colour: null },
    ...overrides,
  };
}

/** A minimal `GmPuppetShipProjection`-shaped fixture (see gm-station-puppet.test.js
 * for the prior-art shape this mirrors). `sensors_radar_range` bounds which
 * `entities` show up as Sensors contacts; entities carry `position`/
 * `hull_fraction` directly so no separate `entity_states` fold is needed. */
function shipProjection({
  entities = [],
  objectives = [],
  messages = [],
  contacts = [],
  sensorsRadarRange = 100,
} = {}) {
  return {
    ship_id: 'ship-player-1',
    name: 'Resolute',
    stations: [{
      station_id: 'sensors',
      name: 'Sensors',
      console: 'gui/sensors-console.html',
      rating: 'Backfill',
      operators: [],
    }],
    ship_config: {
      station_systems: { sensors: ['sensor-main'], comms: ['comms-main'] },
      system_console_families: { 'sensor-main': 'sensors', 'comms-main': 'comms' },
      system_kinds: { 'sensor-main': 'sensors', 'comms-main': 'comms' },
      blackboard_console_families: {},
      sensors_radar_range: sensorsRadarRange,
      sensors_radar_shows: [],
      sensors_radar_selects: [],
      station_tutorials: {},
      station_assist_gaps: {},
    },
    station_ratings: { sensors: 'Backfill' },
    control_sources: {},
    blackboards: [['comms-main', {
      kind: 'Comms',
      data: { messages, objectives: [], contacts, host_station: null },
    }]],
    entities,
    entity_states: [],
    objectives,
    ship_pose: { x: 0, y: 0, z: 0, yaw: 0, forward_speed: 0 },
    navigation_waypoint: null,
    console_hull: [{
      system_id: 'sensor-main', display_name: 'Sensors', current: 40, max_hp: 40,
      tier: 'Nominal', debuff_magnitude: 0,
    }],
  };
}

function entitySnapshot(overrides = {}) {
  return {
    uuid: 'ship-alpha',
    name: 'Alpha',
    position: [10, 0, 0],
    tags: ['ship'],
    radar_icon: 'ship',
    hull_fraction: 0.8,
    ...overrides,
  };
}

// ── diffByIdentity ───────────────────────────────────────────────────────

describe('diffByIdentity', () => {
  it('reports empty for two empty lists', () => {
    const diff = diffByIdentity([], []);
    expect(diff).toEqual({ rows: [], empty: true, equal: false, different: false });
  });

  it('classifies same, changed, truth_only and crew_only in sorted identity order', () => {
    const truth = [
      { id: 'b', name: 'Beta', hull_percent: 50 },
      { id: 'a', name: 'Alpha', hull_percent: 80 },
      { id: 'c', name: 'Charlie', hull_percent: 10 },
    ];
    const crew = [
      { id: 'a', name: 'Alpha', hull_percent: 80 },
      { id: 'b', name: 'Beta', hull_percent: 40 },
      { id: 'd', name: 'Delta', hull_percent: 5 },
    ];
    const diff = diffByIdentity(truth, crew, { fields: ['name', 'hull_percent'] });
    expect(diff.rows.map((row) => row.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(diff.rows.map((row) => row.status)).toEqual(['same', 'changed', 'truth_only', 'crew_only']);
    expect(diff.equal).toBe(false);
    expect(diff.different).toBe(true);
    expect(diff.empty).toBe(false);
  });

  it('reports equal when every identity matches on every compared field', () => {
    const rows = [{ id: 'a', name: 'Alpha' }];
    const diff = diffByIdentity(rows, rows, { fields: ['name'] });
    expect(diff.equal).toBe(true);
    expect(diff.different).toBe(false);
  });

  it('ignores fields not named in options — only declared fields can produce "changed"', () => {
    const truth = [{ id: 'a', name: 'Alpha', extra: 1 }];
    const crew = [{ id: 'a', name: 'Alpha', extra: 2 }];
    expect(diffByIdentity(truth, crew, { fields: ['name'] }).equal).toBe(true);
    expect(diffByIdentity(truth, crew, { fields: ['extra'] }).equal).toBe(false);
  });
});

// ── Category row builders ────────────────────────────────────────────────

describe('truthContactRows', () => {
  it('keeps only point entities (geometry === null) and maps the broad-status fields', () => {
    const region = truthEntity({ entity_id: 'field-1', geometry: { type: 'sphere', radius: 40 } });
    const ship = truthEntity();
    expect(truthContactRows([ship, region])).toEqual([
      { id: 'ship-alpha', name: 'Alpha', hull_percent: 80, destroyed: false },
    ]);
  });

  it('resolves a raw String Table id through wireText, never leaking it into the Truth column ' +
    '(issue #1318 review, finding 1)', () => {
    // `gm_entity` is exempt from host-channel localisation, so an authored
    // entity name arrives as the literal id Rust sent — a real one, so this
    // exercises the actual production string table the vitest setup file
    // loads, not a mock.
    const starbase = truthEntity({
      entity_id: 'starbase-alpha',
      name: 'world.entity.starbase_alpha.name',
    });
    expect(truthContactRows([starbase])).toEqual([
      { id: 'starbase-alpha', name: 'Starbase Alpha', hull_percent: 80, destroyed: false },
    ]);
  });

  it('lets literal text (not a table id) pass through untouched', () => {
    const ship = truthEntity({ name: 'AEV Vanguard' });
    expect(truthContactRows([ship])[0].name).toBe('AEV Vanguard');
  });

  it('accepts an injectable displayText for tests/callers that need a stand-in', () => {
    const ship = truthEntity({ name: 'raw.id' });
    const stub = (value) => `resolved(${value})`;
    expect(truthContactRows([ship], stub)[0].name).toBe('resolved(raw.id)');
  });
});

describe('crewContactRows', () => {
  it('excludes the synthetic tactical-target and navigation-waypoint overlay blips', () => {
    const blips = [
      { uuid: 'ship-alpha', name: 'Alpha', kind: 'ship' },
      { uuid: 'ship-alpha', name: 'Alpha', kind: 'tactical-target' },
      { uuid: 'navigation-waypoint', name: null, kind: 'waypoint' },
    ];
    const rows = crewContactRows(blips, [entitySnapshot()]);
    expect(rows).toEqual([{ id: 'ship-alpha', name: 'Alpha', hull_percent: 80, destroyed: false }]);
  });

  it('reads hull_fraction off the raw entity replica, never inventing a second hull model', () => {
    const blips = [{ uuid: 'ship-alpha', name: 'Alpha', kind: 'ship' }];
    expect(crewContactRows(blips, [entitySnapshot({ hull_fraction: 0 })])[0]).toMatchObject({
      hull_percent: 0, destroyed: true,
    });
    // No `hull_fraction` on the replica means "no hull model here", which is
    // "not destroyed" — the same semantics as Truth's `broad_status`, never
    // `null` (issue #1318 review, finding 5).
    expect(crewContactRows(blips, [entitySnapshot({ hull_fraction: undefined })])[0]).toMatchObject({
      hull_percent: null, destroyed: false,
    });
  });

  it('excludes a planet/star/moon blip Truth\'s world_kind never classifies at all ' +
    '(issue #1318 review, finding 3)', () => {
    // Mirrors combat_test: a destroyer's sensors `shows` list includes
    // "planet", and Earth carries a radar icon and sits inside scan range,
    // but src/gm_projection.rs's `world_kind` returns `None` for planets —
    // Truth never models them, so they must never render as `crew_only`
    // ("Truth already dropped this identity").
    const planet = entitySnapshot({
      uuid: 'earth', name: 'Earth', tags: ['planet'], radar_icon: 'planet', hull_fraction: undefined,
    });
    const ship = entitySnapshot({ uuid: 'ship-alpha' });
    const blips = [
      { uuid: 'ship-alpha', name: 'Alpha', kind: 'ship' },
      { uuid: 'earth', name: 'Earth', kind: 'planet' },
    ];
    expect(crewContactRows(blips, [ship, planet])).toEqual([
      { id: 'ship-alpha', name: 'Alpha', hull_percent: 80, destroyed: false },
    ]);
  });

  it('keeps a structure/station contact and an authored (named) asteroid', () => {
    const outpost = entitySnapshot({
      uuid: 'outpost', name: 'Outpost', tags: ['structure'], radar_icon: 'structure', hull_fraction: undefined,
    });
    const namedAsteroid = entitySnapshot({
      uuid: 'rock-1', name: 'Erebus Rock', tags: ['asteroid'], radar_icon: 'asteroid', hull_fraction: undefined,
    });
    const bareAsteroid = entitySnapshot({
      uuid: 'rock-2', name: null, tags: ['asteroid'], radar_icon: 'asteroid', hull_fraction: undefined,
    });
    const blips = [
      { uuid: 'outpost', name: 'Outpost', kind: 'structure' },
      { uuid: 'rock-1', name: 'Erebus Rock', kind: 'asteroid' },
      { uuid: 'rock-2', name: null, kind: 'asteroid' },
    ];
    expect(crewContactRows(blips, [outpost, namedAsteroid, bareAsteroid]).map((row) => row.id))
      .toEqual(['outpost', 'rock-1']);
  });

  it('drops a blip whose backing raw entity is entirely absent — no tags to classify it by', () => {
    const blips = [{ uuid: 'ghost', name: 'Ghost', kind: 'ship' }];
    expect(crewContactRows(blips, [])).toEqual([]);
  });
});

describe('objectiveRows / commsMessageRows / commsContactRows', () => {
  it('maps the wire shape and drops malformed entries with no stable identity', () => {
    expect(objectiveRows([
      { id: 'obj-1', text: 'objective.text', mandatory: true, status: 'Active' },
      { text: 'no id' },
      null,
    ])).toEqual([
      { id: 'obj-1', text: 'objective.text', text_params: {}, mandatory: true, status: 'Active' },
    ]);
    expect(commsMessageRows([{ id: 'msg-1', sender_name: 'Ops', subject: 'Hail', is_read: false }]))
      .toEqual([{ id: 'msg-1', sender_name: 'Ops', subject: 'Hail', is_read: false }]);
    expect(commsContactRows([{ uuid: 'c-1', name: 'Contact', in_range: false }]))
      .toEqual([{ id: 'c-1', name: 'Contact', in_range: false }]);
  });
});

// ── buildKnowledgeCompare ────────────────────────────────────────────────

describe('buildKnowledgeCompare', () => {
  it('returns fully empty diffs when no ship is selected', () => {
    const compare = buildKnowledgeCompare([], { ships: [], activity: [] }, null);
    expect(compare.contacts.empty).toBe(true);
    expect(compare.objectives.empty).toBe(true);
    expect(compare.comms.messages.empty).toBe(true);
    expect(compare.comms.contacts.empty).toBe(true);
  });

  it('reports Objectives as tautologically equal for any non-empty list — `ClientSimState.objectives` ' +
    'is the SAME array `gui/sim-state.js` stores from the ship projection\'s own field, not merely ' +
    'equal by value — and Comms as equal by construction (issue #1318 review, finding 3)', () => {
    const objectives = [{ id: 'obj-1', text: 'objective.text', mandatory: true, status: 'Active' }];
    const messages = [{ id: 'msg-1', sender_name: 'Ops', subject: 'objective.hail', is_read: false }];
    const contacts = [{ uuid: 'c-1', name: 'Dockmaster', in_range: true }];
    const ship = shipProjection({ objectives, messages, contacts });
    // `buildGmStationConsoleInput` -> `ObjectiveSummary` folds `ship.objectives`
    // into `state.objectives` via `this.objectives = d.objectives || []`
    // (gui/sim-state.js) with no copy — pin the reference identity itself,
    // not just the resulting diff, so a future copy/filter is caught even if
    // it happened to preserve value-equality.
    const state = buildGmStationConsoleInput({ activity: [] }, ship);
    expect(state.objectives).toBe(ship.objectives);
    const compare = buildKnowledgeCompare([], { ships: [ship], activity: [] }, ship);
    expect(compare.objectives).toMatchObject({ equal: true, empty: false, different: false });
    expect(compare.comms.messages).toMatchObject({ equal: true, empty: false });
    expect(compare.comms.contacts).toMatchObject({ equal: true, empty: false });
  });

  it('resolves the same Comms system on both arms for a ship with two Comms consoles, because ' +
    'the producer already sorts `blackboards` by system id before it reaches the wire ' +
    '(issue #1318 review, finding 2)', () => {
    const msgA = { id: 'msg-a', sender_name: 'Ops', subject: 'Comms A', is_read: false };
    const msgB = { id: 'msg-b', sender_name: 'Ops', subject: 'Comms B', is_read: false };
    const ship = shipProjection({});
    // `src/gm_projection.rs` sorts this vector by system id
    // (`blackboards.sort_by(|left, right| left.0.cmp(&right.0))`) before it
    // is ever serialised — no real payload can arrive with 'comms-b' before
    // 'comms-a'. The Truth arm here reads that sorted-first entry directly;
    // the Crew Knowledge arm's `blackboardsOfKind` (gui/console-state.js)
    // falls back to the SAME lexically-first Comms system id when no Station
    // preference is supplied. Both arms therefore resolve 'comms-a' today —
    // a structural equality, not a coincidence of this fixture.
    ship.blackboards = [
      ['comms-a', { kind: 'Comms', data: { messages: [msgA], contacts: [] } }],
      ['comms-b', { kind: 'Comms', data: { messages: [msgB], contacts: [] } }],
    ];
    const compare = buildKnowledgeCompare([], { ships: [ship], activity: [] }, ship);
    expect(compare.comms.messages).toMatchObject({ equal: true, different: false });
    expect(compare.comms.messages.rows.map((row) => row.id)).toEqual(['msg-a']);
  });

  it('finds a Truth contact absent from Crew Knowledge when it is outside this ship\'s sensor range', () => {
    const inRange = entitySnapshot({ uuid: 'near', name: 'Near', position: [10, 0, 0], hull_fraction: 1 });
    const outOfRange = entitySnapshot({ uuid: 'far', name: 'Far', position: [5000, 0, 0], hull_fraction: 1 });
    const ship = shipProjection({ entities: [inRange, outOfRange], sensorsRadarRange: 100 });
    const truth = [
      truthEntity({ entity_id: 'near', name: 'Near', status: { hull_percent: 100, condition_percent: null, destroyed: false } }),
      truthEntity({ entity_id: 'far', name: 'Far', status: { hull_percent: 100, condition_percent: null, destroyed: false } }),
    ];
    const compare = buildKnowledgeCompare(truth, { ships: [ship], activity: [] }, ship);
    const byId = Object.fromEntries(compare.contacts.rows.map((row) => [row.id, row.status]));
    expect(byId.near).toBe('same');
    expect(byId.far).toBe('truth_only');
    expect(compare.contacts.different).toBe(true);
  });

  it('matches a published-infrastructure contact tagged only "civilian" (no structure/station tag) ' +
    'as `same`, never `truth_only` — mirrors the shipped `stranded_lighter`/`skyway_castaway_lifeboat` ' +
    'templates, whose `tags = ["civilian", "infrastructure"]` sit exactly in the gap `world_kind`\'s ' +
    '`infrastructure.is_some()` arm covers on the Truth side but a tag-only crew-side check would miss ' +
    '(issue #1318 review, finding 1)', () => {
    const lighter = entitySnapshot({
      uuid: 'lighter', name: 'Stranded Lighter', tags: ['civilian', 'infrastructure'],
      radar_icon: 'station', hull_fraction: undefined,
      infrastructure: { condition_fraction: 0.55, flags: [], capacities: [] },
    });
    const ship = shipProjection({ entities: [lighter] });
    const truth = [truthEntity({
      entity_id: 'lighter', name: 'Stranded Lighter',
      status: { hull_percent: null, condition_percent: 55, destroyed: false },
    })];
    const compare = buildKnowledgeCompare(truth, { ships: [ship], activity: [] }, ship);
    expect(compare.contacts.rows).toEqual([
      expect.objectContaining({ id: 'lighter', status: 'same' }),
    ]);
  });

  it('never reports a spurious hull_percent difference from widening an f32 fraction to f64 before ' +
    'multiplying (issue #1318 review, finding 4)', () => {
    // 0.004999999888241291 is an f32 value one ULP below the exact fraction
    // that multiplies to 0.5. In plain f64 arithmetic `hullFraction * 100`
    // stays fractionally below 0.5 and rounds DOWN to 0. Rust's
    // `percent()` (src/gm_projection.rs) computes the multiply itself in
    // f32: re-rounding the same exact f64 product to the nearest f32 lands
    // exactly on 0.5, and f32 `.round()` rounds halves away from zero, i.e.
    // UP to 1 — `Math.fround` reproduces that f32 rounding before
    // `Math.round` is applied. Without mirroring that, this identity's hull
    // is bit-for-bit identical on both arms yet renders as `changed`.
    const hullFraction = 0.004999999888241291;
    expect(Math.round(hullFraction * 100)).toBe(0); // naive f64 multiply (the bug)
    expect(Math.round(Math.fround(hullFraction * 100))).toBe(1); // Rust's f32 arithmetic
    const entity = entitySnapshot({ uuid: 'boundary', name: 'Boundary', hull_fraction: hullFraction });
    const ship = shipProjection({ entities: [entity] });
    const truth = [truthEntity({
      entity_id: 'boundary', name: 'Boundary',
      status: { hull_percent: 1, condition_percent: null, destroyed: false },
    })];
    const compare = buildKnowledgeCompare(truth, { ships: [ship], activity: [] }, ship);
    expect(compare.contacts.rows).toEqual([
      expect.objectContaining({ id: 'boundary', status: 'same' }),
    ]);
  });

  it('flags a changed broad-state field when hull differs between Truth and the folded replica', () => {
    const entity = entitySnapshot({ uuid: 'wounded', name: 'Wounded', hull_fraction: 0.9 });
    const ship = shipProjection({ entities: [entity] });
    const truth = [truthEntity({
      entity_id: 'wounded', name: 'Wounded',
      status: { hull_percent: 60, condition_percent: null, destroyed: false },
    })];
    const compare = buildKnowledgeCompare(truth, { ships: [ship], activity: [] }, ship);
    expect(compare.contacts.rows).toEqual([
      expect.objectContaining({ id: 'wounded', status: 'changed' }),
    ]);
  });

  it('never flags "destroyed" as changed for an identity with no hull model on either side ' +
    '(issue #1318 review, finding 5)', () => {
    const entity = entitySnapshot({ uuid: 'hulless', name: 'Hulless', hull_fraction: undefined });
    const ship = shipProjection({ entities: [entity] });
    const truth = [truthEntity({
      entity_id: 'hulless', name: 'Hulless',
      status: { hull_percent: null, condition_percent: 50, destroyed: false },
    })];
    const compare = buildKnowledgeCompare(truth, { ships: [ship], activity: [] }, ship);
    expect(compare.contacts.rows).toEqual([
      expect.objectContaining({ id: 'hulless', status: 'same' }),
    ]);
  });

  it('never flags hull_percent as changed when only one side models a hull reading at all ' +
    '(issue #1318 review, finding 5)', () => {
    // An authored asteroid: the WorldResource snapshot carries `hull_fraction`
    // for it, but Truth's `broad_status` has no `EntitySystemHull` counterpart
    // to read at all, so it reports `hull_percent: null` — a data-shape gap,
    // not a knowledge difference.
    const entity = entitySnapshot({
      uuid: 'rock-a', name: 'Rock A', tags: ['asteroid'], radar_icon: 'asteroid', hull_fraction: 0.7,
    });
    const ship = shipProjection({ entities: [entity] });
    const truth = [truthEntity({
      entity_id: 'rock-a', name: 'Rock A',
      status: { hull_percent: null, condition_percent: null, destroyed: false },
    })];
    const compare = buildKnowledgeCompare(truth, { ships: [ship], activity: [] }, ship);
    expect(compare.contacts.rows).toEqual([
      expect.objectContaining({ id: 'rock-a', status: 'same' }),
    ]);
  });

  it('flags a removed identity: a stale Crew Knowledge contact Truth has already dropped', () => {
    const entity = entitySnapshot({ uuid: 'stale', name: 'Stale' });
    const ship = shipProjection({ entities: [entity] });
    // Truth no longer lists 'stale' at all — the world already removed it.
    const compare = buildKnowledgeCompare([], { ships: [ship], activity: [] }, ship);
    expect(compare.contacts.rows).toEqual([
      expect.objectContaining({ id: 'stale', status: 'crew_only' }),
    ]);
  });

  it('never surfaces Station-private detail (console_hull, non-Sensors/Comms blackboards)', () => {
    const ship = shipProjection({});
    ship.console_hull = [{
      system_id: 'shield-generator', display_name: 'Shields', current: 1, max_hp: 100,
      tier: 'Critical', debuff_magnitude: 0.9,
    }];
    ship.blackboards.push(['weapons-main', {
      kind: 'Weapons', data: { phaser_charge: 1, torpedoes: 4 },
    }]);
    const compare = buildKnowledgeCompare([], { ships: [ship], activity: [] }, ship);
    const serialised = JSON.stringify(compare);
    expect(serialised).not.toMatch(/console_hull|shield-generator|phaser_charge|torpedoes/);
  });
});

// ── createGmKnowledgeCompare (DOM controller) ───────────────────────────

function mount() {
  document.body.innerHTML = `
    <p id="gm-knowledge-pending"></p>
    <section id="gm-knowledge-panel" hidden>
      <select id="gm-knowledge-select"></select>
      ${['contacts', 'objectives', 'comms-messages', 'comms-contacts'].map((key) => `
        <p id="gm-knowledge-${key}-summary"></p>
        <p id="gm-knowledge-${key}-empty"></p>
        <table id="gm-knowledge-${key}-table" hidden><tbody id="gm-knowledge-${key}-rows"></tbody></table>
      `).join('')}
    </section>`;
}

describe('createGmKnowledgeCompare', () => {
  beforeEach(mount);

  it('shows the pending message and hides the panel until a ship exists', () => {
    const controller = createGmKnowledgeCompare({ doc: document, t: (id) => id });
    controller.updateTruth([]);
    controller.updateStations({ ships: [], activity: [] });
    expect(document.getElementById('gm-knowledge-pending').hidden).toBe(false);
    expect(document.getElementById('gm-knowledge-panel').hidden).toBe(true);
    expect(controller.state().selectedShipId).toBeNull();
  });

  it('populates the panel, and selecting another ship recomputes every category (AC #2)', () => {
    const controller = createGmKnowledgeCompare({ doc: document, t: (id) => id });
    const shipA = shipProjection({
      entities: [entitySnapshot({ uuid: 'a-contact', name: 'A Contact' })],
    });
    shipA.ship_id = 'ship-a';
    shipA.name = 'Ship A';
    const shipB = shipProjection({
      entities: [entitySnapshot({ uuid: 'b-contact', name: 'B Contact' })],
    });
    shipB.ship_id = 'ship-b';
    shipB.name = 'Ship B';
    const truth = [
      truthEntity({ entity_id: 'a-contact', name: 'A Contact' }),
      truthEntity({ entity_id: 'b-contact', name: 'B Contact' }),
    ];

    controller.updateTruth(truth);
    controller.updateStations({ ships: [shipA, shipB], activity: [] });

    expect(document.getElementById('gm-knowledge-pending').hidden).toBe(true);
    expect(document.getElementById('gm-knowledge-panel').hidden).toBe(false);
    expect(controller.state().selectedShipId).toBe('ship-a');
    // Truth lists both contacts regardless of selection (it is ship-independent);
    // selecting ship A shows its own contact as "same" and the other ship's
    // contact as "truth_only" (ship A's own Sensors do not know about it).
    const statusFor = (id) => [...document.getElementById('gm-knowledge-contacts-rows').children]
      .find((row) => row.dataset.identity === id)?.dataset.status;
    expect(document.getElementById('gm-knowledge-contacts-rows').children).toHaveLength(2);
    expect(statusFor('a-contact')).toBe('same');
    expect(statusFor('b-contact')).toBe('truth_only');

    expect(controller.select('ship-b')).toBe(true);
    expect(controller.state().selectedShipId).toBe('ship-b');
    expect(statusFor('a-contact')).toBe('truth_only');
    expect(statusFor('b-contact')).toBe('same');
  });

  it('falls back to another ship when the selected one is removed, and resets to pending when none remain (reconnect)', () => {
    const controller = createGmKnowledgeCompare({ doc: document, t: (id) => id });
    const shipA = shipProjection({});
    shipA.ship_id = 'ship-a';
    // ship-b carries a real contact so the tbody has rows to lose — the
    // original assertion below could not fail with an always-empty ship
    // (issue #1318 review, finding 7).
    const shipB = shipProjection({
      entities: [entitySnapshot({ uuid: 'ship-b-contact', name: 'B Contact' })],
    });
    shipB.ship_id = 'ship-b';
    controller.updateTruth([truthEntity({ entity_id: 'ship-b-contact', name: 'B Contact' })]);
    controller.updateStations({ ships: [shipA, shipB], activity: [] });
    controller.select('ship-b');
    expect(controller.state().selectedShipId).toBe('ship-b');
    expect(document.getElementById('gm-knowledge-contacts-rows').children.length).toBeGreaterThan(0);

    // ship-b disconnects/despawns — only ship-a remains.
    controller.updateStations({ ships: [shipA], activity: [] });
    expect(controller.state().selectedShipId).toBe('ship-a');

    // Every ship leaves — the panel returns to its pending state exactly as
    // it started, with no stale rows left behind.
    controller.updateStations({ ships: [], activity: [] });
    expect(document.getElementById('gm-knowledge-pending').hidden).toBe(false);
    expect(document.getElementById('gm-knowledge-panel').hidden).toBe(true);
    expect(document.getElementById('gm-knowledge-contacts-rows').children).toHaveLength(0);

    // A reconnect repopulates cleanly from scratch.
    controller.updateStations({ ships: [shipA], activity: [] });
    expect(document.getElementById('gm-knowledge-pending').hidden).toBe(true);
    expect(controller.state().selectedShipId).toBe('ship-a');
  });

  it('never double-resolves already-localised objective text through the real string table ' +
    '(issue #1318 review, finding 2)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const controller = createGmKnowledgeCompare({ doc: document, t: realT });
      // `entry.text` here stands in for what a real `objective.text` id
      // resolves to by the time it reaches this module: `gm_station`
      // already localised it (see the module doc comment), so it is plain
      // English, not a table id.
      const objectives = [{ id: 'obj-1', text: 'Destroy the enemy cruiser', mandatory: true, status: 'Active' }];
      const ship = shipProjection({ objectives });
      controller.updateTruth([]);
      controller.updateStations({ ships: [ship], activity: [] });

      const row = document.getElementById('gm-knowledge-objectives-rows').children[0];
      expect(row).toBeTruthy();
      const truthCell = row.children[1].textContent;
      const crewCell = row.children[2].textContent;
      expect(truthCell).not.toContain('⟨');
      expect(crewCell).not.toContain('⟨');
      expect(truthCell).toContain('Destroy the enemy cruiser');
      expect(warn).not.toHaveBeenCalled();
    } finally {
      warn.mockRestore();
    }
  });

  it('never leaks a raw Truth String Table id into the rendered contacts table ' +
    '(issue #1318 review, finding 1)', () => {
    const controller = createGmKnowledgeCompare({ doc: document, t: realT });
    const ship = shipProjection({
      entities: [entitySnapshot({ uuid: 'starbase-alpha', name: 'Starbase Alpha' })],
    });
    const truth = [truthEntity({
      entity_id: 'starbase-alpha', name: 'world.entity.starbase_alpha.name',
    })];
    controller.updateTruth(truth);
    controller.updateStations({ ships: [ship], activity: [] });

    const row = document.getElementById('gm-knowledge-contacts-rows').children[0];
    expect(row.dataset.status).toBe('same');
    expect(row.children[1].textContent).toContain('Starbase Alpha');
    expect(row.children[1].textContent).not.toMatch(/^world\./);
  });

  it('clear() resets the panel to its empty pending state', () => {
    const controller = createGmKnowledgeCompare({ doc: document, t: (id) => id });
    const ship = shipProjection({});
    controller.updateStations({ ships: [ship], activity: [] });
    expect(document.getElementById('gm-knowledge-panel').hidden).toBe(false);

    controller.clear();
    expect(document.getElementById('gm-knowledge-pending').hidden).toBe(false);
    expect(document.getElementById('gm-knowledge-panel').hidden).toBe(true);
    expect(controller.state()).toEqual({ selectedShipId: null, shipIds: [] });
  });
});
