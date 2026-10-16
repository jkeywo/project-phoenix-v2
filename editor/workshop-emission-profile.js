import {parse} from 'smol-toml';
import {inspectEntityComposition} from './workshop-entity-composition.js';
import {authoredAndGeneratedSystems} from './workshop-generated-systems.js';
import {acceptWorkshopChanges} from './workshop-acceptance.js';
import {textMembers,tomlBasicString} from './workshop-definitions.js';

export function validateEmissionProfiles(entity) {
  let selected=null;
  for(const system of authoredAndGeneratedSystems(entity)) {
    const config=system.config?.emission_profile;if(config===undefined)continue;
    if(selected||system.kind!=='command'||!entity.console?.some(console=>console.id===system.console))throw new Error('An emission profile needs one actual Console-owned Command System.');
    if(!config||typeof config!=='object'||Array.isArray(config)||Object.keys(config).some(key=>!['initial','advisory_rate'].includes(key))
      ||(config.initial!==undefined&&!['normal','low_emission'].includes(config.initial))||typeof config.advisory_rate!=='number'
      ||!Number.isFinite(Math.fround(config.advisory_rate))||Math.fround(config.advisory_rate)<=0)throw new Error('Use a supported initial emission intent and a positive finite advisory rate.');
    selected={owner:system.id,config:structuredClone(config)};
  }
  return selected;
}
export function inspectEmissionProfile(draft,dependencies,path) {
  const inspection=inspectEntityComposition(draft,dependencies,path),entity=inspection.resolved.value;
  const selected=validateEmissionProfiles(entity);
  const owners=authoredAndGeneratedSystems(entity).filter(system=>system.kind==='command'&&entity.console?.some(console=>console.id===system.console)).map(system=>system.id);
  return {...inspection,owners,selected};
}
export function guardEmissionProfileReferences(_before,after){validateEmissionProfiles(after);}

/** Materialise only the selected profile leaves; inherited machinery and AI
 * remain inherited. Actual native TOML edits preserve unrelated source. */
export async function applyEmissionProfileOperation({draft,provider,runtime,dependencies,operation,current=()=>true}) {
  const selected=structuredClone(operation);
  return acceptWorkshopChanges({draft,provider,runtime,dependencies,current,
    prepare:async(captured,effective)=>{
      const inspection=inspectEmissionProfile(captured,effective,selected.path);
      if(!inspection.owners.includes(selected.owner))throw new Error('Choose an actual Command owner.');
      if(inspection.selected&&inspection.selected.owner!==selected.owner)throw new Error('Edit the existing profile owner instead of creating a second profile.');
      const entity=structuredClone(inspection.resolved.value),system=entity.system.find(row=>row.id===selected.owner);
      system.config={...system.config,emission_profile:selected.value};
      if(validateEmissionProfiles(entity)?.owner!==selected.owner)throw new Error('Configure a supported profile instead of erasing inherited capability.');
      const before=captured.read(selected.path),local=parse(before),rows=local.system??[];
      let index=rows.findIndex(row=>row.id===selected.owner),row=index>=0?rows[index]:null;
      if(!row?.config?.emission_profile&&!selected.materialise)throw new Error('Explicitly materialise the selected inherited or new profile before editing.');
      const edits=[];
      if(index<0){index=rows.length;edits.push({op:'append_table',path:['system'],fields:[['id',tomlBasicString(selected.owner)]]});}
      const path=['system',index,'config'];
      if(!row?.config)edits.push({op:'put',path,value_source:'{}'});
      if(!row?.config?.emission_profile)edits.push({op:'put',path:[...path,'emission_profile'],value_source:'{}'});
      for(const[key,value]of Object.entries({initial:selected.value.initial??'normal',advisory_rate:selected.value.advisory_rate}))
        edits.push({op:'put',path:[...path,'emission_profile',key],value_source:typeof value==='string'?tomlBasicString(value):String(value)});
      const after=await runtime.compose(textMembers(captured),{document_path:selected.path,expected_source:before,edits});
      return before===after?[]:[{path:selected.path,before,after}];
    },inspect:(candidate,effective)=>inspectEmissionProfile(candidate,effective,selected.path)});
}
