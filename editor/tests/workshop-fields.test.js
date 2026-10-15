import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectFields, validateFields, prepareFieldsOperation, applyFieldsOperation } from '../workshop-fields.js';
import { resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const value = () => ({ shape: { type: 'sphere', radius: 10 }, field: { falloff: 'linear', strength: 1, sensor_absorption_per_unit: 0.1, comms_absorption_per_unit: 0.2 }, signal_material: null });
const make = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
describe('shared field source authoring', () => {
  it('accepts one physical profile transaction with exact undo/redo/export and preserves unrelated comments/effects', async () => {
    const source = '# source\ntags=["region"]\n[shape]\ntype="sphere"\nradius=5\n\n# effects remain\n[effects.blocks_impulse]\n', draft = make(source), environment = value();
    environment.field.evolution = { duration_secs: 12, translation: [1,2,3], final_scale: 2, final_strength: 0 };
    const runtime = { validate: vi.fn(async () => ({ accepted: true })) };
    await applyFieldsOperation({ draft, runtime, dependencies: deps, operation: { path, value: environment } });
    const after = draft.read(path); expect(after).toContain('# effects remain'); expect(parse(after).effects).toEqual(parse(source).effects);
    expect(parse(after).field).toEqual(environment.field); expect(runtime.validate).toHaveBeenCalledOnce();
    draft.undo(); expect(draft.read(path)).toBe(source); draft.redo();
    expect(inspectFields(new WorkshopDocument(draft.archive()), deps, path).environment).toEqual(environment);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'runtime refusal' }] });
    await expect(applyFieldsOperation({ draft, runtime, dependencies: deps, operation: { path, value: value() } })).rejects.toThrow(); expect(draft.read(path)).toBe(after);
  });
  it('requires explicit inheritance materialisation and clears old evolution/provenance atomically', () => {
    const parent = value(); parent.field.evolution = { duration_secs: 10, translation: [1,0,0], final_scale: 2, final_strength: 1 };
    const baseSource = stringify({ tags: ['region'], shape: parent.shape, field: parent.field }), source = 'includes=["base.toml"]\n', draft = make(source), dependencies = { base_files: { [base]: baseSource }, packs: [] };
    expect(() => prepareFieldsOperation(draft, dependencies, { path, value: value() })).toThrow(/materialise/);
    const after = prepareFieldsOperation(draft, dependencies, { path, value: value(), materialise: true }).changes[0].after;
    const resolved = resolveTemplate(path, { [path]: after, [base]: baseSource }, parse);
    expect(resolved.resolved.value.field).not.toHaveProperty('evolution'); expect(draft.read(path)).toBe(source);
    expect(() => prepareFieldsOperation(draft, dependencies, { path, value: { shape: null, field: null, signal_material: null }, materialise: true })).toThrow(/inherited environment/);
  });
  it.each([v => { v.shape.radius = 0; }, v => { v.shape.radius = 1e-50; }, v => { v.shape.radius = Infinity; },
    v => { v.field.falloff = 'code'; }, v => { v.field.strength = NaN; }, v => { v.field.comms_absorption_per_unit = -1; },
    v => { v.field.evolution = { duration_secs: 0, translation: [0,0,0], final_scale: 1, final_strength: 1 }; },
    v => { v.shape = { type: 'box', half_extents: [1,1], yaw: 0 }; }, v => { v.shape = { type: 'torus', inner_radius: 3, outer_radius: 2 }; },
    v => { v.signal_material = { sensor_opaque: true }; }, v => { v.shape = null; }])('refuses unsupported or nonphysical environment %#', mutate => {
      const environment = value(); mutate(environment); expect(() => validateFields(environment)).toThrow();
  });
  it('cannot remove geometry still used by legacy effects', () => {
    expect(() => validateFields({ shape: null, field: null, signal_material: null }, { effects: { blocks_impulse: {} } })).toThrow(/shape/);
  });  it('authors explicit substantial bounds only on variants and a causal quiet probe without endpoint double charging', () => {
    for (const stem of ['signal_planet_earth', 'signal_moon_luna', 'signal_asteroid_huge']) {
      const resolved = resolveTemplate('assets/entities/' + stem + '.toml', { read: path => fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : null }, parse).resolved.value;
      validateFields({ shape: resolved.shape, field: null, signal_material: resolved.signal_material }, resolved);
      expect(resolved.shape.radius).toBe(resolved.collider.radius); expect(resolved.signal_material.sensor_opaque && resolved.signal_material.comms_opaque).toBe(true);
    }
    for (const stem of ['planet_earth', 'moon_luna', 'asteroid_common_1_huge', 'asteroid_common_1_small']) expect(parse(fs.readFileSync('assets/entities/' + stem + '.toml', 'utf8'))).not.toHaveProperty('signal_material');
    const cloud = parse(fs.readFileSync('assets/entities/region_signal_cloud.toml', 'utf8'));
    validateFields({ shape: cloud.shape, field: cloud.field, signal_material: null }, cloud);
    expect(cloud.effects).not.toHaveProperty('radar_dampening'); expect(cloud.effects).not.toHaveProperty('comms_jammed');
    const world = parse(fs.readFileSync('assets/worlds/probe_signal_fields.toml', 'utf8'));
    expect(world).not.toHaveProperty('deadline'); expect(world.script.setup).toContain('on_flag_set("scan.field-control.taken"');
    expect(world.script.setup).toContain('ctx.effects.replace_region_field("moving-cloud"');
  });

});
