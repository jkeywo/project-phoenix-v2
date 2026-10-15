import fs from 'node:fs';
import { posix } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { resolveTemplate } from '../entity-includes.js';
import { validateHelmProjection, inspectHelmProjection, prepareHelmProjectionOperation, applyHelmProjectionOperation } from '../workshop-helm-projection.js';

const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
const edit = (draft, value, dependencies = deps, materialise = false) => prepareHelmProjectionOperation(draft, dependencies, { path, value, materialise }).changes[0].after;

describe('accepted Helm projection source', () => {
  it.each([true, null, {}, { horizon_secs: 0 }, { horizon_secs: -1 }, { horizon_secs: Infinity }, { horizon_secs: '30' }, { horizon_secs: 1e-46 }, { horizon_secs: 1e40 }, { horizon_secs: 30, extra: true }])('refuses invalid policies without touching source', value => {
    const before = '[helm_console]\nturn_rate = 2\n', draft = make(before);
    expect(() => edit(draft, value)).toThrow(); expect(draft.read(path)).toBe(before);
    expect(draft.snapshot().history.undo).toHaveLength(0);
  });
  it('adds an absent policy and preserves movement, radar and comments exactly', () => {
    const source = '# ship\n[helm_console] # controls\nturn_rate = 2 # keep\n\n[helm_console.radar]\nrange = 120 # reach\n';
    const draft = make(source), after = edit(draft, { horizon_secs: 45 });
    expect(after).toBe(source.replace('[helm_console] # controls\n', '[helm_console] # controls\nprojection = { horizon_secs = 45 }\n'));
    expect(parse(after).helm_console).toEqual({ projection: { horizon_secs: 45 }, turn_rate: 2, radar: { range: 120 } });
    expect(draft.read(path)).toBe(source);
    expect(() => edit(make('name="no helm"\n'), false)).toThrow(/actual Helm/);
  });
  it('disables nested projection without deleting its commentary or rewriting siblings', () => {
    const source = '# ship\r\n[helm_console]\r\nturn_rate = 2\r\n[helm_console.projection] # estimate\r\n# purpose\r\nhorizon_secs = 30 # horizon\r\n[helm_console.radar]\r\nrange = 120 # reach\r\n';
    const after = edit(make(source), false);
    expect(parse(after).helm_console.projection).toBe(false);
    expect(after).toContain('turn_rate = 2\r\n');
    expect(after).toContain('# estimate\r\n# purpose\r\n# horizon\r\n[helm_console.radar]\r\nrange = 120 # reach\r\n');
    expect(after.replaceAll('\r\n', '')).not.toContain('\n');
    const enabled = edit(make(after), { horizon_secs: 12.5 });
    expect(parse(enabled).helm_console.projection).toEqual({ horizon_secs: 12.5 });
    expect(enabled).toContain('# horizon\r\n[helm_console.radar]');
  });
  it('preserves inline trailing comments and handles a header with no final newline', () => {
    expect(edit(make('[helm_console]'), false)).toBe('[helm_console]\nprojection = false\n');
    const source = '[helm_console]\nprojection = false # choice\nturn_rate = 2\n';
    expect(edit(make(source), { horizon_secs: 20 })).toBe(source.replace('false', '{ horizon_secs = 20 }'));
    const bomDraft = make('\uFEFF' + source);
    // The shared include parser refuses BOM source; this form must retain it.
    expect(() => edit(bomDraft, { horizon_secs: 20 })).toThrow(/TOML/);
    expect(bomDraft.read(path)).toBe('\uFEFF' + source);
    expect(() => edit(make('helm_console = { projection = false, turn_rate = 2 }\n'), { horizon_secs: 2 })).toThrow(/Expand/);
    const tricky = 'description="""\n[helm_console]\nprojection=false\n"""\n[helm_console]\nturn_rate=2\n';
    expect(() => edit(make(tricky), { horizon_secs: 2 })).toThrow(/Expand/);
  });
  it('explicitly overrides inherited table with false, then inherited false with a policy', async () => {
    const parent = '[helm_console]\nturn_rate = 2\n[helm_console.projection]\nhorizon_secs = 30\n[helm_console.radar]\nrange = 120\n';
    const source = 'includes=["base.toml"]\n# child\n', draft = make(source), dependencies = { base_files: { [base]: parent }, packs: [] };
    expect(inspectHelmProjection(draft, dependencies, path).inherited).toBe(true);
    expect(() => edit(draft, false, dependencies)).toThrow(/materialise/);
    const runtime = { validate: vi.fn(async () => ({ accepted: true })) };
    await applyHelmProjectionOperation({ draft, runtime, dependencies, operation: { path, value: false, materialise: true } });
    const disabled = draft.read(path);
    expect(resolveTemplate(path, { [path]: disabled, [base]: parent }, parse).resolved.value.helm_console).toEqual({ turn_rate: 2, radar: { range: 120 }, projection: false });
    expect(inspectHelmProjection(new WorkshopDocument(draft.archive()), dependencies, path).projection).toBe(false);
    draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toBe(disabled);
    const fromFalse = { base_files: { [base]: '[helm_console]\nturn_rate = 2\nprojection = false\n' }, packs: [] };
    const enabled = edit(make(source), { horizon_secs: 60 }, fromFalse, true);
    expect(resolveTemplate(path, { [path]: enabled, ...fromFalse.base_files }, parse).resolved.value.helm_console).toEqual({ turn_rate: 2, projection: { horizon_secs: 60 } });
  });
  it('accepts before mutation and refuses authoritative or stale results without history changes', async () => {
    const source = '[helm_console]\nturn_rate = 2\n', draft = make(source), runtime = { validate: vi.fn(async () => ({ accepted: false, findings: [{ message: 'policy refused' }] })) };
    const options = { draft, runtime, dependencies: deps, operation: { path, value: { horizon_secs: 30 } } };
    await expect(applyHelmProjectionOperation(options)).rejects.toThrow(); expect(draft.read(path)).toBe(source);
    expect(draft.snapshot().history.undo).toHaveLength(0);
    let answer; runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }));
    const pending = applyHelmProjectionOperation(options); await vi.waitFor(() => expect(answer).toBeTypeOf('function'));
    draft.apply([{ path, before: source, after: source + '# intervening edit\n' }]); answer({ accepted: true });
    await expect(pending).rejects.toThrow(/stale/); expect(draft.read(path)).toBe(source + '# intervening edit\n');
    expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it('disables and re-enables the actual inherited Dynasty horizon without rewriting its Helm AI', async () => {
    const hull = 'assets/entities/dynasty_player_cruiser.toml', source = fs.readFileSync(hull, 'utf8'), base_files = {};
    const collect = file => {
      const text = fs.readFileSync(file, 'utf8');
      for (const include of parse(text).includes || []) {
        const child = posix.normalize(posix.join(posix.dirname(file), include));
        if (!(child in base_files)) { base_files[child] = fs.readFileSync(child, 'utf8'); collect(child); }
      }
    };
    collect(hull);
    const dependencies = { base_files, packs: [] }, draft = new WorkshopDocument(createStoreZip([
      { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path: hull, text: source },
    ])), runtime = { validate: vi.fn(async () => ({ accepted: true })) };
    const before = inspectHelmProjection(draft, dependencies, hull);
    expect(before.inherited).toBe(true); expect(before.projection).toEqual({ horizon_secs: 60 });
    await applyHelmProjectionOperation({ draft, runtime, dependencies, operation: { path: hull, value: false, materialise: true } });
    let resolved = inspectHelmProjection(new WorkshopDocument(draft.archive()), dependencies, hull).resolved.value;
    expect(resolved.helm_console).toEqual({ ...before.resolved.value.helm_console, projection: false });
    expect(draft.read(hull).startsWith(source)).toBe(true);
    draft.undo(); expect(draft.read(hull)).toBe(source); draft.redo();
    await applyHelmProjectionOperation({ draft, runtime, dependencies, operation: { path: hull, value: { horizon_secs: 24 } } });
    resolved = inspectHelmProjection(new WorkshopDocument(draft.archive()), dependencies, hull).resolved.value;
    expect(resolved.helm_console).toEqual({ ...before.resolved.value.helm_console, projection: { horizon_secs: 24 } });
  });
  it('accepts a finite f32 policy and the explicit disabled scalar', () => {
    expect(() => validateHelmProjection({ horizon_secs: 0.1 })).not.toThrow();
    expect(() => validateHelmProjection(false)).not.toThrow();
  });
});
