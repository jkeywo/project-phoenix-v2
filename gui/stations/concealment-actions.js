import {defineStationAction,createCorrelatedActionSender} from './action-support.js';
export const CONCEALMENT_ENABLED_ACTION=defineStationAction({id:'science.concealment-enabled',contexts:['concealment'],labelKey:'concealment.enabled',bindings:[null,null]});
export function registerConcealmentActions(registry,{getState,sendAction}={}) {
  const send=createCorrelatedActionSender(sendAction);
  registry.register(CONCEALMENT_ENABLED_ACTION,({actionId,correlation,inputMs,detail}={})=>{
    const row=getState?.()?.system_status?.find(row=>row.system_id===detail?.owner&&!row.delegated);
    if(!row?.concealment?.controllable||typeof detail.enabled!=='boolean')return false;
    return send(actionId,correlation,inputMs,'concealment_enabled',{control_system_id:row.system_id,enabled:detail.enabled});
  });
  return registry;
}
