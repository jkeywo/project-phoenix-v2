import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import { parse } from 'smol-toml';
import { writeSystemComposition, initialTurret, validateTurretComposition } from '../workshop-system-composition.js';
import { mergeComposeFragments, resolveTemplate } from '../entity-includes.js';

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

it('keeps the finite turret palette closed and references exactly its actual power input', () => {
  const system = { kind: 'phaser_bank', power_group: 'bank' }, components = initialTurret();
  expect(() => validateTurretComposition(system, components, [])).not.toThrow();
  expect(() => validateTurretComposition(system, [], [])).not.toThrow();
  for (const mutation of [
    rows => rows[1].config.traverse_deg_per_sec = 0,
    rows => rows[1].config.traverse_deg_per_sec = '20',
    rows => rows[1].config.traverse_arc_deg = 361,
    rows => rows[1].config.facing_deg = Infinity,
    rows => rows[1].config.aim_tolerance_deg = 180,
    rows => rows[1].target_channel = 'unread',
    rows => rows[1].power_input = 'missing',
    rows => rows.push(structuredClone(rows[1])),
  ]) { const candidate = structuredClone(components); mutation(candidate); expect(() => validateTurretComposition(system, candidate, [])).toThrow(); }
  expect(() => validateTurretComposition(system, components, [{ name: 'target', kind: 'entity_target' }])).toThrow();
  expect(() => validateTurretComposition({ ...system, power_group: undefined }, components, [])).toThrow();
});

it('resolves the actual Cruiser and inherited Dynasty with one forebank and one passive mount', () => {
  for (const hull of ['alliance_cruiser', 'dynasty_player_cruiser']) {
    const value = resolveTemplate(`assets/entities/${hull}.toml`, { read: path => fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : null }, parse).resolved.value;
    const mounted = value.system.filter(system => system.component?.some(row => row.kind === 'turret'));
    expect(mounted.map(row => row.id).sort()).toEqual(['passive-receiver', 'phaser-fore']);
    for (const system of mounted) expect(() => validateTurretComposition(system, system.component, system.channel || [])).not.toThrow();
    const receiver = value.sensing.passive.find(row => row.system === 'passive-receiver');
    expect(receiver.mount).toEqual({ facing_deg: 0, arc_deg: 120, range: 100 });
    expect(mounted.find(row => row.id === 'phaser-fore').component.find(row => row.kind === 'turret').config.traverse_deg_per_sec).toBe(45);
  }
});
