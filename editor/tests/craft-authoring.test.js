import fs from 'node:fs';
import {expect,it} from 'vitest';
import {parse} from 'smol-toml';
import {COMPONENT_SCHEMA} from '../component-schema.js';
import {validatePersonnel} from '../workshop-personnel.js';
import {resolveTemplate} from '../entity-includes.js';
import {initialBay,validateBayComposition,writeSystemComposition} from '../workshop-system-composition.js';
const read=path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null;
const resolve=path=>{const result=resolveTemplate(path,{read},parse);expect(result.ok).toBe(true);return result.resolved.value;};
it('keeps the actual optional Bay energy, damage, dedicated space and all complexity modes explicit',()=>{
 const base=resolve('assets/entities/alliance_cruiser.toml'),entity=resolve('assets/entities/probe_carrier.toml');
 const bay=entity.system.find(row=>row.kind==='craft-bay');validateBayComposition(bay,bay.component,bay.channel||[]);
 expect(entity.energy.reactor).toEqual(base.energy.reactor);
 expect(entity.energy.consumer.filter(row=>row.system!==bay.id)).toEqual(base.energy.consumer);
 expect(entity.energy.group.filter(row=>row.id!==bay.power_group)).toEqual(base.energy.group);
 expect(entity.energy.group.find(row=>row.id===bay.power_group)).toMatchObject({parent:'support',default_flow_per_sec:0.5});
 expect(entity.system.filter(row=>row.id!==bay.id)).toEqual(base.system);
 expect(entity.interior.placements[bay.id]).toBe(bay.component[1].config.compartment);
 expect(entity.logistics.storage?.some(row=>row.compartment===bay.component[1].config.compartment)).not.toBe(true);
 const station=entity.station.find(row=>row.id===bay.console),previous=base.station.find(row=>row.id===bay.console);
 for(const rating of station.rating){const old=previous.rating.find(row=>row.name===rating.name);expect([...rating.automated_systems,...rating.detailed_systems]).toContain(bay.id);expect({...rating,automated_systems:rating.automated_systems.filter(id=>id!==bay.id),detailed_systems:rating.detailed_systems.filter(id=>id!==bay.id)}).toEqual(old);}
});
it('refuses unsafe component bindings and nonfinite or ambiguous Bay work before source writes',()=>{
 const system={id:'bay',kind:'craft-bay',power_group:'actual-flow'},initial=initialBay();
 expect(()=>validateBayComposition(system,initial.components,[])).not.toThrow();
 for(const change of [rows=>rows[1].power_input='missing',rows=>rows[1].config.capacity_space=0,rows=>rows[1].config.release_offset=[0,Infinity,0],rows=>rows[1].config.launch_work_secs=0,rows=>rows[1].config.compartment='',rows=>rows.push({kind:'power_input',id:'extra'}),rows=>rows[1].invented_phase='ready']){
  const components=structuredClone(initial.components);change(components);expect(()=>validateBayComposition(system,components,[])).toThrow();
 }
 expect(()=>validateBayComposition(system,initial.components,[{name:'unused',kind:'boolean'}])).toThrow();
});
it('writes only the selected actual System composition and retains exact accepted source and includes',()=>{
 const source=read('assets/entities/probe_carrier.toml'),entity=resolve('assets/entities/probe_carrier.toml'),bay=entity.system.find(row=>row.kind==='craft-bay');
 const components=structuredClone(bay.component);components[1].config.launch_work_secs=4;
 const after=writeSystemComposition(source,bay.id,{components,channels:[],materialise:true},bay),parsed=parse(after);
 expect(parsed.includes).toEqual(parse(source).includes);expect(parsed.system[0].component[1].config.launch_work_secs).toBe(4);expect(parsed.energy).toEqual(parse(source).energy);expect(parsed.interior).toEqual(parse(source).interior);expect(parsed.system[0].config.ai).toEqual(parse(source).system[0].config.ai);
});

it('authors optional finite passenger berths and explicit information-only Bay feed without changing probes or seats',()=>{
 const base=resolve('assets/entities/probe_carrier.toml'),carrier=resolve('assets/entities/personnel_carrier.toml');
 const pod=resolve('assets/entities/personnel_pod.toml'),probe=resolve('assets/entities/physical_probe.toml');
 expect(probe.probe.passenger_berths).toBeUndefined();expect(pod.probe).toEqual({space:1,passenger_berths:['passenger']});
 expect(pod.system).toEqual(probe.system);expect(pod.personnel).toBeUndefined();expect(carrier.system).toEqual(base.system);expect(carrier.station).toEqual(base.station);
 expect(carrier.console.find(row=>row.id==='engineering').feed).toContainEqual({source:'probe-bay',kind:'Bay'});
 expect(carrier.system.find(row=>row.id==='probe-bay').console).toBe('tactical');
 expect(COMPONENT_SCHEMA.probe.fields.find(row=>row.key==='passenger_berths')).toMatchObject({type:'array',items:'string'});
 expect(()=>validatePersonnel(carrier.personnel,carrier)).not.toThrow();
 const bad=structuredClone(carrier.personnel);bad.repair_team[0].competencies.push('invented-pilot');expect(()=>validatePersonnel(bad,carrier)).toThrow();
});
