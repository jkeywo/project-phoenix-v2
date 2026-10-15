import {expect,it} from 'vitest';
import fs from 'node:fs';
import {parse} from 'smol-toml';
import {resolveTemplate} from '../entity-includes.js';
import {initialIdentity,validateIdentityComposition,writeSystemComposition} from '../workshop-system-composition.js';
import {systemTreeVocabulary} from '../../gui/workshop-state-trees.js';
const owner={id:'transponder',kind:'identity-emitter',power_group:'radio'};
it('authors a closed literal transponder profile and preserves sibling source',()=>{
 const value=initialIdentity();validateIdentityComposition(owner,value.components,value.channels);
 const source='# keep\n[[system]]\nid="transponder"\nkind="identity-emitter"\npower_group="radio"\n[[system]] # sibling\nid="other"\nkind="sensors"\n';
 const written=writeSystemComposition(source,owner.id,value,owner);expect(parse(written).system[0].component).toEqual(value.components);expect(written).toContain('[[system]] # sibling\nid="other"');
 for(const change of [v=>v.components[1].power_input='absent',v=>v.components[1].config.callsign='',v=>v.components[1].config.callsign='x'.repeat(129),v=>v.components[1].config.callsign='a\n',v=>v.components[1].config.faction='secret-actor',v=>v.components[1].config.source_uuid='hidden',v=>v.channels[0].kind='entity_target',v=>v.channels[0].initial.value='true',v=>v.components.push({kind:'coolant',id:'x'})]){
  const bad=initialIdentity();change(bad);expect(()=>validateIdentityComposition(owner,bad.components,bad.channels)).toThrow();
 }
});
it('resolves real finite hardware, placement, costs and inherited ratings without rewriting the reactor',()=>{
 const resolve=path=>resolveTemplate(path,{read:path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null},parse).resolved.value;
 const hull=resolve('assets/entities/identity_cruiser.toml'),base=resolve('assets/entities/alliance_cruiser.toml'),system=hull.system.find(row=>row.kind==='identity-emitter');
 validateIdentityComposition(system,system.component,system.channel,hull);
 expect(hull.energy.reactor).toEqual(base.energy.reactor);
 expect(hull.station.map(row=>row.id)).toEqual(base.station.map(row=>row.id));
 expect(hull.station.find(row=>row.id==='comms').rating.find(row=>row.name==='Std').detailed_systems).toContain(system.id);
 expect(system.channel[0].initial.value).toBe(false);expect(system.config.ai.tree[0].state[0].task[0].action.value.value).toBe(true);
 for(const mutate of [v=>delete v.interior.placements[system.id],v=>v.energy.consumer.find(row=>row.system===system.id).continuous_cost_per_sec=0]){
  const bad=structuredClone(hull);mutate(bad);expect(()=>validateIdentityComposition(system,system.component,system.channel,bad)).toThrow();
 }
 const vocabulary=systemTreeVocabulary({...system,component:system.component.map(row=>row.kind==='identity_emitter'?{...row,enabled_channel:'active'}:row)},{hosts:{'identity-emitter':{facts:['enabled'],channels:['enabled']}},channels:[]});
 expect(vocabulary.channels).toEqual(['active']);expect(vocabulary.facts).toEqual(['enabled']);
});
