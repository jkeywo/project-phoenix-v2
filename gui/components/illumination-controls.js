import {t,wireText} from '../strings.js';
import {ILLUMINATION_ACTION_IDS} from '../stations/illumination-actions.js';
/** One Science emitter's own channels and paid emission; no receiver census. */
export function mountIlluminationControls(doc) {
  const node=doc.createElement('fieldset'),title=doc.createElement('legend'),label=doc.createElement('label'),picker=doc.createElement('select'),status=doc.createElement('p');
  title.textContent=t('illumination.title');label.textContent=t('illumination.target');label.append(picker);node.dataset.illumination='';
  const apply=doc.createElement('button'),clear=doc.createElement('button'),toggle=doc.createElement('button');
  for(const button of [apply,clear,toggle])button.type='button';
  apply.textContent=t('illumination.apply');clear.textContent=t('illumination.clear');
  node.append(title,status,label,apply,clear,toggle);
  let owner=null,value=null,readonly=true;
  const emit=(kind,fields)=>doc.defaultView.activateSemanticAction?.(ILLUMINATION_ACTION_IDS[kind],{context:'illumination',source:'control',detail:{owner,...fields}});
  function paint(){
    status.textContent=t(value?.emitting?'illumination.emitting':value?.enabled?'illumination.waiting':'illumination.off');
    toggle.textContent=t(value?.enabled?'illumination.disable':'illumination.enable');
    apply.disabled=readonly || !value?.targets?.some(row=>row.contact===picker.value);
    clear.disabled=readonly || value?.target==null;toggle.disabled=readonly;picker.disabled=readonly;
  }
  apply.onclick=()=>{if(!apply.disabled)emit('target',{target_uuid:picker.value});};
  clear.onclick=()=>{if(!clear.disabled)emit('target',{target_uuid:null});};
  toggle.onclick=()=>{if(!toggle.disabled)emit('enabled',{enabled:!value.enabled});};
  picker.addEventListener('change',paint);
  picker.addEventListener('keydown',event=>{if(!['Tab','Escape'].includes(event.key))event.stopPropagation();});
  return {node,update(system,next,delegated=false){
    if(owner!==system)picker.dataset.choices='';owner=system;value=next;node.hidden=!next;if(!next)return;
    readonly=delegated || !next.controllable;
    const signature=JSON.stringify(next.targets.map(row=>row.contact));
    if(picker.dataset.choices!==signature){
      const previous=picker.value;picker.dataset.choices=signature;
      picker.replaceChildren(...next.targets.map(row=>{const option=doc.createElement('option');option.value=row.contact;return option;}));
      if(next.targets.some(row=>row.contact===previous))picker.value=previous;
      else if(next.targets.some(row=>row.contact===next.target))picker.value=next.target;
    }
    next.targets.forEach((row,index)=>{const name=row.name?wireText(row.name):t('console.sensors.basic_contact');if(picker.options[index].textContent!==name)picker.options[index].textContent=name;});
    for(const control of [label,apply,clear,toggle])control.hidden=readonly;
    paint();
  },dispose(){node.remove();}};
}
