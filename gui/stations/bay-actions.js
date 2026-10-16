import {normalizeCraftReception} from '../comms-requests.js';
import {defineStationAction,createCorrelatedActionSender} from './action-support.js';
export const BAY_ACTION_IDS=Object.freeze({launch:'craft.launch',recover:'craft.recover',cancel:'craft.cancel'});
export const BAY_ACTIONS=Object.freeze(Object.entries(BAY_ACTION_IDS).map(([kind,id])=>defineStationAction({id,contexts:['bay'],labelKey:'craft.bay.'+kind,bindings:[null,null]})));
export function registerBayActions(registry,{getState,sendAction}={}) {
 const send=createCorrelatedActionSender(sendAction);
 for(const [index,kind]of Object.keys(BAY_ACTION_IDS).entries())registry.register(BAY_ACTIONS[index],({actionId,correlation,inputMs,detail}={})=>{
  const row=getState?.()?.system_status?.find(row=>row.system_id===detail?.owner&&!row.delegated),view=row?.bay;
  if(!view?.controllable)return false;
  if(kind==='launch'&&view.can_launch&&view.stored?.some(craft=>craft.id===detail.craft))
   return send(actionId,correlation,inputMs,'launch_craft',{control_system_id:row.system_id,craft:detail.craft});
  if(kind==='recover'&&view.can_recover&&view.contacts?.some(contact=>contact.contact===detail.contact)) {
   const reception=detail.reception===undefined?undefined:normalizeCraftReception(detail.reception);
   if(detail.reception!==undefined && (!reception || !view.receptions?.some(held=>held.contact===detail.contact
      && JSON.stringify(normalizeCraftReception(held.order))===JSON.stringify(reception))))return false;
   return send(actionId,correlation,inputMs,'recover_craft',{control_system_id:row.system_id,contact:detail.contact,
      ...(reception?{reception}:{})});
  }
  if(kind==='cancel'&&view.can_cancel)return send(actionId,correlation,inputMs,'cancel_craft_handling',{control_system_id:row.system_id});
  return false;
 });return registry;
}
