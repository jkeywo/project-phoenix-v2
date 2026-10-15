import fs from 'node:fs';
import {expect,it} from 'vitest';
import {parse} from 'smol-toml';
import {resolveTemplate} from '../entity-includes.js';

it('resolves the optional actual drive onto finite existing Power/Stock owners without changing default hulls',()=>{
  const read=path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null;
  const resolve=path=>{const result=resolveTemplate(path,{read},parse);expect(result.ok).toBe(true);return result.resolved.value;};
  const base=resolve('assets/entities/alliance_cruiser.toml'),reference=resolve('assets/entities/local_jump_cruiser.toml');
  const drive=reference.system.find(row=>row.id==='local-jump'),binding=drive.component.find(row=>row.kind==='transit_drive');
  expect(drive.kind).toBe('transit-drive');expect(drive.console).toBe('helm');
  expect(binding.navigation_source).toBe('navigation');expect(drive.component.map(row=>row.kind)).toEqual(['power_input','transit_drive']);
  expect(binding.config.fuel).toEqual({store:'local-transit-fuel',amount:2});
  expect(reference.energy.reactor).toEqual(base.energy.reactor);
  expect(reference.energy.consumer.filter(row=>row.system!=='local-jump')).toEqual(base.energy.consumer);
  expect(reference.system.filter(row=>row.id!=='local-jump')).toEqual(base.system);
  const store=reference.logistics.store.find(row=>row.id===binding.config.fuel.store),main=reference.logistics.store.find(row=>row.id===store.source);
  expect(store).toMatchObject({role:'local',system:'local-jump',compartment:'engineering',resource:'transit_fuel'});
  expect(main).toMatchObject({role:'main',resource:'transit_fuel'});
  expect(store.endpoint.kind).toBe('capacity');
  const capacity=reference.infrastructure.capacity.find(row=>row.id===store.endpoint.id),hold=reference.infrastructure.capacity.find(row=>row.id===main.endpoint.id);
  expect([capacity.amount,capacity.ceiling,hold.amount,hold.ceiling]).toEqual([6,12,18,36]);
  expect(reference.interior.placements['local-jump']).toBe('engineering');
  expect(reference.logistics.handling).toEqual(base.logistics.handling);
  expect(base.system.some(row=>row.id==='local-jump')).toBe(false);
  for (const owner of [drive.id, reference.logistics.handling[0].system]) {
    const consoleId=reference.system.find(row=>row.id===owner).console;
    const console=reference.console.find(row=>row.id===consoleId),original=base.console.find(row=>row.id===consoleId);
    expect(console.feed).toContainEqual({source:'stock/local-jump',kind:'Stock'});
    expect(console.feed.filter(row=>row.source!=='stock/local-jump')).toEqual(original.feed);
    expect({...console,feed:original.feed}).toEqual(original);
  }
});
