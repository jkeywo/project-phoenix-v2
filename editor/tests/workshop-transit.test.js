import fs from 'node:fs';
import { posix } from 'node:path';
import { expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectTransit, validateTransitEntity, prepareTransitOperation, applyTransitOperation, guardTransitReferences } from '../workshop-transit.js';
const path = 'assets/entities/test-jump.toml';
function fixture(child = false) {
  const base = 'assets/entities/local_jump_cruiser.toml', base_files = {};
  const collect = file => { base_files[file] = fs.readFileSync(file, 'utf8');
    for (const include of parse(base_files[file]).includes || []) collect(posix.normalize(posix.join(posix.dirname(file), include))); };
  collect(base);
  const source = child ? '# child keeps commentary\nincludes=["local_jump_cruiser.toml"]\n' : base_files[base];
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  return { draft, source, dependencies: { base_files, packs: [] }, runtime: { validate: vi.fn(async () => ({ accepted: true })) } };
}
function operation(f, change = {}) {
  const system = inspectTransit(f.draft, f.dependencies, path).drives[0], drive = system.component.find(row => row.kind === 'transit_drive');
  return { path, owner: system.id, navigation_source: drive.navigation_source, config: { ...drive.config, max_distance: 400, ...change } };
}
it('edits the real reference profile through accepted history/export while preserving other equipment and comments', async () => {
  const f = fixture(), before = inspectTransit(f.draft, f.dependencies, path).resolved.value;
  await applyTransitOperation({ ...f, operation: operation(f) });
  const after = inspectTransit(f.draft, f.dependencies, path).resolved.value;
  expect(after.energy).toEqual(before.energy); expect(after.logistics).toEqual(before.logistics);
  expect(after.system.find(row => row.id === 'local-jump').component[1].config.max_distance).toBe(400);
  expect(f.draft.read(path)).toContain('# two finite fuel units');
  f.draft.undo(); expect(f.draft.read(path)).toBe(f.source); f.draft.redo();
  expect(inspectTransit(new WorkshopDocument(f.draft.archive()), f.dependencies, path).resolved.value).toEqual(after);
});
it('materialises an inherited actual drive only with explicit consent and preserves the supplying include', async () => {
  const f = fixture(true), op = operation(f, { reading_ticks: 0 });
  expect(() => prepareTransitOperation(f.draft, f.dependencies, op)).toThrow(/materialise/);
  await applyTransitOperation({ ...f, operation: { ...op, materialise: true } });
  const resolved = inspectTransit(new WorkshopDocument(f.draft.archive()), f.dependencies, path);
  expect(resolved.drives[0].component[1].config.reading_ticks).toBe(0);
  expect(resolved.drives[0].component[1].config.max_distance).toBe(400);
  expect(f.draft.read(path)).toContain('# child keeps commentary');
  expect(f.draft.paths()).toEqual(['scenarios.toml', path]);
});
it('refuses invalid finite profiles and broken actual Navigation/Power/Stock references without writing', () => {
  const f = fixture(), entity = inspectTransit(f.draft, f.dependencies, path).resolved.value;
  for (let fault = 0; fault < 13; fault++) {
    const candidate = structuredClone(entity), system = candidate.system.find(row => row.id === 'local-jump'), drive = system.component[1];
    if (fault === 0) drive.config.preparation_work_secs = 1e-100;
    if (fault === 1) drive.config.scatter_radius = Infinity;
    if (fault === 2) drive.config.reading_ticks = -1;
    if (fault === 3) drive.navigation_source = 'helm-thrust';
    if (fault === 4) system.channel = [{ name: 'unused', kind: 'boolean' }];
    if (fault === 5) candidate.energy.consumer.find(row => row.system === system.id).continuous_cost_per_sec = 0;
    if (fault === 6) candidate.logistics.store.find(row => row.id === drive.config.fuel.store).system = 'navigation';
    if (fault === 7) candidate.interior.placements[system.id] = 'bridge';
    if (fault === 8) drive.config.fuel.amount = 13;
    if (fault === 9) candidate.console.find(row => row.id === system.console).feed = [];
    if (fault === 10) candidate.console.find(row => row.id === system.console).feed.find(row => row.source === drive.navigation_source).kind = 'Helm';
    if (fault === 11 || fault === 12) {
      const receiver = fault === 11 ? system : candidate.system.find(row => row.id === candidate.logistics.handling[0].system);
      candidate.console.find(row => row.id === receiver.console).feed = candidate.console.find(row => row.id === receiver.console).feed.filter(row => row.source !== 'stock/' + system.id);
    }
    expect(() => guardTransitReferences(entity, candidate), `fault ${fault}`).toThrow();
  }
  expect(f.draft.read(path)).toBe(f.source);
});
it('rejects refusal/stale candidates and unsupported source rewrites without altering history', async () => {
  const f = fixture(), op = operation(f);
  f.runtime.validate.mockResolvedValueOnce({ accepted: false });
  await expect(applyTransitOperation({ ...f, operation: op })).rejects.toThrow();
  expect(f.draft.read(path)).toBe(f.source);
  let answer; f.runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }));
  const pending = applyTransitOperation({ ...f, operation: op }); await vi.waitFor(() => expect(answer).toBeTypeOf('function'));
  f.draft.apply([{ path, before: f.source, after: f.source + '# newer\n' }]); answer({ accepted: true });
  await expect(pending).rejects.toThrow(/stale/); expect(f.draft.read(path)).toBe(f.source + '# newer\n');
});
it('keeps the fixed-body gravity policy closed and refuses a moving actor policy', () => {
  validateTransitEntity({ arrival_gravity: false });
  validateTransitEntity({ arrival_gravity: { exclusion_radius: 8 }, collider: { movable: false } });
  for (const policy of [true, null, {}, { exclusion_radius: 0 }, { exclusion_radius: 1e100 }, { exclusion_radius: 8, mass: 10 }])
    expect(() => validateTransitEntity({ arrival_gravity: policy })).toThrow();
  expect(() => validateTransitEntity({ arrival_gravity: { exclusion_radius: 8 }, tags: ['ship'] })).toThrow(/fixed/);
});

it('disables and re-enables inherited fixed-body gravity without changing other source', async () => {
  const source = '# authored fixed body\nincludes=["body.toml"]\n', dependencies = { base_files: { 'assets/entities/body.toml': '[arrival_gravity]\nexclusion_radius=8\n' }, packs: [] };
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { validate: vi.fn(async () => ({ accepted: true })) };
  const options = { draft, dependencies, runtime, operation: { type: 'gravity', path, value: false } };
  expect(() => prepareTransitOperation(draft, dependencies, options.operation)).toThrow(/materialise/);
  await applyTransitOperation({ ...options, operation: { ...options.operation, materialise: true } });
  expect(inspectTransit(new WorkshopDocument(draft.archive()), dependencies, path).resolved.value.arrival_gravity).toBe(false);
  draft.undo(); expect(draft.read(path)).toBe(source); draft.redo();
  await applyTransitOperation({ ...options, operation: { type: 'gravity', path, value: { exclusion_radius: 16 } } });
  expect(inspectTransit(new WorkshopDocument(draft.archive()), dependencies, path).resolved.value.arrival_gravity).toEqual({ exclusion_radius: 16 });
  expect(draft.read(path)).toContain('# authored fixed body');
});

it('refuses a multiline source layout that resembles a System header instead of changing unrelated data', () => {
  const f = fixture(), text = f.source.replace('name = "entity.local_jump_cruiser.name"', 'name = """\n[[system]]\nid="local-jump"\n"""');
  f.draft.apply([{ path, before: f.source, after: text }]); const before = f.draft.snapshot();
  expect(() => prepareTransitOperation(f.draft, f.dependencies, operation(f))).toThrow();
  expect(f.draft.snapshot()).toEqual(before);
});

it('accepts finite gravity response through the same drive history and rejects invalid edits without writing', async () => {
  const f=fixture(),before=inspectTransit(f.draft,f.dependencies,path).resolved.value;
  for(const response of [-1,Infinity,NaN,'0.25',1e100]) {
    expect(()=>prepareTransitOperation(f.draft,f.dependencies,operation(f,{gravity_response:response}))).toThrow();
    expect(f.draft.read(path)).toBe(f.source);
  }
  for(const response of [0,0.25,2]) {
    await applyTransitOperation({...f,operation:operation(f,{gravity_response:response})});
    const after=inspectTransit(f.draft,f.dependencies,path).resolved.value;
    expect(after.system.find(row=>row.id==='local-jump').component[1].config.gravity_response).toBe(response);
    expect(after.energy).toEqual(before.energy);expect(after.logistics).toEqual(before.logistics);
    f.draft.undo();expect(f.draft.read(path)).toBe(f.source);
  }
});
