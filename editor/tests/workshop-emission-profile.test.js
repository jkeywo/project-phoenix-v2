import {expect,it,vi} from 'vitest';
import {parse,stringify} from 'smol-toml';
import {WorkshopDocument} from '../workshop-document.js';
import {createStoreZip} from '../mod-pack-export.js';
import {applyEmissionProfileOperation,inspectEmissionProfile,validateEmissionProfiles} from '../workshop-emission-profile.js';
const path='assets/entities/emission.toml',deps={base_files:{},packs:[]};
const entity=()=>({name:'Reference',console:[{id:'orders'}],system:[{id:'orders-core',kind:'command',console:'orders',config:{other_supported_data:'preserved'}},{id:'drive',kind:'helm-thrust',console:'orders'}]});
const value=()=>({initial:'low_emission',advisory_rate:0.25});
const make=source=>new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="emission"\nname="Emission"\nversion="1"\n'},{path,text:source}]));
// Model exactly the native structural packet for transaction tests. Real native
// source/comment preservation is a separate integration-owned proof.
function runtime(){return {validate:vi.fn(async()=>({accepted:true})),compose:vi.fn(async(files,request)=>{
  expect(request.expected_source).toBe(files[path]);const source=parse(files[path]);
  for(const edit of request.edits){
    if(edit.op==='append_table'){expect(edit.path).toEqual(['system']);(source.system??=[]).push(Object.fromEntries(edit.fields.map(([key,value])=>[key,parse('v='+value).v])));}
    else {expect(edit.op).toBe('put');const parent=edit.path.slice(0,-1).reduce((row,key)=>row[key],source);parent[edit.path.at(-1)]=parse('v='+edit.value_source).v;}
  }return stringify(source);
})};}
it('uses the actual native structural editor and one accepted history entry',async()=>{
  const source=stringify(entity()),draft=make(source),engine=runtime();
  await applyEmissionProfileOperation({draft,runtime:engine,dependencies:deps,operation:{path,owner:'orders-core',value:value(),materialise:true}});
  expect(engine.compose.mock.calls[0][1].edits).toEqual([
    {op:'put',path:['system',0,'config','emission_profile'],value_source:'{}'},
    {op:'put',path:['system',0,'config','emission_profile','initial'],value_source:'"low_emission"'},
    {op:'put',path:['system',0,'config','emission_profile','advisory_rate'],value_source:'0.25'},
  ]);
  expect(inspectEmissionProfile(draft,deps,path).selected).toEqual({owner:'orders-core',config:value()});
  expect(parse(draft.read(path)).system[0].config.other_supported_data).toBe('preserved');
  expect(parse(draft.read(path)).system[1]).toEqual(entity().system[1]);
  draft.undo();expect(draft.read(path)).toBe(source);draft.redo();
  expect(inspectEmissionProfile(new WorkshopDocument(draft.archive()),deps,path).selected.config).toEqual(value());
});
it('materialises only new inherited profile leaves and preserves existing inherited machinery',async()=>{
  const source='includes=["base.toml"]\n# own source\n',draft=make(source),dependencies={base_files:{'assets/entities/base.toml':stringify(entity())},packs:[]},engine=runtime();
  await expect(applyEmissionProfileOperation({draft,runtime:engine,dependencies,operation:{path,owner:'orders-core',value:value()}})).rejects.toThrow(/materialise/);
  expect(engine.compose).not.toHaveBeenCalled();expect(draft.read(path)).toBe(source);
  await applyEmissionProfileOperation({draft,runtime:engine,dependencies,operation:{path,owner:'orders-core',value:value(),materialise:true}});
  expect(engine.compose.mock.calls[0][1].edits.slice(0,2)).toEqual([
    {op:'append_table',path:['system'],fields:[['id','"orders-core"']]},
    {op:'put',path:['system',0,'config'],value_source:'{}'},
  ]);
  const selected=inspectEmissionProfile(draft,dependencies,path);
  expect(selected.resolved.value.system[1]).toEqual(entity().system[1]);
  expect(selected.resolved.value.system[0].config.other_supported_data).toBe('preserved');
  draft.undo();expect(draft.read(path)).toBe(source);
});
it('rejects unsupported ownership, open profiles and stale/refused candidates without source writes',async()=>{
  for(const change of [row=>row.system[0].kind='comms',row=>row.system[0].console='missing',row=>row.system.push({...structuredClone(row.system[0]),id:'other-command'})]){
    const row=entity();row.system[0].config.emission_profile=value();change(row);expect(()=>validateEmissionProfiles(row)).toThrow();
  }
  for(const config of [{...value(),limit:1},{...value(),advisory_rate:0},{...value(),initial:'undetectable'}]){
    const row=entity();row.system[0].config.emission_profile=config;expect(()=>validateEmissionProfiles(row)).toThrow();
  }
  const draft=make(stringify(entity())),before=draft.read(path),engine=runtime();engine.validate=async()=>({accepted:false});
  await expect(applyEmissionProfileOperation({draft,runtime:engine,dependencies:deps,operation:{path,owner:'orders-core',value:value(),materialise:true}})).rejects.toThrow();
  expect(draft.read(path)).toBe(before);
  const delayed=runtime();delayed.compose=async(...args)=>{const result=await engine.compose(...args);draft.edit(path,before+'\n# later edit\n');return result;};
  await expect(applyEmissionProfileOperation({draft,runtime:delayed,dependencies:deps,operation:{path,owner:'orders-core',value:value(),materialise:true}})).rejects.toThrow(/stale/);
  expect(draft.read(path)).toContain('# later edit');
});
