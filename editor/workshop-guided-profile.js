import {parse,stringify} from 'smol-toml';
import {inspectWeapons,prepareWeaponOperation} from './workshop-weapons.js';
import {acceptWorkshopChanges} from './workshop-acceptance.js';
export const GUIDED_FLIGHT_DEFAULTS=Object.freeze({speed:15,turn_rate_deg_per_sec:45,lifespan:20,durability:1,detonation_radius:5,damage_hull:50,damage_shields:5,shield_pierce:0,burst_interval_secs:0.3});
export function guidedProfile(value){const raw=value?.torpedoes;if(!raw)return null;return {...GUIDED_FLIGHT_DEFAULTS,...Object.fromEntries(Object.keys(GUIDED_FLIGHT_DEFAULTS).filter(key=>raw[key]!==undefined).map(key=>[key,raw[key]])),seeker:structuredClone(raw.seeker??false),receiver:raw.receiver ? {accept_external_illumination:false,...structuredClone(raw.receiver)} : false};}
export function validateGuidedProfile(profile){
 const finite=value=>typeof value==='number'&&Number.isFinite(Math.fround(value));
 const nonnegative=value=>finite(value)&&value>=0;const positive=value=>finite(value)&&Math.fround(value)>0;
 if(!profile||Object.keys(profile).some(key=>![...Object.keys(GUIDED_FLIGHT_DEFAULTS),'seeker','receiver'].includes(key))
   || !['speed','lifespan','durability','burst_interval_secs'].every(key=>positive(profile[key]))
   || !['turn_rate_deg_per_sec','detonation_radius'].every(key=>nonnegative(profile[key]))
   || !['damage_hull','damage_shields'].every(key=>Number.isInteger(profile[key])&&profile[key]>=0&&profile[key]<=2147483647)
   || !finite(profile.shield_pierce)||profile.shield_pierce<0||profile.shield_pierce>1)
   throw new Error('Use finite supported shared flight, durability, payload and cadence.');
 for(const key of ['seeker','receiver']){
   const equipment=profile[key];if(equipment===false)continue;
   const fields=key==='seeker'?['range','fire_arc_deg','minimum_transmission']:['range','minimum_transmission','maximum_sample_age_secs','accept_external_illumination'];
   if(!equipment||typeof equipment!=='object'||Array.isArray(equipment)||Object.keys(equipment).some(key=>!fields.includes(key))
      || !positive(equipment.range)||!positive(equipment.minimum_transmission)||equipment.minimum_transmission>1
      || key==='seeker'&&(!nonnegative(equipment.fire_arc_deg)||equipment.fire_arc_deg>360)
      || key==='receiver'&&(!positive(equipment.maximum_sample_age_secs)||typeof equipment.accept_external_illumination!=='boolean'))
     throw new Error('Seeker/receiver equipment requires an explicit finite profile or false.');
 }
}
function replace(source,value){
 const heads=[...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)].map(match=>({start:match.index,name:match[1].replace(/\]$/,'').trim()}));
 const spans=heads.filter(row=>row.name==='torpedoes'||row.name.startsWith('torpedoes.')).map(row=>({start:row.start,end:heads.find(next=>next.start>row.start)?.start??source.length}));
 if(!spans.length&&parse(source).torpedoes!==undefined)throw new Error('Expand inline torpedoes into supported TOML tables before editing.');
 let after=source;for(const span of spans.slice().reverse())after=after.slice(0,span.start)+after.slice(span.end);
 const nl=source.includes('\r\n')?'\r\n':'\n',text=stringify({torpedoes:value}).trim().replace(/\n/g,nl)+nl;
 return spans.length?after.slice(0,spans[0].start)+text+after.slice(spans[0].start):after+(after.endsWith(nl)?nl:nl+nl)+text;
}
export function prepareGuidedProfileOperation(draft,dependencies,operation){
 const inspection=inspectWeapons(draft,dependencies,operation.path),before=draft.read(operation.path);
 if(!inspection.resolved.value.torpedoes)throw new Error('Select actual shared torpedo equipment first.');
 if(inspection.fields.some(field=>(field.path==='torpedoes'||field.path.startsWith('torpedoes.'))&&field.owner!==inspection.path)&&!operation.materialise)
   throw new Error('Explicitly materialise inherited shared flight equipment before editing.');
 const profile=structuredClone(operation.profile);validateGuidedProfile(profile);
 let after=replace(before,{...structuredClone(inspection.resolved.value.torpedoes),...profile});
 // Generic guided mounts declare the same shared speed/life envelope. Update
 // their data in this one accepted transaction rather than leaving an invalid rig.
 for(const row of inspection.weapons.filter(row=>row.section==='weapon'&&row.weapon.delivery.kind==='guided')){
   const candidate={paths:()=>draft.paths(),isBinary:path=>draft.isBinary(path),read:path=>path===operation.path?after:draft.read(path)};
   const weapon=structuredClone(row.weapon);weapon.mount.range=profile.speed*profile.lifespan;
   if(!Number.isFinite(Math.fround(weapon.mount.range))||Math.fround(weapon.mount.range)<=0)throw new Error('Shared guided range must remain representable.');
   after=prepareWeaponOperation(candidate,dependencies,{path:operation.path,section:'weapon',originalId:weapon.id,weapon,materialise:operation.materialise}).changes[0].after;
 }
 parse(after);return {changes:[{path:operation.path,before,after}]};
}
export async function applyGuidedProfileOperation({draft,provider,runtime,dependencies,operation,current=()=>true}){
 const selected=structuredClone(operation);return acceptWorkshopChanges({draft,provider,runtime,dependencies,current,stale:'stale-guided-profile',
 prepare:(candidate,effective)=>prepareGuidedProfileOperation(candidate,effective,selected).changes,
 inspect:(candidate,effective)=>inspectWeapons(candidate,effective,selected.path)});
}
