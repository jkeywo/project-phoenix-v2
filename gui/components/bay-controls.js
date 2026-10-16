import {t,wireText} from '../strings.js';
import {BAY_ACTION_IDS} from '../stations/bay-actions.js';
/** Own stored roster/current permitted recovery contacts, never foreign custody. */
export function mountBayControls(doc) {
 const node=doc.createElement('fieldset'),title=doc.createElement('legend'),status=doc.createElement('p');
 title.textContent=t('craft.bay.title');node.dataset.craftBay='';node.append(title,status);
 let owner=null,view=null,readonly=true,disposed=false;
 const controls={};
 for(const kind of ['launch','recover']) {
  const label=doc.createElement('label'),select=doc.createElement('select'),button=doc.createElement('button');
  label.textContent=t('craft.bay.'+kind);label.append(select);button.type='button';button.textContent=t('craft.bay.'+kind);node.append(label,button);
  controls[kind]={label,select,button};select.onchange=paint;
  select.onkeydown=event=>{if(!['Tab','Escape'].includes(event.key))event.stopPropagation();};
  button.onclick=()=>{if(button.disabled)return;emit(kind,kind==='launch'?{craft:select.value}:{contact:select.value});};
 }
 const cancel=doc.createElement('button');cancel.type='button';cancel.textContent=t('craft.bay.cancel');node.append(cancel);
 cancel.onclick=()=>{if(!cancel.disabled)emit('cancel',{});};
 const receptionList=doc.createElement('div'),receptionRows=new Map();node.append(receptionList);
 function receptionPaint(){
  const rows=view?.receptions || [],live=new Set();
  for(const record of rows){
   const key=record.order?.request?.origin_id;if(!key)continue;live.add(key);
   let row=receptionRows.get(key);
   if(!row){
    const node=doc.createElement('p'),text=doc.createElement('span'),button=doc.createElement('button');button.type='button';
    node.append(text,button);receptionList.append(node);row={node,text,button,record:null,armed:null};receptionRows.set(key,row);
    const signature=()=>JSON.stringify({owner,order:row.record?.order,contact:row.record?.contact});
    button.onpointerdown=()=>{row.armed=signature();};
    button.onkeydown=event=>{if(event.key==='Enter'||event.key===' '){row.armed=signature();event.stopPropagation();}};
    button.onclick=()=>{const armed=row.armed;row.armed=null;if(button.disabled || armed!==null&&armed!==signature())return;
      emit('recover',{contact:row.record.contact,reception:row.record.order});};
   }
   row.record=record;
   row.text.textContent=t('craft.bay.reception_from',{source:wireText(record.source_name || '')})+' '+t(record.contact?'craft.bay.reception_observed':'craft.bay.reception_waiting');
   row.button.textContent=t('craft.bay.recover');row.button.hidden=readonly;
   row.button.disabled=readonly||!view?.can_recover||!record.contact||!view.contacts?.some(contact=>contact.contact===record.contact);
  }
  for(const [key,row]of receptionRows)if(!live.has(key)){row.button.disabled=true;row.node.remove();receptionRows.delete(key);}
 }
 function emit(kind,detail){if(disposed)return;doc.defaultView.activateSemanticAction?.(BAY_ACTION_IDS[kind],{context:'bay',source:'control',detail:{owner,...detail}});}
 function paint(){
  for(const kind of ['launch','recover']) {
   const {button,select}=controls[kind],rows=kind==='launch'?view?.stored:view?.contacts;
   button.disabled=readonly||!view?.['can_'+kind]||!rows?.some(row=>(kind==='launch'?row.id:row.contact)===select.value);
   select.disabled=readonly||!rows?.length;
  }
  cancel.disabled=readonly||!view?.can_cancel;receptionPaint();
 }
 return {node,update(system,next,delegated=false){
  if(disposed)return;
  if(owner!==system)for(const row of Object.values(controls))row.signature=null;
  owner=system;view=next;node.hidden=!next;if(!next){for(const row of receptionRows.values())row.button.disabled=true;return;}
  readonly=delegated||!next.controllable;
  status.textContent=t('craft.bay.capacity',{used:next.occupied_space,total:next.capacity_space})
   +(next.crew_transfer_in_progress?' \u00b7 '+t('craft.bay.personnel_transfer'):'')
   +(next.progress==null?'':' · '+Math.round(next.progress/10)+'%')
   +(next.outcome?' · '+t('craft.bay.outcome.'+next.outcome):next.paused?' · '+t('craft.bay.paused'):next.ready?' · '+t('craft.bay.ready'):'');
  for(const kind of ['launch','recover']) {
   const row=controls[kind],options=(kind==='launch'?next.stored:next.contacts)||[],key=value=>kind==='launch'?value.id:value.contact;
   const signature=JSON.stringify(options.map(key));
   if(row.signature!==signature){const previous=row.select.value;row.signature=signature;row.select.replaceChildren(...options.map(value=>{const option=doc.createElement('option');option.value=key(value);return option;}));if(options.some(value=>key(value)===previous))row.select.value=previous;}
   options.forEach((value,index)=>{const text=kind==='launch'?(value.name?wireText(value.name):t('craft.bay.stored',{index:index+1})):t('craft.bay.contact',{index:index+1,tick:value.observed_tick});if(row.select.options[index].textContent!==text)row.select.options[index].textContent=text;});
   row.label.hidden=readonly;row.button.hidden=readonly;
  }
  cancel.hidden=readonly;paint();
 },dispose(){disposed=true;readonly=true;owner=null;view=null;
   for(const button of node.querySelectorAll('button'))button.disabled=true;
   for(const select of node.querySelectorAll('select'))select.disabled=true;
   for(const row of receptionRows.values()){row.record=null;row.armed=null;}
   node.remove();}};
}
