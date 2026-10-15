import {defineStationAction,createCorrelatedActionSender} from './action-support.js';
export const IDENTITY_ENABLED_ACTION=defineStationAction({id:'comms.identity-enabled',contexts:['identity'],labelKey:'identity.enabled',bindings:[null,null]});
export function registerIdentityActions(registry,{getState,sendAction}={}) {
  const send=createCorrelatedActionSender(sendAction);
  registry.register(IDENTITY_ENABLED_ACTION,({actionId,correlation,inputMs,detail}={})=>{
    const row=getState?.()?.system_status?.find(row=>row.system_id===detail?.owner&&!row.delegated);
    if(!row?.identity?.controllable||typeof detail.enabled!=='boolean')return false;
    return send(actionId,correlation,inputMs,'identity_emitter_enabled',{control_system_id:row.system_id,enabled:detail.enabled});
  });
  return registry;
}
