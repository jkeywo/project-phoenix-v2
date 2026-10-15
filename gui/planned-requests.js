const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
const id=value=>typeof value==='string'&&value.trim().length>0&&value.length<=1024&&!/[\u0000-\u001f\u007f]/.test(value);
const tick=value=>Number.isSafeInteger(value)&&value>=0;
const keys=(value,allowed)=>object(value)&&Object.keys(value).every(key=>allowed.includes(key));
export function repairWorkTarget(value){return keys(value,['kind','compartment','system','component'])&&
  (value.kind==='compartment'?id(value.compartment)&&!Object.hasOwn(value,'system')&&!Object.hasOwn(value,'component'):
    value.kind==='component'&&id(value.system)&&id(value.component)&&!Object.hasOwn(value,'compartment'));}
export function repairRequest(value){return keys(value,['type','data'])&&
  (value.type==='Work'?repairWorkTarget(value.data):['Station','Console','System'].includes(value.type)?id(value.data):value.type==='Core'&&!Object.hasOwn(value,'data'));}
/** Instructions copied only from the already-permitted producing publication. */
export function plannedRequests(value,kind){
  if(!Array.isArray(value)||value.length>32)return [];
  const seen=new Set(),result=[];
  for(const row of value){
    if(!object(row)||!id(row.id)||!tick(row.released_tick)||seen.has(row.id))return [];
    const valid=kind==='repair'?keys(row,['id','released_tick','team','request','target','goal'])&&Number.isInteger(row.team)&&row.team>=0&&row.team<=255&&repairRequest(row.request)&&repairWorkTarget(row.target)&&(row.target.kind==='compartment'?row.goal==='structure':['patch','full'].includes(row.goal))&&(row.request.type!=='Work'||row.request.data.kind===row.target.kind&&row.request.data.compartment===row.target.compartment&&row.request.data.system===row.target.system&&row.request.data.component===row.target.component):
      kind==='delivery'&&keys(row,['id','released_tick','source','destination','commodity','amount'])&&id(row.source)&&id(row.destination)&&id(row.commodity)&&Number.isSafeInteger(row.amount)&&row.amount>0;
    if(!valid)return [];
    seen.add(row.id);result.push(structuredClone(row));
  }
  return result;
}
export function plannedRepairDispatch(view,requested){
  if(view?.planned_controllable!==true||view.repair_auto)return null;
  const row=plannedRequests(view.planned_requests,'repair').find(row=>row.id===requested);
  if(!row||!view.teams?.some(team=>team.id===row.team&&team.status==='idle')||view.external_dispatch?.team_idx===row.team)return null;
  return {team_idx:row.team,target:row.request};
}
export function plannedDeliveryDispatch(view,requested){
  if(view?.planned_controllable!==true||view.logistics_auto)return null;
  const row=plannedRequests(view.planned_requests,'delivery').find(row=>row.id===requested);
  if(!row||row.amount>view.max_batch)return null;
  const from=view.stores?.find(store=>store.id===row.source),to=view.stores?.find(store=>store.id===row.destination);
  if(from?.role!=='main'||to?.role!=='local'||from.resource!==row.commodity||to.resource!==row.commodity)return null;
  return {source:row.source,destination:row.destination,amount:row.amount};
}
