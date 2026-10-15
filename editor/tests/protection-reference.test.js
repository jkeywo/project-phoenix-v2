import {expect,it} from 'vitest';
import fs from 'node:fs';
import {parse} from 'smol-toml';
import {resolveTemplate} from '../entity-includes.js';
const resolve=path=>resolveTemplate(path,{read:path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null},parse).resolved.value;
it('places the real protection owner explicitly in every inherited Science rating without simplifying unsupported gear',()=>{
 const base=resolve('assets/entities/alliance_cruiser.toml'),value=resolve('assets/entities/protection_cruiser.toml');
 const before=base.station.find(row=>row.id==='science'),after=value.station.find(row=>row.id==='science');
 expect(after.rating.map(row=>row.name)).toEqual(before.rating.map(row=>row.name));
 for(const rating of after.rating){
  const original=before.rating.find(row=>row.name===rating.name);
  expect([...rating.automated_systems,...rating.detailed_systems].filter(id=>id==='signal-protection')).toHaveLength(1);
  expect({...rating,automated_systems:rating.automated_systems.filter(id=>id!=='signal-protection'),detailed_systems:rating.detailed_systems.filter(id=>id!=='signal-protection')}).toEqual(original);
 }
 expect(after.rating.find(row=>row.name==='Guard').automated_systems).toContain('signal-protection');
 expect(after.rating.find(row=>row.name==='Std').detailed_systems).toContain('signal-protection');
});
