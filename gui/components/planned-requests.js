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

/** Accepted future work is visible without offering premature controls. */
export function renderPlannedRepairIntents(container,view){
  if(!container)return;
  const waiting=(view?.planned_intents||[]).filter(row=>row.released_tick===null||row.disposition);
  container.hidden=!waiting.length;
  container.replaceChildren();
  for(const intent of waiting){
    const row=container.ownerDocument.createElement('p');row.dataset.intent=intent.id;
    const subject=wireText(intent.target?.compartment??intent.target?.system);
    const requirements=intent.prerequisites.filter(dep=>!dep.satisfied).map(dep=>t(`planning.intent.wait_${dep.condition.kind}`)).join('; ')||t('planning.intent.wait_release');
    const status=intent.disposition;
    const detail=status?t(`planning.intent.status_${status.kind}`)+(status.kind==='restored_externally'?' '+t(status.consequence_applied===null?'planning.intent.consequence_pending':status.consequence_applied?'planning.intent.consequence_applied':'planning.intent.consequence_refused'):''):requirements;
    row.textContent=t('planning.intent.instruction',{subject,requirements:detail});
    container.append(row);
  }
}
