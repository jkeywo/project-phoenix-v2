import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { resolveTemplate } from '../entity-includes.js';
import { prepareShipOperation } from '../workshop-ship-authoring.js';
import { inspectWeaponGroups, validateWeaponGroups, prepareWeaponGroupsOperation, applyWeaponGroupsOperation, guardWeaponGroupReferences } from '../workshop-weapon-groups.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const source = '# retained\n[[system]]\nid="beam"\nkind="phaser_bank"\n[weapons_console]\nmode="Manual" # keep\n';
const group = { id: 'pair', label: 'weapon.group.main_beams', members: ['beam'] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
const edit = (draft, value, dependencies = deps, materialise = false) => prepareWeaponGroupsOperation(draft, dependencies, { path, value, materialise }).changes[0].after;
describe('prepared weapon group accepted source', () => {
  it('validates prepared groups against each actual resolved Cruiser without changing its equipment', () => {
    for (const hull of ['alliance_cruiser', 'dynasty_player_cruiser']) {
      const entity = resolveTemplate(`assets/entities/${hull}.toml`, { read: name => fs.existsSync(name) ? fs.readFileSync(name, 'utf8') : null }, parse).resolved.value;
      expect(entity.weapons_console.group).toHaveLength(2);
      expect(() => validateWeaponGroups(entity.weapons_console.group, entity)).not.toThrow();
      expect(entity.weapons_console.group.flatMap(row => row.members)).toEqual(['phaser-fore', 'phaser-aft', 'torpedo-tube-fore-port', 'torpedo-tube-fore-starboard', 'torpedo-tube-aft']);
    }
  });
  it('replaces groups atomically while preserving other Weapons settings and exact outside source', () => {
    const after = edit(make(source), [group]); expect(after).toContain(source); expect(parse(after).weapons_console.group).toEqual([group]);
    const cleared = edit(make(after), []); expect(parse(cleared).weapons_console).toEqual({ mode: 'Manual', group: [] }); expect(cleared).toContain('# retained');
    expect(parse(edit(make(cleared), [group])).weapons_console.group).toEqual([group]);
  });
  it.each([[{ ...group, members: [] }], [{ ...group, members: ['beam', 'beam'] }], [{ ...group, members: ['unknown'] }], [group, group], [{ ...group, extra: true }], [{ ...group, id: 'NotValid' }]])('refuses closed-schema/member failures without edits', value => {
    const draft = make(source); expect(() => edit(draft, value)).toThrow(); expect(draft.read(path)).toBe(source);
  });
  it('requires explicit inherited materialisation and an empty override really clears inherited groups', () => {
    const inherited = edit(make(source), [group]), child = 'includes=["base.toml"]\n# child\n', dependencies = { base_files: { [base]: inherited }, packs: [] };
    const draft = make(child); expect(inspectWeaponGroups(draft, dependencies, path).inherited).toBe(true); expect(() => edit(draft, [], dependencies)).toThrow(/materialise/);
    const cleared = edit(draft, [], dependencies, true);
    expect(resolveTemplate(path, { [path]: cleared, [base]: inherited }, parse).resolved.value.weapons_console.group).toEqual([]);
  });
  it('refuses losing a group member through another authoring surface', () => {
    const entity = parse(source); entity.weapons_console.group = [group];
    expect(() => guardWeaponGroupReferences(entity, { ...entity, system: [] })).toThrow(/actual weapon/);
    expect(() => validateWeaponGroups([group], { system: [{ id: 'beam', kind: 'sensor_radar' }] })).toThrow();
    const text = 'tags=["ship"]\n' + edit(make(source), [group]), draft = make(text);
    expect(() => prepareShipOperation(draft, deps, { system_kinds: ['phaser_bank'] }, { path, type: 'system-remove', id: 'beam' })).toThrow(/actual weapon/);
    expect(draft.read(path)).toBe(text);
  });
  it('accepts through ordinary validation/history, then undo and redo preserve exact source', async () => {
    const draft = make(source), runtime = { validate: async () => ({ accepted: true }) };
    await applyWeaponGroupsOperation({ draft, runtime, dependencies: deps, operation: { path, value: [group] } });
    expect(parse(draft.read(path)).weapons_console.group).toEqual([group]); draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(parse(draft.read(path)).weapons_console.group).toEqual([group]);
  });
});
