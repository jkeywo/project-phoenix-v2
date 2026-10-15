import {t,wireText} from '../strings.js';
const states=new WeakMap();
/** Stable own instruction rows; no current condition/stock inferred here. */
export function renderPlannedRequests(container,view,onActivate){
  if(!container)return;states.set(container,{view,onActivate});
  container.hidden=!view?.planned_requests?.length;
  const requests=view?.planned_requests||[],present=new Set(requests.map(row=>row.id));
  for(const node of [...container.children])if(!present.has(node.dataset.request))node.remove();
  for(const request of requests){
    let node=[...container.children].find(node=>node.dataset.request===request.id);
    if(!node){node=container.ownerDocument.createElement('p');node.dataset.request=request.id;
      const label=container.ownerDocument.createElement('span'),button=container.ownerDocument.createElement('button');button.type='button';
      button.textContent=t('planning.request.apply');button.addEventListener('click',()=>{const current=states.get(container);if(current?.view?.planned_controllable===true&&current.view.planned_requests?.some(row=>row.id===node.dataset.request))current.onActivate(node.dataset.request);});
      node.append(label,button);container.append(node);}
    const subject=request.request?(request.request.type==='Work'?wireText(request.request.data.compartment??request.request.data.system)+' '+(request.request.data.component??''):wireText(request.request.data??request.request.type)):`${request.amount} ${wireText(request.commodity)}: ${wireText(request.source)} → ${wireText(request.destination)}`;
    node.firstChild.textContent=t('planning.request.instruction',{id:request.id,tick:request.released_tick,subject})+' ';
    node.lastChild.disabled=view.planned_controllable!==true;
  }
}
