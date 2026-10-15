import {defineStationAction,createCorrelatedActionSender} from './action-support.js';
import {familyView} from '../console-payload.js';

export const HELM_TRANSIT_ACTIONS=Object.freeze(['begin','cancel'].map(kind=>defineStationAction({
  id:`helm.transit-${kind}`,contexts:['helm'],labelKey:`helm.transit.${kind}`,bindings:[null,null],
})));
export function registerTransitActions(registry,{getState=()=>null,sendAction}={}) {
  const send=createCorrelatedActionSender(sendAction);
  for(const [index,kind] of ['begin','cancel'].entries()) registry.register(HELM_TRANSIT_ACTIONS[index],({actionId,correlation,inputMs,detail}={})=>{
    const state=getState(),projected=familyView(state||{},'helm');
    const view=Object.keys(projected).length?projected:state;
    const drive=view?.transit_drives?.find(row=>row.system_id===detail?.systemId&&row.controllable===true);
    if(!drive||drive[`can_${kind}`]!==true)return false;
    return send(actionId,correlation,inputMs,kind==='begin'?'begin_transit':'cancel_transit',{control_system_id:drive.system_id});
  });
  return registry;
}
