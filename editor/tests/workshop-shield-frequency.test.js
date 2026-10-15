import {describe,it,expect} from 'vitest';
import fs from 'node:fs';
import {parse} from 'smol-toml';
import {WorkshopDocument} from '../workshop-document.js';
import {createStoreZip} from '../mod-pack-export.js';
import {resolveTemplate} from '../entity-includes.js';
import {prepareShieldFrequencyOperation,applyShieldFrequencyOperation,validateShieldFrequency} from '../workshop-shields.js';
import {validateEnquiryObservation,validateSensing} from '../workshop-sensing.js';
const path='assets/entities/test.toml',base='assets/entities/base.toml',deps={base_files:{},packs:[]};
const source='# retained\n[[shield_arc]]\nid="fore"\n[shields_console]\nfrequency=0.5\n# policy\n[shields_console.base]\nmax_hp=100\n';
const policy={response:{mismatch_loss:.1},cycle:{frequencies:[.2,.8],period_secs:30}};
const make=text=>new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n'},{path,text}]));
describe('shield enquiry equipment accepted source',()=>{
 it('edits response/cycle while preserving actual other settings and accepted history',async()=>{
  const draft=make(source);await applyShieldFrequencyOperation({draft,dependencies:deps,runtime:{validate:async()=>({accepted:true})},operation:{path,value:policy}});
  expect(parse(draft.read(path)).shields_console).toEqual({frequency:.5,base:{max_hp:100},frequency_response:policy.response,frequency_cycle:policy.cycle});
  expect(draft.read(path)).toContain('# retained');draft.undo();expect(draft.read(path)).toBe(source);draft.redo();
  const cleared=prepareShieldFrequencyOperation(draft,deps,{path,value:{...policy,cycle:false}}).changes[0].after;
  expect(parse(cleared).shields_console.frequency_cycle).toBe(false);
 });
 it('explicitly materialises inherited policy and disables inherited cycling',()=>{
  const inherited=prepareShieldFrequencyOperation(make(source),deps,{path,value:policy}).changes[0].after;
  const draft=make('includes=["base.toml"]\n'),dependencies={base_files:{[base]:inherited},packs:[]};
  expect(()=>prepareShieldFrequencyOperation(draft,dependencies,{path,value:{...policy,cycle:false}})).toThrow(/materialise/);
  const after=prepareShieldFrequencyOperation(draft,dependencies,{path,materialise:true,value:{...policy,cycle:false}}).changes[0].after;
  expect(resolveTemplate(path,{[path]:after,[base]:inherited},parse).resolved.value.shields_console.frequency_cycle).toBe(false);
 });
 it.each([-.1,1.1,NaN,Infinity])('refuses invalid response %s without modifying source',loss=>{
  const draft=make(source);expect(()=>prepareShieldFrequencyOperation(draft,deps,{path,value:{...policy,response:{mismatch_loss:loss}}})).toThrow();expect(draft.read(path)).toBe(source);
 });
 it('requires finite real equipment and explicit subject permission',()=>{
  expect(()=>validateShieldFrequency(policy,{})).toThrow();
  expect(()=>validateShieldFrequency({...policy,cycle:{frequencies:[.2,.2],period_secs:1}},parse(source))).toThrow();
  expect(()=>validateEnquiryObservation({property:[{property:{kind:'shield_frequency'},normal:true,step:.01}]},parse(source))).not.toThrow();
  expect(()=>validateEnquiryObservation({property:[{property:{kind:'shield_frequency'},normal:true,step:.01}]},{})).toThrow();
 });
 it('resolves explicit ordinary and rotating special policies without changing other fleet equipment',()=>{
  const resolve=name=>resolveTemplate('assets/entities/'+name+'.toml',{read:file=>fs.existsSync(file)?fs.readFileSync(file,'utf8'):null},parse).resolved.value;
  const ordinary=resolve('alliance_cruiser'),ordinaryFoe=resolve('ship_harrow_standard_shields_cruiser'),special=resolve('ship_harrow_resonant_cruiser'),original=resolve('ship_harrow_cruiser');
  expect(ordinary.shields_console.frequency_response.mismatch_loss).toBe(.1);
  expect(()=>validateSensing(ordinary.sensing,ordinary)).not.toThrow();
  expect(ordinary.sensing.focused[0].default_aim || 'general').toBe('general');
  expect(ordinary.sensing.focused[0].aims.some(row=>row.aim==='shields')).toBe(true);
  expect(special.shields_console.frequency_response.mismatch_loss).toBe(.6);expect(special.shields_console.frequency_cycle.frequencies).toEqual([.2,.8]);
  expect(ordinaryFoe.shields_console.frequency_response.mismatch_loss).toBe(.1);expect(ordinaryFoe.shield_arc).toEqual(original.shield_arc);expect(original.shields_console.frequency_cycle).toBeUndefined();expect(special.shield_arc).toEqual(original.shield_arc);
 });
});
