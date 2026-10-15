import {expect,it} from 'vitest';
import fs from 'node:fs';
import {parse} from 'smol-toml';
import {resolveTemplate} from '../entity-includes.js';
import {initialProtection,validateProtectionComposition,writeSystemComposition,initialJamming,validateJammingComposition} from '../workshop-system-composition.js';
import {validateChannelInitials} from '../workshop-protection.js';
import {systemTreeVocabulary} from '../../gui/workshop-state-trees.js';
const owner={id:'guard',kind:'signal-protection',power_group:'science'};
const draft=()=>{const value=initialProtection();value.components[1].config.systems={radio:2,sensors:1};value.channels[1].initial.value=['radio','sensors'];return value;};
it('keeps typed initial values optional, closed and bounded at the real creation frontier',()=>{
 validateChannelInitials([{name:'enabled',kind:'boolean'},{name:'target',kind:'entity_target'}]);
 for(const initial of [{kind:'boolean',value:'false'},{kind:'system_priority',value:['same','same']},{kind:'entity_target',value:'x'.repeat(129)},{kind:'boolean',value:true,hidden:1}]) {
  expect(()=>validateChannelInitials([{name:'intent',kind:initial.kind,initial}])).toThrow();
 }
 expect(()=>validateChannelInitials([{name:'intent',kind:'entity_target',initial:{kind:'entity_target',value:'x'.repeat(128)}}])).not.toThrow();
});
it('roundtrips exact ordered costs and initial intent without rewriting sibling source',()=>{
 const value=draft();validateProtectionComposition(owner,value.components,value.channels);
 const source='# retain\n[[system]]\nid="guard"\nkind="signal-protection"\npower_group="science"\n[[system]] # untouched\nid="other"\nkind="sensors"\n';
 const written=writeSystemComposition(source,owner.id,value,owner),actual=parse(written).system[0];
 expect(actual.component).toEqual(value.components);expect(actual.channel).toEqual(value.channels);expect(written).toContain('[[system]] # untouched\nid="other"');
 for(const change of [v=>v.components[1].config.capacity=0,v=>v.components[1].config.strength=1e-100,v=>v.components[1].config.systems.radio=4,v=>v.channels[1].initial.value=['foreign'],v=>v.channels[1].kind='entity_target',v=>v.components[1].power_input='absent',v=>v.components[1].config.systems=['radio']]){
  const bad=draft();change(bad);expect(()=>validateProtectionComposition(owner,bad.components,bad.channels)).toThrow();
 }
});
it('requires actual compatible own equipment and exposes only the enabled tree output',()=>{
 const hull=resolveTemplate('assets/entities/protection_cruiser.toml',{read:path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null},parse).resolved.value;
 const system=hull.system.find(row=>row.kind==='signal-protection');validateProtectionComposition(system,system.component,system.channel,hull);
 const bad=structuredClone(hull);delete bad.comms.radio;expect(()=>validateProtectionComposition(system,system.component,system.channel,bad)).toThrow();
 const vocabulary=systemTreeVocabulary({...system,component:system.component.map(row=>row.kind==='signal_protection'?{...row,enabled_channel:'active'}:row)},{hosts:{'signal-protection':{facts:['enabled','has_priorities'],channels:['enabled']}},channels:[]});
 expect(vocabulary.channels).toEqual(['active']);expect(vocabulary.facts).toEqual(['enabled','has_priorities']);
});
it('authors the closed Sensors jammer effect without changing legacy default semantics',()=>{
 const value=initialJamming(),owner={kind:'jamming',power_group:'radio'};
 value.components[1].config.effect='sensors';validateJammingComposition(owner,value.components,value.channels);
 value.components[1].config.effect='blind';expect(()=>validateJammingComposition(owner,value.components,value.channels)).toThrow();
 delete value.components[1].config.effect;validateJammingComposition(owner,value.components,value.channels);
});
