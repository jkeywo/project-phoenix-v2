import {t} from './strings.js';
import {guidedProfile,GUIDED_FLIGHT_DEFAULTS} from '../editor/workshop-guided-profile.js';
/** One shared equipment profile; emitted rounds retain their frozen data. */
export function mountGuidedProfile({root,apply}){
 const doc=root.ownerDocument,node=doc.createElement('fieldset'),legend=doc.createElement('legend');legend.textContent=t('workshop.guided.title');node.append(legend);node.id='workshop-guided-profile';
 const label=(control,key)=>{const row=doc.createElement('label');row.htmlFor=control.id;row.textContent=t('workshop.guided.'+key);row.append(control);node.append(row);return control;};
 const input=(key,type='number')=>{const control=doc.createElement('input');control.id='workshop-guided-'+key;control.type=type;if(type==='number')control.step='any';return label(control,key);};
 const flight=Object.fromEntries(Object.keys(GUIDED_FLIGHT_DEFAULTS).map(key=>[key,input(key)]));
 const seekerEnabled=input('seeker','checkbox'),receiverEnabled=input('receiver','checkbox');
 const seeker=Object.fromEntries(['range','fire_arc_deg','minimum_transmission'].map(key=>[key,input('seeker-'+key)]));
 const receiver=Object.fromEntries(['range','minimum_transmission','maximum_sample_age_secs'].map(key=>[key,input('receiver-'+key)]));
 const external=input('external','checkbox'),materialise=input('materialise','checkbox');
 const save=doc.createElement('button');save.type='button';save.textContent=t('workshop.guided.apply');save.id='workshop-guided-apply';node.append(save);root.append(node);
 let inspection=null,stamp='',disabled=true,documentIdentity=null;
 function availability(){for(const control of node.querySelectorAll('input,button'))control.disabled=disabled;for(const control of Object.values(seeker))control.disabled||=!seekerEnabled.checked;for(const control of [...Object.values(receiver),external])control.disabled||=!receiverEnabled.checked;}
 seekerEnabled.onchange=receiverEnabled.onchange=availability;
 save.onclick=()=>{if(save.disabled)return;apply({path:inspection.path,materialise:materialise.checked,profile:{...Object.fromEntries(Object.entries(flight).map(([key,control])=>[key,control.valueAsNumber])),seeker:seekerEnabled.checked?Object.fromEntries(Object.entries(seeker).map(([key,control])=>[key,control.valueAsNumber])):false,receiver:receiverEnabled.checked?{...Object.fromEntries(Object.entries(receiver).map(([key,control])=>[key,control.valueAsNumber])),accept_external_illumination:external.checked}:false}});};
 return{refresh(next,blocked=false,identity=null){if(identity!==documentIdentity){stamp='';documentIdentity=identity;}inspection=next;const profile=guidedProfile(next?.resolved.value);node.hidden=!profile;disabled=blocked||!profile;
 const key=JSON.stringify([next?.path,profile]);if(profile&&key!==stamp){stamp=key;for(const[key,control]of Object.entries(flight))control.value=profile[key];seekerEnabled.checked=!!profile.seeker;receiverEnabled.checked=!!profile.receiver;
 const seekerValue=profile.seeker||{range:200,fire_arc_deg:90,minimum_transmission:0.1},receiverValue=profile.receiver||{range:200,minimum_transmission:0.1,maximum_sample_age_secs:2};
 for(const[key,control]of Object.entries(seeker))control.value=seekerValue[key];for(const[key,control]of Object.entries(receiver))control.value=receiverValue[key];external.checked=profile.receiver?.accept_external_illumination===true;materialise.checked=false;}
 availability();},dispose(){node.remove();}};
}
