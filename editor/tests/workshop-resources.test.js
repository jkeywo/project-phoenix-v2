import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { prepareResourceOperation, applyResourceOperation, inspectResources } from '../workshop-resources.js';
import { mergeComposeFragments, resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/depot.toml', base = 'assets/entities/base.toml';
const source = `# retain this\n[torpedoes]\ncount=3 # count note\nspeed=10\n[umbilical]\ncapacity="torpedoes"\nrate=1\ndirection="collect"\nmin_power_level=1\n[[infrastructure.capacity]]\nid="ammo"\namount=10 # stock note\nceiling=20\nlabel="stock.label"\n[[infrastructure.capacity]]\nid="fuel"\namount=5\n[appearance]\ncolor=[1,2,3]\n`;
const dependencies = { base_files: {}, packs: [] };
function draft(text = source) {
  return new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text }]));
}
const operation = (current, fields, deps = dependencies) => prepareResourceOperation(current, deps, { path, ...fields }).changes[0].after;
describe('Workshop resource authoring', () => {
  it.each(['\n', '\r\n'])('changes exact capacity scalars while preserving siblings and source comments %j', nl => {
    const before = source.replace(/\n/g, nl), after = operation(draft(before), { type: 'capacity-set', id: 'ammo', amount: 6, ceiling: 25, label: 'label#hash' });
    expect(after.startsWith(before.slice(0, before.indexOf('[[infrastructure.capacity]]')))).toBe(true);
    expect(after).toContain('amount=6 # stock note');
    expect(after.slice(after.indexOf('id="fuel"'))).toBe(before.slice(before.indexOf('id="fuel"')));
    expect(parse(after).infrastructure.capacity[0]).toEqual({ id: 'ammo', amount: 6, ceiling: 25, label: 'label#hash' });
  });
  it('adds absent fields at EOF without requiring a trailing newline', () => {
    const current = draft('[[infrastructure.capacity]]\nid="ammo"\namount=5');
    const after = operation(current, { type: 'capacity-set', id: 'ammo', amount: 5, ceiling: 10 });
    expect(parse(after).infrastructure.capacity[0]).toEqual({ id: 'ammo', amount: 5, ceiling: 10 });
  });
  it('replaces literal labels containing hashes while preserving the real comment', () => {
    const current = draft("[[infrastructure.capacity]]\nid='ammo'\namount=5\nlabel='stock#old' # actual comment\n");
    const after = operation(current, { type: 'capacity-set', id: 'ammo', amount: 5, ceiling: 10, label: 'stock#new' });
    expect(after).toContain('label="stock#new" # actual comment');
    expect(after).not.toContain("#old'");
  });
  it('edits magazine without changing tube or weapon configuration', () => {
    const after = operation(draft(), { type: 'magazine-set', count: 8 });
    expect(after).toContain('count=8 # count note'); expect(parse(after).torpedoes.speed).toBe(10);
  });
  it('adds supported capacity entries', () => {
    const after = operation(draft(), { type: 'capacity-set', id: 'parts', amount: 1, ceiling: 6 });
    expect(parse(after).infrastructure.capacity.map(row => row.id)).toEqual(['ammo', 'fuel', 'parts']);
  });
  it('requires explicit inherited edits and keeps include source untouched', () => {
    const deps = { base_files: { [base]: source }, packs: [] }, current = draft('includes=["base.toml"]\n# child\n');
    expect(() => operation(current, { type: 'capacity-set', id: 'ammo', amount: 1, ceiling: 20 }, deps)).toThrow(/materialise/);
    const after = operation(current, { type: 'capacity-set', id: 'ammo', amount: 1, ceiling: 20, materialise: true }, deps);
    expect(after.startsWith('includes=["base.toml"]\n# child\n')).toBe(true);
    expect(mergeComposeFragments(parse(source), parse(after)).infrastructure.capacity).toEqual([
      { id: 'ammo', amount: 1, ceiling: 20, label: 'stock.label' }, { id: 'fuel', amount: 5 }]);
    expect(deps.base_files[base]).toBe(source);
  });
  it('replaces tagged service/provider variants and prunes obsolete provenance', () => {
    const baseText = source + '[resource_services.torpedoes]\nkind="capacity"\nid="ammo"\n[umbilical.link.provider]\nkind="capacity"\nid="ammo"\n';
    const current = draft('includes=["base.toml"]\n'), deps = { base_files: { [base]: baseText }, packs: [] };
    const after = operation(current, { type: 'service-set', name: 'torpedoes', endpoint: { kind: 'torpedo_magazine' }, materialise: true }, deps);
    const result = resolveTemplate(path, { [path]: after, [base]: baseText }, parse);
    expect(result.ok).toBe(true);
    expect(result.resolved.value.resource_services.torpedoes).toEqual({ kind: 'torpedo_magazine' });
    expect(result.resolved.provenance.fields.has('resource_services.torpedoes.id')).toBe(false);
    const afterLink = operation(draft(after), { type: 'link-set', provider: { kind: 'torpedo_magazine' }, materialise: true }, deps);
    expect(resolveTemplate(path, { [path]: afterLink, [base]: baseText }, parse).resolved.value.umbilical.link.provider).toEqual({ kind: 'torpedo_magazine' });
  });
  it('requires materialisation for quoted service names too', () => {
    const text = source + '[resource_services."ammo.reload"]\nkind="capacity"\nid="ammo"\n';
    const deps = { base_files: { [base]: text }, packs: [] }, current = draft('includes=["base.toml"]\n');
    expect(() => operation(current, { type: 'service-set', name: 'ammo.reload', endpoint: { kind: 'torpedo_magazine' } }, deps)).toThrow(/materialise/);
    const after = operation(current, { type: 'service-set', name: 'ammo.reload', endpoint: { kind: 'torpedo_magazine' }, materialise: true }, deps);
    expect(mergeComposeFragments(parse(text), parse(after)).resource_services['ammo.reload']).toEqual({ kind: 'torpedo_magazine' });
  });
  it('writes named service link, keeping unrelated capacity source', () => {
    const after = operation(draft(), { type: 'link-set', service: 'reload' });
    expect(parse(after).umbilical.link.service).toBe('reload'); expect(after).toContain('# stock note');
  });
  it('switches inherited explicit links back to named services without retaining the provider', () => {
    const text = source + '[umbilical.link.provider]\nkind="capacity"\nid="ammo"\n';
    const deps = { base_files: { [base]: text }, packs: [] }, current = draft('includes=["base.toml"]\n');
    const after = operation(current, { type: 'link-set', service: 'reload', materialise: true }, deps);
    expect(mergeComposeFragments(parse(text), parse(after)).umbilical.link).toEqual({ service: 'reload' });
  });
  it('refuses inline selected source instead of appending duplicate definitions', () => {
    const current = draft('infrastructure={capacity=[{id="ammo",amount=2,ceiling=4}]}\n');
    expect(() => operation(current, { type: 'capacity-set', id: 'ammo', amount: 3, ceiling: 4 })).toThrow(/Expand inline/);
  });
  it('round trips history and archive; refused candidates never land', async () => {
    const current = draft(), runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyResourceOperation({ draft: current, runtime, operation: { path, type: 'capacity-set', id: 'ammo', amount: 7, ceiling: 20 } });
    const after = current.read(path); expect(runtime.validate).toHaveBeenCalledTimes(1);
    current.undo(); expect(current.read(path)).toBe(source); current.redo(); expect(current.read(path)).toBe(after);
    const reload = new WorkshopDocument(current.archive()); expect(inspectResources(reload, dependencies, path).capacities[0].amount).toBe(7);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ severity: 'error', message: 'capacity missing' }] });
    await expect(applyResourceOperation({ draft: current, runtime, operation: { path, type: 'service-set', name: 'fuel', endpoint: { kind: 'capacity', id: 'missing' } } })).rejects.toThrow();
    expect(current.read(path)).toBe(after); expect(current.snapshot().history.undo).toHaveLength(1);
  });
  it('rejects fractional, negative and overfull stocks', () => {
    for (const [amount, ceiling] of [[-1, 3], [0.5, 3], [4, 3]]) expect(() => operation(draft(), { type: 'capacity-set', id: 'ammo', amount, ceiling })).toThrow();
  });
});
