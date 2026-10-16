import fs from 'node:fs';
import {expect,it,vi} from 'vitest';
import {parse} from 'smol-toml';
import {resolveTemplate} from '../entity-includes.js';
import {WorkshopDocument} from '../workshop-document.js';
import {createStoreZip} from '../mod-pack-export.js';
import {applyWeaponOperation,inspectWeapons,prepareWeaponOperation} from '../workshop-weapons.js';

it('edits the actual Comms EMP bank through existing weapon history without changing its owners or services',async()=>{
  const path='assets/entities/disruptor_cruiser.toml',before=fs.readFileSync(path,'utf8');
  const read=file=>fs.existsSync(file)?fs.readFileSync(file,'utf8'):null;
  const original=resolveTemplate(path,{read},parse).resolved.value;
  const base_files=Object.fromEntries(fs.readdirSync('assets/entities',{recursive:true}).filter(file=>file.endsWith('.toml')).map(file=>{const path=`assets/entities/${file.replaceAll('\\','/')}`;return[path,read(path)];}));
  const dependencies={base_files,packs:[]};
  const draft=new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="disruptor-test"\nname="Disruptor test"\nversion="1"\n'},{path,text:before}]));
  const selected=inspectWeapons(draft,dependencies,path).weapons.find(row=>row.weapon.id==='blaster-disruptor');
  expect(selected.section).toBe('weapon');expect(selected.weapon.impact.emp).toEqual({duration_secs:3});
  selected.weapon.impact.emp.duration_secs=4;selected.weapon.mount.range=42;
  // This owner test proves source/history wiring. The real accepted loader is
  // exercised separately by the native configuration and full-host tests.
  const runtime={dependencies:async()=>dependencies,validate:vi.fn(async()=>({accepted:true,findings:[]}))};
  const operation={path,section:selected.section,originalId:selected.weapon.id,weapon:selected.weapon};
  await applyWeaponOperation({draft,runtime,operation});
  const after=draft.read(path),exported=new WorkshopDocument(draft.archive());
  const value=resolveTemplate(path,{...base_files,[path]:exported.read(path)},parse).resolved.value;
  expect(value.weapon.find(row=>row.id==='blaster-disruptor')).toMatchObject({impact:{emp:{duration_secs:4}},mount:{range:42}});
  for(const key of ['energy','interior','station','console','system'])expect(value[key]).toEqual(original[key]);
  const unchanged=original.weapon.filter(row=>row.id!=='blaster-disruptor');
  expect(value.weapon.filter(row=>row.id!=='blaster-disruptor')).toEqual(unchanged);
  draft.undo();expect(draft.read(path)).toBe(before);draft.redo();expect(draft.read(path)).toBe(after);
  const invalid=structuredClone(operation);invalid.weapon.impact.emp.duration_secs=0;
  expect(()=>prepareWeaponOperation(draft,dependencies,invalid)).toThrow(/EMP|duration/i);
  expect(draft.read(path)).toBe(after);
  runtime.validate.mockResolvedValueOnce({accepted:false,findings:[{severity:'error',message:'Actual loader refused this equipment.'}]});
  await expect(applyWeaponOperation({draft,runtime,operation})).rejects.toThrow();
  expect(draft.read(path)).toBe(after);expect(draft.snapshot().history.undo).toHaveLength(1);
});
