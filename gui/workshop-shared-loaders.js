import {t} from './strings.js';
/** Optional physical groups; accepted source editing owns validation/history. */
export function mountSharedLoaders({root,apply}){
  const doc=root.ownerDocument,node=doc.createElement('fieldset'),legend=doc.createElement('legend');legend.textContent=t('workshop.loader.title');node.append(legend);root.append(node);
  const select=doc.createElement('select'),id=doc.createElement('input'),rate=doc.createElement('input'),members=doc.createElement('select'),materialise=doc.createElement('input');
  rate.type='number';rate.step='any';rate.min='0';members.multiple=true;materialise.type='checkbox';
  let serial=0;const label=(control,key)=>{const row=doc.createElement('label');control.id='workshop-shared-loader-'+(++serial);row.htmlFor=control.id;row.textContent=t(key);row.append(control);node.append(row);};
  label(select,'workshop.loader.title');label(id,'workshop.loader.id');label(rate,'workshop.loader.rate');label(members,'workshop.loader.members');label(materialise,'workshop.guided.materialise');
  const save=doc.createElement('button'),remove=doc.createElement('button');save.type=remove.type='button';save.textContent=t('workshop.loader.save');remove.textContent=t('workshop.loader.remove');node.append(save,remove);
  let current=null,identity=null,disabled=true,disposed=false;
  const fill=(control,values)=>{control.replaceChildren();for(const value of values){const option=doc.createElement('option');option.value=option.textContent=value;control.append(option);}};
  const paint=()=>{const group=current?.resolved.value.torpedoes?.loader_groups?.find(group=>group.id===select.value);id.value=group?.id??'';rate.value=group?.work_rate??1;for(const option of members.options)option.selected=!!group?.tubes.includes(option.value);};
  select.addEventListener('change',paint);
  const submit=deleting=>{
    if(disposed||disabled||!current)return;
    const groups=structuredClone(current.resolved.value.torpedoes?.loader_groups??[]).filter(group=>group.id!==select.value);
    if(!deleting)groups.push({id:id.value,work_rate:rate.valueAsNumber,tubes:[...members.selectedOptions].map(option=>option.value)});
    void apply({path:current.path,groups,materialise:materialise.checked});
  };
  save.addEventListener('click',()=>submit(false));remove.addEventListener('click',()=>submit(true));
  return {node,refresh(inspection,busy,draft){current=inspection;disabled=!!busy||!inspection?.resolved.value.torpedoes;node.hidden=!inspection?.resolved.value.torpedoes;
    const key=[draft,inspection?.path,draft?.sourceRevision];if(identity?.[0]!==key[0]||identity?.[1]!==key[1]||identity?.[2]!==key[2]){identity=key;
      fill(select,['',...(inspection?.resolved.value.torpedoes?.loader_groups??[]).map(group=>group.id)]);
      fill(members,(inspection?.weapons??[]).filter(row=>row.weapon.delivery.kind==='guided').map(row=>row.weapon.strategy_id||row.weapon.id));materialise.checked=false;paint();}
    for(const input of node.querySelectorAll('input,select,button'))input.disabled=disabled;
  },dispose(){disposed=true;current=null;node.remove();}};
}
