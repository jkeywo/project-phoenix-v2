import {defineStationAction, createCorrelatedActionSender} from './action-support.js';
export const ILLUMINATION_ACTION_IDS=Object.freeze({target:'science.illumination-target',enabled:'science.illumination-enabled'});
export const ILLUMINATION_ACTIONS=Object.freeze(Object.entries(ILLUMINATION_ACTION_IDS).map(([name,id])=>defineStationAction({id,contexts:['illumination'],labelKey:'illumination.'+name,bindings:[null,null]})));
export function registerIlluminationActions(registry,{getState,sendAction}={}) {
  const send=createCorrelatedActionSender(sendAction);
  for(const [index,name] of Object.keys(ILLUMINATION_ACTION_IDS).entries()) registry.register(ILLUMINATION_ACTIONS[index],({actionId,correlation,inputMs,detail}={})=>{
    const row=getState?.()?.system_status?.find(row=>row.system_id===detail?.owner && !row.delegated);
    const view=row?.illumination;if(!view?.controllable)return false;
    if(name==='target' && (detail.target_uuid===null || view.targets?.some(target=>target.contact===detail.target_uuid)))
      return send(actionId,correlation,inputMs,'illumination_target',{control_system_id:row.system_id,target_uuid:detail.target_uuid});
    if(name==='enabled' && typeof detail.enabled==='boolean')
      return send(actionId,correlation,inputMs,'illumination_enabled',{control_system_id:row.system_id,enabled:detail.enabled});
    return false;
  });return registry;
}
