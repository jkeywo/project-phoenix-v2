import {parse} from 'smol-toml';
import {inspectWeapons} from './workshop-weapons.js';
import {acceptWorkshopChanges} from './workshop-acceptance.js';
import {textMembers,tomlBasicString} from './workshop-definitions.js';
import {validateLoaderGroups} from './workshop-loader-model.js';
const groupsSource=groups=>'['+groups.map(group=>`{ id = ${tomlBasicString(group.id)}, work_rate = ${group.work_rate}, tubes = [${group.tubes.map(tomlBasicString).join(', ')}] }`).join(', ')+']';
export async function applySharedLoaders({draft,provider,runtime,dependencies,operation,current=()=>true}){
  const selected=structuredClone(operation);
  return acceptWorkshopChanges({draft,provider,runtime,dependencies,current,prepare:async(captured,effective)=>{
    const inspection=inspectWeapons(captured,effective,selected.path),entity=inspection.resolved.value;
    if(!entity.torpedoes)throw new Error('Select actual guided equipment.');
    validateLoaderGroups(entity,selected.groups);
    const before=captured.read(selected.path),local=parse(before);
    if(!Object.hasOwn(local.torpedoes??{},'loader_groups')&&!selected.materialise)throw new Error('Explicitly materialise inherited or new loader groups before editing.');
    const edits=[];
    if(!local.torpedoes)edits.push({op:'put',path:['torpedoes'],value_source:'{}'});
    edits.push({op:'put',path:['torpedoes','loader_groups'],value_source:groupsSource(selected.groups)});
    const after=await runtime.compose(textMembers(captured),{document_path:selected.path,expected_source:before,edits});
    return after===before?[]:[{path:selected.path,before,after}];
  },inspect:(candidate,effective)=>{const inspection=inspectWeapons(candidate,effective,selected.path);validateLoaderGroups(inspection.resolved.value);return inspection;}});
}
