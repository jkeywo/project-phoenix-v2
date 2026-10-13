import { describe, it, expect } from 'vitest';
import { parse } from 'smol-toml';
import { writeSystemComposition } from '../workshop-system-composition.js';
import { mergeComposeFragments } from '../entity-includes.js';

const source = `# untouched header
[[system]] # ownership preserved
id = "dock"
kind = "dock"
console = "helm" # retained
power_group = "helm"
[[system.component]]
id = "mate"
kind = "dock_mating"
[system.component.config]
range = 10
[[system.component]]
id = "power"
kind = "power_input"
[[system.channel]]
name = "enabled"
kind = "boolean"
[[system.channel]]
name = "target"
kind = "entity_target"
[[system]] # sibling
id = "other"
kind = "helm_thrust"
`;

describe('System component source transaction', () => {
  it('reorders and edits nested components only inside their containing System', () => {
    const effective = parse(source).system[0];
    const components = structuredClone(effective.component).reverse(); components[1].config.range = 25;
    const after = writeSystemComposition(source, 'dock', { components, channels: effective.channel }, effective);
    expect(after).toContain('# untouched header');
    expect(after).toContain('console = "helm" # retained');
    expect(after.slice(after.indexOf('[[system]] # sibling'))).toBe(source.slice(source.indexOf('[[system]] # sibling')));
    const result = parse(after).system[0];
    expect(result.component.map(row => row.id)).toEqual(['power', 'mate']);
    expect(result.component[1].config.range).toBe(25);
    expect(result.channel).toEqual(effective.channel);
  });
  it('refuses implicit inherited edits and uses tombstones for explicit removal', () => {
    const local = '[[system]]\nid="dock"\nconsole="helm"\n';
    const effective = parse(source).system[0];
    const operation = { components: [effective.component[1]], channels: effective.channel };
    expect(() => writeSystemComposition(local, 'dock', operation, effective)).toThrow(/Materialise inherited/);
    const after = writeSystemComposition(local, 'dock', { ...operation, materialise: true }, effective);
    const merged = mergeComposeFragments({ system: [effective] }, parse(after));
    expect(merged.system[0].component).toEqual([effective.component[1]]);
    expect(parse(after).system[0].component).toContainEqual({ id: 'mate', _remove: true });
  });
  it('cannot edit a System supplied wholly by an include', () => {
    expect(() => writeSystemComposition('includes=["base.toml"]\n', 'dock', {}, parse(source).system[0])).toThrow(/containing System/);
  });
});
