import { expect, it, vi } from 'vitest';
import fs from 'node:fs';
import { parse, stringify } from 'smol-toml';
import { resolveTemplate } from '../entity-includes.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectWeapons, applyWeaponOperation } from '../workshop-weapons.js';

it('authors the real finite PDC battery through the accepted weapon/history flow', async () => {
  const read = file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null;
  const actual = resolveTemplate('assets/entities/pdc_cruiser.toml', { read }, parse).resolved.value;
  const parent = resolveTemplate('assets/entities/broadside_cruiser.toml', { read }, parse).resolved.value;
  const weapon = actual.weapon.find(row => row.id === 'blaster-side');
  expect(weapon.targets).toEqual(['missile']);
  expect(weapon.mount.fire_arc_deg).toBe(360);
  expect(actual.weapons_console.radar.selects).toContain('missile');
  expect(actual.system.find(row => row.id === 'blaster-side').component || []).toEqual([]);
  expect(actual.energy).toEqual(parent.energy);
  expect(actual.infrastructure).toEqual(parent.infrastructure);
  expect(actual.logistics).toEqual(parent.logistics);
  expect(actual.station).toEqual(parent.station);
  expect(actual.infrastructure.capacity.filter(row => row.id.endsWith('kinetic-rounds')).reduce((total, row) => total + row.amount, 0)).toBe(22);
  const path = 'assets/entities/pdc.toml', before = stringify(actual);
  const draft = new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="pdc"\nname="PDC"\nversion="1"\n' },
    { path, text: before },
  ]));
  const dependencies = { base_files: {}, packs: [] };
  const value = inspectWeapons(draft, dependencies, path).weapons.find(row => row.weapon.id === 'blaster-side').weapon;
  value.targets = [];
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  await applyWeaponOperation({ draft, runtime, operation: { path, section: 'weapon', originalId: 'blaster-side', weapon: value } });
  const readBack = current => inspectWeapons(current, dependencies, path).weapons.find(row => row.weapon.id === 'blaster-side').weapon;
  expect(readBack(new WorkshopDocument(draft.archive())).targets).toEqual([]);
  expect(readBack(draft).delivery.rounds).toEqual({ store: 'local-kinetic-rounds', amount: 1 });
  draft.undo(); expect(draft.read(path)).toBe(before); draft.redo();
  expect(readBack(draft).targets).toEqual([]);
  expect(runtime.validate).toHaveBeenCalledOnce();
});
