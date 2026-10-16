import {defineStationAction,createCorrelatedActionSender} from './action-support.js';
import {emissionIntents} from '../emission-profile.js';
export const EMISSION_INTENT_ACTION=defineStationAction({id:'command.emission-intent',contexts:['command-emission'],labelKey:'emission.intent',bindings:[null,null]});
/** Caller supplies only current permitted Command cards, not global ship state. */
export function registerEmissionProfileActions(registry,{getViews,sendAction}={}) {
  const send=createCorrelatedActionSender(sendAction);
  registry.register(EMISSION_INTENT_ACTION,({actionId,correlation,inputMs,detail}={})=>{
    if(!emissionIntents.includes(detail?.intent))return false;
    const choices=getViews?.();
    if(!Array.isArray(choices))return false;
    const current=choices.filter(row=>row?.owner===detail.owner&&row.controllable===true);
    if(current.length!==1)return false;
    return send(actionId,correlation,inputMs,'set_emission_intent',{control_system_id:detail.owner,intent:detail.intent});
  });
  return registry;
}
