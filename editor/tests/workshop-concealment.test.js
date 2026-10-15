import {expect,it} from 'vitest';
import fs from 'node:fs';
import {parse} from 'smol-toml';
import {resolveTemplate} from '../entity-includes.js';
import {initialConcealment,validateConcealmentComposition,writeSystemComposition} from '../workshop-system-composition.js';
import {systemTreeVocabulary} from '../../gui/workshop-state-trees.js';
const owner={id:'field',kind:'concealment',power_group:'science'};
it('authors an actual bounded field through existing composition source edits',()=>{
 const value=initialConcealment();validateConcealmentComposition(owner,value.components,value.channels);
 const source='# keep\n[[system]]\nid="field"\nkind="concealment"\npower_group="science"\n[[system]] # sibling\nid="other"\nkind="sensors"\n';
 const written=writeSystemComposition(source,owner.id,value,owner);
 expect(parse(written).system[0].component).toEqual(value.components);expect(written).toContain('[[system]] # sibling\nid="other"');
 for(const change of [v=>v.components[1].power_input='absent',v=>v.components[1].config.max_suppression=1,
  v=>v.components[1].config.max_suppression=0,v=>v.components[1].config.max_suppression=1-1e-12,
  v=>v.components[1].config.emission_scale=0,v=>v.components[1].config.emission_scale=1e-100,
  v=>v.components[1].config.emission_scale=Infinity,v=>v.components[1].config.invisible=true,
  v=>v.channels[0].kind='entity_target',v=>v.channels[0].initial.value='true',v=>v.components.push({kind:'coolant',id:'x'})]) {
  const bad=initialConcealment();change(bad);expect(()=>validateConcealmentComposition(owner,bad.components,bad.channels)).toThrow();
 }
});
it('keeps real finite supply, earned seats, Operation, placement and supported AI',()=>{
 const resolve=path=>resolveTemplate(path,{read:path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null},parse).resolved.value;
 const hull=resolve('assets/entities/concealment_cruiser.toml'),base=resolve('assets/entities/alliance_cruiser.toml');
 const system=hull.system.find(row=>row.kind==='concealment');validateConcealmentComposition(system,system.component,system.channel,hull);
 expect(hull.energy.reactor).toEqual(base.energy.reactor);expect(hull.station.map(row=>row.id)).toEqual(base.station.map(row=>row.id));
 expect(hull.energy.group.filter(row=>row.id!=='energy-concealment')).toEqual(base.energy.group);
 const groups=new Set(hull.energy.group.map(row=>row.id));expect(hull.energy.group.every(row=>!row.parent||groups.has(row.parent))).toBe(true);
 expect(hull.station.find(row=>row.id==='science').rating.find(row=>row.name==='Std').detailed_systems).toContain(system.id);
 expect(system.channel[0].initial.value).toBe(false);expect(system.config.ai.tree[0].state[0].task[0].action.value.value).toBe(true);
 for(const mutate of [v=>delete v.interior.placements[system.id],v=>v.energy.consumer.find(row=>row.system===system.id).continuous_cost_per_sec=0]) {
  const bad=structuredClone(hull);mutate(bad);expect(()=>validateConcealmentComposition(system,system.component,system.channel,bad)).toThrow();
 }
 const vocabulary=systemTreeVocabulary({...system,component:system.component.map(row=>row.kind==='concealment_emitter'?{...row,enabled_channel:'active'}:row)},
  {hosts:{concealment:{facts:['enabled'],channels:['enabled']}},channels:[]});
 expect(vocabulary.channels).toEqual(['active']);expect(vocabulary.facts).toEqual(['enabled']);
});
