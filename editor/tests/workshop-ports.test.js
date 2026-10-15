import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectPortServices, validatePortService, preparePortServiceOperation, applyPortServiceOperation } from '../workshop-ports.js';
import { prepareResourceOperation } from '../workshop-resources.js';
import { resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const resolve = file => resolveTemplate(file, { read: name => fs.existsSync(name) ? fs.readFileSync(name, 'utf8') : null }, parse).resolved.value;
const make = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
const fixture = () => resolve('assets/entities/station_axiom.toml');
describe('authorised finite provider source authoring', () => {
  it('binds only existing paid physical Dock equipment and preserves explicit legacy omission', () => {
    const value = fixture(); expect(value.port_service.system).toBe('dock'); validatePortService(value.port_service, value);
    for (const mutate of [v => v.port_service.system = 'missing', v => v.port_service.system = null,
      v => v.system.find(row => row.id === 'dock').component = [],
      v => v.system.find(row => row.id === 'dock').damage_component = [],
      v => v.system.find(row => row.id === 'dock').console = 'absent',
      v => v.energy.consumer.find(row => row.system === 'dock').continuous_cost_per_sec = 0,
      v => delete v.interior.placements.dock]) {
      const candidate = structuredClone(value); mutate(candidate); expect(() => validatePortService(candidate.port_service, candidate)).toThrow();
    }
    const legacy = structuredClone(value.port_service); delete legacy.system; validatePortService(legacy, value);
    const source = stringify(value), draft = make(source), after = preparePortServiceOperation(draft, deps, { path, value: legacy }).changes[0].after;
    expect(parse(after).port_service.system).toBeUndefined(); expect(parse(after).infrastructure).toEqual(value.infrastructure);
    const manual = resolve('assets/entities/manual_trade_port.toml'); expect(manual.port_service.system).toBe('dock'); validatePortService(manual.port_service, manual);
  });
  it('refuses a header-like multiline string without touching unrelated source', () => {
    const value = fixture(), draft = make('description="""\n[port_service]\nsecret=1\n"""\n' + stringify(value));
    expect(() => preparePortServiceOperation(draft, deps, { path, value: value.port_service })).toThrow();
    expect(draft.snapshot().history.undo).toHaveLength(0);
  });
  it('edits explicit lot quotas through accepted source/history/export and leaves rejected drafts untouched', async () => {
    const value = fixture(), source = '# header\n' + stringify(value) + '\n# tail\n', draft = make(source), profile = structuredClone(value.port_service); profile.offer[0].quota = 3;
    const runtime = { validate: vi.fn(async () => ({ accepted: true })) };
    await applyPortServiceOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile } }); const after = draft.read(path);
    expect(after).toContain('# tail\n'); expect(parse(after).infrastructure).toEqual(value.infrastructure); expect(parse(after).port_service).toEqual(profile);
    draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(inspectPortServices(new WorkshopDocument(draft.archive()), deps, path).port).toEqual(profile);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'terms refused' }] });
    await expect(applyPortServiceOperation({ draft, runtime, dependencies: deps, operation: { path, value: value.port_service } })).rejects.toThrow(); expect(draft.read(path)).toBe(after);
  });
  it('requires explicit inherited materialisation and atomically replaces terms and provenance', () => {
    const value = fixture(), source = 'includes=["base.toml"]\n', draft = make(source), baseSource = stringify(value), dependencies = { base_files: { [base]: baseSource }, packs: [] };
    const profile = { ...value.port_service, eligible_factions: [], offer: [value.port_service.offer[0]] };
    expect(() => preparePortServiceOperation(draft, dependencies, { path, value: profile })).toThrow(/materialise/);
    const after = preparePortServiceOperation(draft, dependencies, { path, value: profile, materialise: true }).changes[0].after;
    expect(resolveTemplate(path, { [path]: after, [base]: baseSource }, parse).resolved.value.port_service).toEqual(profile); expect(draft.read(path)).toBe(source);
  });
  it.each([p => { p.eligible_factions = ['invalid']; }, p => { p.offer = []; }, p => { p.offer[0].unit_amount = 0; },
    p => { p.offer[0].quota = -1; }, p => { p.offer[0].cost = [{ resource: 'alloy', amount: 1 }]; }, p => { p.offer.push({ ...p.offer[0], id: 'duplicate-resource' }); },
    p => { p.offer[0].resource = 'absent'; }, p => { p.offer[0].quota = Number.MAX_SAFE_INTEGER; p.offer[0].unit_amount = 10; }, p => { p.currency = 'credits'; }])('rejects unsupported provider terms', mutate => {
    const value = fixture(), profile = structuredClone(value.port_service); mutate(profile); expect(() => validatePortService(profile, value)).toThrow();
  });
  it('allows explicit deny-all and finite manual stock costs, while guarding endpoint removal', () => {
    const value = resolve('assets/entities/manual_trade_port.toml'); validatePortService(value.port_service, value);
    validatePortService({ ...value.port_service, eligible_factions: [] }, value);
    expect(() => prepareResourceOperation(make(stringify(value)), deps, { path, type: 'service-set', name: 'alloy', endpoint: { kind: 'capacity', id: 'missing-stock' } })).toThrow(/Ports first/);
    expect(value.infrastructure.capacity.find(row => row.id === 'alloy')).toMatchObject({ amount: 0, ceiling: 10000 });
    expect(value.infrastructure.capacity.find(row => row.id === 'machinery_parts').amount).toBe(10000);
  });
  it('preserves actual managed commodity identity and rejects two resource names for one endpoint', () => {
    const value = fixture(), profile = structuredClone(value.port_service);
    value.resource_services.alloy = { kind: 'capacity', id: 'machinery_parts' };
    expect(() => validatePortService(profile, value)).toThrow(/two different resources/);
    const ship = resolve('assets/entities/alliance_cruiser.toml');
    ship.resource_services.machinery_parts = { kind: 'capacity', id: 'hold-ammunition' };
    const offer = { policy: 'free_allied_replenishment', eligible_factions: [], offer: [{ id: 'parts', resource: 'machinery_parts', quota: 1, unit_amount: 1 }] };
    expect(() => validatePortService(offer, ship)).toThrow(/actual resource identity/);
  });
  it('keeps legacy free service amounts in single units and creates no starting reserve fuel', () => {
    const value = fixture(); validatePortService(value.port_service, value);
    expect(value.port_service.offer.every(row => row.unit_amount === 1 && row.cost.length === 0)).toBe(true);
    expect(value.port_service.offer.find(row => row.resource === 'reserve_fuel')).toMatchObject({ quota: 10000 });
    expect(value.infrastructure.capacity.find(row => row.id === 'reserve_fuel')).toMatchObject({ amount: 0, ceiling: 10000 });
    expect(value.infrastructure.capacity.find(row => row.id === 'torpedoes')).toMatchObject({ amount: 1000, ceiling: 1000 });
    expect(value.port_service.offer.find(row => row.resource === 'torpedoes')).toMatchObject({ quota: 100 });
    for (const resource of ['coolant', 'machinery_parts', 'structural_parts', 'alloy']) expect(value.infrastructure.capacity.find(row => row.id === resource).amount).toBe(10000);
    for (const hull of ['alliance_cruiser', 'dynasty_player_cruiser']) {
      const ship = resolve(`assets/entities/${hull}.toml`); expect(ship.umbilical.allowed_resources).toContain('torpedoes');
      expect(ship.torpedoes.initial_count + ship.infrastructure.capacity.find(row => row.id === 'hold-ammunition').amount).toBe(6);
    }
    const skyway = parse(fs.readFileSync('assets/worlds/falling_skyway.toml', 'utf8')).entity.find(row => row.id === 'skyway_control').overrides;
    for (const resource of ['reserve_fuel', 'torpedoes']) expect(skyway.infrastructure.capacity.find(row => row.id === resource)).toEqual(value.infrastructure.capacity.find(row => row.id === resource));
  });
  it('authors clear mated poses and identical source/recreated berth points without changing LODs or model scale', () => {
    const station = fixture(), ship = resolve('assets/entities/alliance_cruiser.toml');
    expect(station.system.find(row => row.id === station.port_service.system)?.component.some(row => row.kind === 'dock_mating')).toBe(true);
    const rig = parse(fs.readFileSync('assets/models/alliance_starbase_recreated.model.toml', 'utf8'));
    const source = parse(fs.readFileSync('assets/models/alliance_starbase.model.toml', 'utf8'));
    const own = parse(fs.readFileSync('assets/models/alliance_cruiser_recreated.model.toml', 'utf8'));
    const radius = ship.collider.radius + ship.collider.length / 2;
    for (const [name, target] of Object.entries(rig.markers)) {
      const original = source.markers[name];
      original.position.forEach((position, axis) => expect(position * source.base.scale[axis] + source.base.offset[axis]).toBeCloseTo(target.position[axis], 8));
      for (const [ownName, marker] of Object.entries(own.markers).filter(([id]) => id.startsWith('dock'))) {
        const yaw = Math.atan2(target.direction[0], target.direction[2]) - Math.atan2(-marker.direction[0], -marker.direction[2]);
        const c = Math.cos(yaw), s = Math.sin(yaw), x = target.position[0] - (c * marker.position[0] + s * marker.position[2]), z = target.position[2] - (-s * marker.position[0] + c * marker.position[2]);
        expect(Math.hypot(x, z) - station.collider.radius - radius, name + ownName).toBeGreaterThan(4);
      }
    }
  });
});
