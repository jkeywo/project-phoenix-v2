import {t} from './strings.js';
import {inspectEmissionProfile,applyEmissionProfileOperation} from '../editor/workshop-emission-profile.js';
import {runWorkshopMutation} from './workshop-edit-session.js';

/** Existing draft/history, accepted native source editor and Preview/Test owner. */
export function mountWorkshopEmissionProfile({root,provider,runtime,draft:getDraft,busy=()=>false,setBusy=()=>{},changed=()=>{},preview=()=>false,test=()=>false,attach=true}) {
  const doc=root.ownerDocument,node=doc.createElement('section'),status=doc.createElement('p');
  const create=(tag,id,key)=>{const value=doc.createElement(tag);value.id='workshop-emission-'+id;if(key)value.textContent=t('workshop.emission.'+key);return value;};
  const label=(control,key)=>{const value=doc.createElement('label');value.htmlFor=control.id;value.textContent=t('workshop.emission.'+key);value.append(control);return value;};
  const title=create('h3','title','title'),template=create('select','template'),owner=create('select','owner'),initial=create('select','initial'),rate=create('input','rate'),materialise=create('input','materialise');
  rate.type='number';rate.step='any';rate.min='0';materialise.type='checkbox';
  for(const value of ['normal','low_emission']){const option=doc.createElement('option');option.value=value;option.textContent=t('emission.'+value);initial.append(option);}
  const save=create('button','save','save'),show=create('button','preview','preview'),exercise=create('button','test','test');
  for(const button of [save,show,exercise])button.type='button';status.setAttribute('role','status');
  node.dataset.workshopEmission='';node.append(title,label(template,'template'),label(owner,'owner'),label(initial,'initial'),label(rate,'rate'),label(materialise,'materialise'),save,show,exercise,status);
  let dependencies=null,inspection=null,disposed=false,loading=false,generation=0,observed=null,observedDraft=null;
  function controls(){for(const control of node.querySelectorAll('input,select,button'))control.disabled=disposed||loading||busy()||!inspection?.owners.length;template.disabled=disposed||loading||busy()||!dependencies;}
  function inspect(force=false){
    if(disposed)return;
    const draft=getDraft();if(!draft||!dependencies){inspection=null;controls();return;}
    const paths=draft.paths().filter(path=>path.startsWith('assets/entities/')&&path.endsWith('.toml')&&!draft.isBinary(path)).sort();
    const path=paths.includes(template.value)?template.value:paths[0]||'';
    if([...template.options].map(row=>row.value).join('\n')!==paths.join('\n'))template.replaceChildren(...paths.map(path=>{const option=doc.createElement('option');option.value=option.textContent=path;return option;}));template.value=path;
    const key=JSON.stringify([draft.sourceRevision,path]);if(!force&&observedDraft===draft&&observed===key){controls();return;}
    try{inspection=inspectEmissionProfile(draft,dependencies,path);observed=key;observedDraft=draft;
      const choices=inspection.selected?[inspection.selected.owner]:inspection.owners;
      owner.replaceChildren(...choices.map(value=>{const option=doc.createElement('option');option.value=option.textContent=value;return option;}));
      initial.value=inspection.selected?.config.initial??'normal';rate.value=inspection.selected?.config.advisory_rate??'0.25';materialise.checked=false;status.textContent=choices.length?'':t('workshop.emission.unavailable');
    }catch(error){inspection=null;status.textContent=t('workshop.emission.refused',{reason:error.message});}controls();
  }
  async function load(){if(disposed||loading||dependencies||typeof runtime.dependencies!=='function')return;loading=true;controls();const token=++generation;
    try{const value=await runtime.dependencies();if(!disposed&&token===generation)dependencies=value;}
    catch(error){if(!disposed)status.textContent=t('workshop.emission.refused',{reason:error.message});}
    finally{if(!disposed&&token===generation){loading=false;inspect(true);}}
  }
  template.onchange=()=>{generation++;inspect(true);};
  save.onclick=()=>{
    if(disposed||busy()||!inspection?.owners.includes(owner.value))return;
    const captured=getDraft(),token=++generation,operation={path:template.value,owner:owner.value,value:{initial:initial.value,advisory_rate:rate.valueAsNumber},materialise:materialise.checked};
    const current=()=>!disposed&&token===generation&&getDraft()===captured;
    void runWorkshopMutation({setBusy,current,start:controls,invoke:()=>applyEmissionProfileOperation({draft:captured,provider,runtime,dependencies,operation,current}),
      success:()=>{changed(operation.path);inspect(true);status.textContent=t('workshop.changed');},error:error=>{status.textContent=t('workshop.emission.refused',{reason:error.report?.findings?.[0]?.message||error.message});},release:controls});
  };
  show.onclick=()=>{if(!disposed&&!busy()&&inspection?.owners.length)preview(template.value);};exercise.onclick=()=>{if(!disposed&&!busy()&&inspection?.owners.length)test(template.value);};
  if(attach)root.append(node);void load();
  return {node,update(){if(disposed)return;inspect();void load();},dispose(){disposed=true;generation++;inspection=null;controls();node.remove();}};
}
