import fs from 'node:fs';
import { posix } from 'node:path';
import { expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectHelmMovement, prepareHelmMovementOperation, applyHelmMovementOperation, validateHelmMovement, guardHelmMovementReferences } from '../workshop-helm-movement.js';
export const path = 'assets/entities/test.toml', profile = { thrust: 'drive', steering: 'rudder', lateral: 'side' };
export const source = `# hull\n[helm_console]\nmax_speed=20 # untouched\n[[console]]\nid="helm"\n[[system]]\nid="drive"\nkind="helm_thrust"\nconsole="helm"\n[[system]]\nid="rudder"\nkind="helm_steering"\nconsole="helm"\n[[system]]\nid="side"\nkind="lateral_thrust"\nconsole="helm"\n`;
export const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
const deps = { base_files: {}, packs: [] };
it('supports closed actual bindings and guards adjacent owner removal', () => {
  const entity = parse(source); validateHelmMovement(profile, entity);
  for (const value of [true, null, {}, { ...profile, lateral: null }, { ...profile, steering: 'drive' }, { ...profile, thrust: 'missing' }, { ...profile, other: 1 }])
    expect(() => validateHelmMovement(value, entity)).toThrow();
  expect(() => guardHelmMovementReferences({}, { ...entity, helm_control: profile, system: [] })).toThrow();
});
it('adds bindings through accepted source and preserves unrelated source exactly', async () => {
  const draft = make(source), runtime = { validate: vi.fn(async () => ({ accepted: true })) };
  await applyHelmMovementOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile } });
  expect(draft.read(path).startsWith(source)).toBe(true); expect(parse(draft.read(path)).helm_control).toEqual(profile);
  draft.undo(); expect(draft.read(path)).toBe(source); draft.redo();
  expect(inspectHelmMovement(new WorkshopDocument(draft.archive()), deps, path).value).toEqual(profile);
});
it('preserves inline and table commentary while refusing source layouts that could alter other data', () => {
  const inline = 'helm_control={thrust="drive",steering="rudder"} # retained choice\n' + source;
  const disabled = prepareHelmMovementOperation(make(inline), deps, { path, value: false }).changes[0].after;
  expect(parse(disabled).helm_control).toBe(false); expect(disabled).toContain('# retained choice'); expect(disabled).toContain(source);
  const nested = source + '\n[helm_control] # binding header\nthrust="drive" # actual owner\nsteering="rudder"\n';
  const enabled = prepareHelmMovementOperation(make(nested), deps, { path, value: profile }).changes[0].after;
  expect(enabled).toContain('# binding header'); expect(enabled).toContain('# actual owner');
  expect(parse(enabled).helm_control).toEqual(profile);
  const tricky = 'description="""\n[helm_control]\nthrust="fiction"\n"""\n' + source;
  const guarded = make(tricky);
  expect(() => prepareHelmMovementOperation(guarded, deps, { path, value: profile })).toThrow();
  expect(guarded.read(path)).toBe(tricky); expect(guarded.snapshot().history.undo).toHaveLength(0);
});
it('explicitly disables and re-enables inherited bindings and clears inherited lateral atomically', async () => {
  const text = 'includes=["base.toml"]\n# child\n', dependencies = { base_files: { 'assets/entities/base.toml': source + '\n[helm_control]\nthrust="drive"\nsteering="rudder"\nlateral="side"\n' }, packs: [] };
  const draft = make(text), runtime = { validate: vi.fn(async () => ({ accepted: true })) };
  expect(() => prepareHelmMovementOperation(draft, dependencies, { path, value: false })).toThrow(/materialise/);
  await applyHelmMovementOperation({ draft, runtime, dependencies, operation: { path, value: false, materialise: true } });
  expect(inspectHelmMovement(new WorkshopDocument(draft.archive()), dependencies, path).value).toBe(false);
  draft.undo(); expect(draft.read(path)).toBe(text); draft.redo();
  const withoutLateral = { thrust: 'drive', steering: 'rudder' };
  await applyHelmMovementOperation({ draft, runtime, dependencies, operation: { path, value: withoutLateral } });
  expect(inspectHelmMovement(new WorkshopDocument(draft.archive()), dependencies, path).value).toEqual(withoutLateral);
});
it('refuses acceptance and stale results without writing a candidate', async () => {
  const draft = make(source), runtime = { validate: vi.fn(async () => ({ accepted: false })) };
  const options = { draft, runtime, dependencies: deps, operation: { path, value: profile } };
  await expect(applyHelmMovementOperation(options)).rejects.toThrow(); expect(draft.snapshot().history.undo).toHaveLength(0);
  let answer; runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }));
  const pending = applyHelmMovementOperation(options); await vi.waitFor(() => expect(answer).toBeTypeOf('function'));
  draft.apply([{ path, before: source, after: source + '# intervening edit\n' }]); answer({ accepted: true });
  await expect(pending).rejects.toThrow(/stale/); expect(draft.read(path)).toBe(source + '# intervening edit\n');
});
it('edits the actual inherited Cruiser without retaining an inherited lateral owner', async () => {
  const hull = 'assets/entities/dynasty_player_cruiser.toml', before = fs.readFileSync(hull, 'utf8'), base_files = {};
  const collect = file => { for (const include of parse(fs.readFileSync(file, 'utf8')).includes || []) {
    const child = posix.normalize(posix.join(posix.dirname(file), include)); if (!(child in base_files)) { base_files[child] = fs.readFileSync(child, 'utf8'); collect(child); }
  } }; collect(hull);
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path: hull, text: before }]));
  const dependencies = { base_files, packs: [] }, original = inspectHelmMovement(draft, dependencies, hull);
  expect(original.value.lateral).toBe('helm-lateral-thrust');
  const value = { thrust: original.value.thrust, steering: original.value.steering };
  await applyHelmMovementOperation({ draft, dependencies, runtime: { validate: async () => ({ accepted: true }) }, operation: { path: hull, value, materialise: true } });
  const after = inspectHelmMovement(new WorkshopDocument(draft.archive()), dependencies, hull);
  expect(after.value).toEqual(value); expect(after.resolved.value.helm_console).toEqual(original.resolved.value.helm_console);
});
