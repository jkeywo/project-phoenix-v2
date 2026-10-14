import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectInterior, initialInterior, initialAccessGateTree, validateInterior, prepareInteriorOperation, applyInteriorOperation } from '../workshop-interiors.js';
import { resolveTemplate } from '../entity-includes.js';
import { interiorProfile, interiorSource } from './interior-fixture.js';
import { stringify } from 'smol-toml';
import { prepareShipOperation } from '../workshop-ship-authoring.js';
import { prepareShieldOperation, inspectShields, defaultShieldArc } from '../workshop-shields.js';
import { shieldSource } from './shield-fixture.js';
const path = 'assets/entities/ship.toml', base = 'assets/entities/base.toml', dependencies = { base_files: {}, packs: [] };
const draft = (source = interiorSource()) => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
const prepare = (current, interior, extra = {}, deps = dependencies) => prepareInteriorOperation(current, deps, { path, interior, ...extra }).changes[0].after;
describe('explicit physical interior authoring', () => {
  it('rejects the retired legacy placement adapter', () => {
    const profile = interiorProfile(); profile.repair_placements = { core: 'engineering' };
    expect(() => prepare(draft(), profile)).toThrow(/retired repair placements/);
  });
  it('includes generated equipment and leaves control/knowledge declarations unchanged', () => {
    const current = draft(), read = inspectInterior(current, dependencies, path);
    expect(read.systems.map(system => system.id)).toEqual(['helm', 'shield-arc-fore']);
    const interior = read.interior; interior.compartment[0].x = -22; interior.compartment[0].material = 'unusual-alloy';
    const after = prepare(current, interior);
    expect(parse(after).interior.compartment[0]).toMatchObject({ x: -22, material: 'unusual-alloy' });
    expect(after.slice(0, after.indexOf('[interior]'))).toBe(current.read(path).slice(0, current.read(path).indexOf('[interior]')));
    expect(after.slice(after.indexOf('[health]'))).toBe(current.read(path).slice(current.read(path).indexOf('[health]')));
    const annotated = draft(current.read(path).replace('[health]', '# unrelated next component\n[health]'));
    expect(prepare(annotated, interior)).toContain('# unrelated next component\n[health]');
  });
  it('creates an explicit complete starter without introducing a parser fallback', () => {
    const current = draft(interiorSource().slice(0, interiorSource().indexOf('[interior]'))), read = inspectInterior(current, dependencies, path);
    expect(read.interior).toBeNull();
    const starter = initialInterior(read.systems, read.teamCount);
    expect(parse(prepare(current, starter)).interior.placements).toEqual({ helm: 'bridge', 'shield-arc-fore': 'bridge' });
    expect(starter.team_homes).toEqual(['bridge']);
  });
  it('materialises the complete inherited profile atomically and clears removed provenance', () => {
    const source = interiorSource(), current = draft('includes=["base.toml"]\n# exact child\n'), deps = { base_files: { [base]: source }, packs: [] };
    const read = inspectInterior(current, deps, path), interior = read.interior;
    expect(() => prepare(current, interior, {}, deps)).toThrow(/materialise/);
    interior.compartment = [interior.compartment[0]]; interior.deck = [interior.deck[0]]; interior.connection = [];
    interior.team_homes = ['bridge']; interior.placements['shield-arc-fore'] = 'bridge';
    const after = prepare(current, interior, { materialise: true }, deps);
    const resolved = resolveTemplate(path, { [path]: after, [base]: source }, parse);
    expect(resolved.resolved.value.interior).toEqual(interior);
    expect([...resolved.resolved.provenance.fields].filter(([field]) => field.startsWith('interior.')).every(([, origin]) => origin.source === path)).toBe(true);
    expect(deps.base_files[base]).toBe(source);
  });
  it.each([
    ['unplaced generated equipment', profile => delete profile.placements['shield-arc-fore']],
    ['foreign placement', profile => profile.placements.foreign = 'bridge'],
    ['unknown home', profile => profile.team_homes[0] = 'void'],
    ['wrong team count', profile => profile.team_homes = []],
    ['nonfinite geometry', profile => profile.compartment[0].x = NaN],
    ['zero rectangle', profile => profile.compartment[0].width = 0],
    ['negative deck weight', profile => profile.deck[0].damage_weight = -1],
    ['unknown boundary', profile => profile.connection[0].boundary = 'foreign'],
    ['disconnected compartments', profile => profile.connection = []],
    ['duplicate identities', profile => profile.compartment[1].id = 'bridge'],
  ])('refuses %s without modifying the draft', (_, change) => {
    const current = draft(), before = current.read(path), profile = interiorProfile(); change(profile);
    expect(() => prepare(current, profile)).toThrow(); expect(current.read(path)).toBe(before);
  });
  it('rejects overlapping footprints on the same physical deck', () => {
    const current = draft(), profile = interiorProfile(); profile.compartment[1].deck = 'upper'; profile.compartment[1].x = -10;
    expect(() => validateInterior(profile, inspectInterior(current, dependencies, path).systems, 1)).toThrow(/overlap/);
  });
  it('accepts through runtime validation with undo/redo/export, and preserves source on refusal', async () => {
    const current = draft(), before = current.read(path), runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const profile = interiorProfile(); profile.connection[0].travel_secs = 8;
    await applyInteriorOperation({ draft: current, runtime, operation: { path, interior: profile } });
    const after = current.read(path); current.undo(); expect(current.read(path)).toBe(before); current.redo(); expect(current.read(path)).toBe(after);
    expect(parse(new WorkshopDocument(current.archive()).read(path)).interior.connection[0].travel_secs).toBe(8);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'Boundary System has no supported gate component' }] });
    profile.compartment[0].width = 15;
    await expect(applyInteriorOperation({ draft: current, runtime, operation: { path, interior: profile } })).rejects.toThrow();
    expect(current.read(path)).toBe(after); expect(current.snapshot().history.undo).toHaveLength(1);
  });
  it('changes repair team count and chosen homes in the same source transaction', () => {
    const current = draft(), profile = interiorProfile(); profile.team_homes.push('bridge');
    const after = parse(prepare(current, profile, { teamCount: 2 }));
    expect(after.repair.repair_team_count).toBe(2); expect(after.interior.team_homes).toEqual(['engineering', 'bridge']);
    const baseSource = interiorSource(), inherited = draft('includes=["base.toml"]\n');
    const deps = { base_files: { [base]: baseSource }, packs: [] };
    expect(() => prepare(inherited, profile, { teamCount: 2 }, deps)).toThrow(/materialise/);
    const resolved = resolveTemplate(path, { [path]: prepare(inherited, profile, { teamCount: 2, materialise: true }, deps), [base]: baseSource }, parse).resolved.value;
    expect(resolved.repair.repair_team_count).toBe(2); expect(resolved.interior.team_homes).toEqual(profile.team_homes);
  });
  it('adds, renames and removes explicit Systems together with their physical placements', () => {
    const value = parse(interiorSource()); delete value.shield_arc; value.tags = ['ship'];
    value.station = [{ id: 'bridge', name: 'Bridge', description: 'Bridge', rank: '' }];
    value.console = [{ id: 'flight', home_station: 'bridge', screen: 'gui/helm.html' }]; value.system[0].console = 'flight';
    value.interior.placements = { helm: 'bridge' };
    const current = draft(stringify(value)), schema = { system_kinds: ['helm_thrust', 'viewscreen'] };
    const op = body => ({ path, ...body });
    const moved = prepareShipOperation(current, dependencies, schema, op({ type: 'system-update', id: 'helm', fields: { kind: 'helm_thrust' }, compartment: 'engineering' })).changes[0].after;
    expect(parse(moved).interior.placements.helm).toBe('engineering');
    const add = op({ type: 'system-add', id: 'display', kind: 'viewscreen', console: 'flight' });
    expect(() => prepareShipOperation(current, dependencies, schema, add)).toThrow(/physical compartment/);
    const added = prepareShipOperation(current, dependencies, schema, { ...add, compartment: 'engineering' }).changes[0].after;
    expect(parse(added).interior.placements.display).toBe('engineering');
    const renamed = prepareShipOperation(draft(added), dependencies, schema, op({ type: 'system-set', id: 'display', field: 'id', value: 'monitor' })).changes[0].after;
    expect(parse(renamed).interior.placements).toEqual({ helm: 'bridge', monitor: 'engineering' });
    const removed = prepareShipOperation(draft(renamed), dependencies, schema, op({ type: 'system-remove', id: 'monitor' })).changes[0].after;
    expect(parse(removed).interior.placements).toEqual({ helm: 'bridge' });
    value.interior.connection[0].boundary = 'helm';
    expect(() => prepareShipOperation(draft(stringify(value)), dependencies, schema, op({ type: 'system-remove', id: 'helm' }))).toThrow(/boundary connections/);
  });
  it('adds and removes generated shield placements with the existing energy transaction', () => {
    const current = draft(shieldSource()), read = inspectInterior(current, dependencies, path), profile = initialInterior(read.systems, read.teamCount);
    const source = prepare(current, profile), arc = { ...defaultShieldArc(), id: 'port', label: 'Port' };
    expect(() => prepareShieldOperation(draft(source), dependencies, { path, arc })).toThrow(/physical compartment/);
    const added = prepareShieldOperation(draft(source), dependencies, { path, arc, compartment: 'bridge' }).changes[0].after;
    expect(parse(added).interior.placements['shield-arc-port']).toBe('bridge');
    expect(inspectShields(draft(added), dependencies, path).arcs).toHaveLength(3);
    const removed = prepareShieldOperation(draft(added), dependencies, { path, originalId: 'port', remove: true }).changes[0].after;
    expect(parse(removed).interior.placements['shield-arc-port']).toBeUndefined(); expect(parse(removed).energy.consumer).toHaveLength(2);
    const rooms = interiorProfile(); rooms.team_homes = []; rooms.placements = { 'shield-arc-fore': 'bridge', 'shield-arc-aft': 'bridge' };
    const twoRooms = prepare(current, rooms), fore = inspectShields(draft(twoRooms), dependencies, path).arcs[0];
    const moved = prepareShieldOperation(draft(twoRooms), dependencies, { path, originalId: 'fore', arc: fore, compartment: 'engineering' }).changes[0].after;
    expect(parse(moved).interior.placements['shield-arc-fore']).toBe('engineering');
  });
  it('requires actual Repair knowledge before admitting a newly authored gate', () => {
    const value = parse(interiorSource()); delete value.shield_arc; value.tags = ['ship'];
    value.station = [{ id: 'bridge', name: 'Bridge', description: 'Bridge', rank: '' }];
    value.console = [{ id: 'flight', home_station: 'bridge', screen: 'gui/helm.html' }]; value.system[0].console = 'flight'; value.interior.placements = { helm: 'bridge' };
    const schema = { system_kinds: ['helm_thrust', 'repair', 'access-gate'], console_publication_kinds: { repair: 'Repair' }, console_feed_kinds: ['Repair'] };
    const operation = { path, type: 'system-add', id: 'hatch', kind: 'access-gate', console: 'flight', compartment: 'bridge', ai: initialAccessGateTree(),
      components: [{ id: 'door', kind: 'access_boundary', sealed_channel: 'sealed', transition_secs: 1 }], channels: [{ name: 'sealed', kind: 'boolean' }] };
    expect(() => prepareShipOperation(draft(stringify(value)), dependencies, schema, operation)).toThrow(/typed Repair observation feed/);
    value.console[0].feed = [{ source: 'repair', kind: 'Repair' }];
    const fed = prepareShipOperation(draft(stringify(value)), dependencies, schema, operation).changes[0].after;
    expect(parse(fed).interior.placements.hatch).toBe('bridge');
    delete value.console[0].feed; value.system.push({ id: 'repair', kind: 'repair', console: 'flight' }); value.interior.placements.repair = 'engineering';
    expect(() => prepareShipOperation(draft(stringify(value)), dependencies, schema, operation)).not.toThrow();
  });
});
