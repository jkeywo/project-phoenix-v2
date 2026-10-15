import {defineStationAction, createCorrelatedActionSender} from './action-support.js';
export const JAMMING_ACTION_IDS=Object.freeze({target:'comms.jamming-target',enabled:'comms.jamming-enabled'});
export const JAMMING_ACTIONS=Object.freeze(Object.entries(JAMMING_ACTION_IDS).map(([name,id])=>defineStationAction({id,contexts:['jamming'],labelKey:'jamming.'+name,bindings:[null,null]})));
export function registerJammingActions(registry,{getState,sendAction}={}) {
  const send=createCorrelatedActionSender(sendAction);
  for(const [index,name] of Object.keys(JAMMING_ACTION_IDS).entries()) registry.register(JAMMING_ACTIONS[index],({actionId,correlation,inputMs,detail}={})=>{
    const row=getState?.()?.system_status?.find(row=>row.system_id===detail?.owner && !row.delegated);
    const view=row?.jamming;if(!view?.controllable)return false;
    if(name==='target' && (detail.target_uuid===null || view.targets?.some(target=>target.contact===detail.target_uuid)))
      return send(actionId,correlation,inputMs,'jamming_target',{control_system_id:row.system_id,target_uuid:detail.target_uuid});
    if(name==='enabled' && typeof detail.enabled==='boolean')
      return send(actionId,correlation,inputMs,'jamming_enabled',{control_system_id:row.system_id,enabled:detail.enabled});
    return false;
  });return registry;
}
