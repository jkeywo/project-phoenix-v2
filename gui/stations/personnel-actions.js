import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView } from '../console-payload.js';
import { personnelPosting, personnelTreatment, personnelDestination, personnelTransfer } from '../personnel.js';
export const PERSONNEL_TREAT_ACTION_ID = 'repair.treat-officer';
export const PERSONNEL_DESTINATION_ACTION_ID = 'repair.officer-destination';
export const PERSONNEL_TRANSFER_ACTION_ID = 'repair.transfer-officer';
export const PERSONNEL_POST_ACTION_ID = 'repair.post-officer';
export const PERSONNEL_ACTION = defineStationAction({ id: PERSONNEL_POST_ACTION_ID,
  contexts: ['repair', 'engineering', 'captain'], labelKey: 'personnel.action.post', bindings: [null, null] });
export const PERSONNEL_ACTIONS = Object.freeze([PERSONNEL_ACTION,
  defineStationAction({id:PERSONNEL_TREAT_ACTION_ID,contexts:['repair','engineering','captain'],labelKey:'personnel.action.treat',bindings:[null,null]}),
  defineStationAction({id:PERSONNEL_DESTINATION_ACTION_ID,contexts:['repair','engineering','captain'],labelKey:'personnel.action.destination',bindings:[null,null]}),
  defineStationAction({id:PERSONNEL_TRANSFER_ACTION_ID,contexts:['repair','engineering','captain'],labelKey:'personnel.action.transfer',bindings:[null,null]})]);
export function registerPersonnelActions(registry, options = {}) {
  if (registry.action(PERSONNEL_POST_ACTION_ID)) return registry;
  const send = createCorrelatedActionSender(options.sendAction);
  registry.register(PERSONNEL_ACTION, ({ actionId, correlation, inputMs, detail } = {}) => {
    const repair = familyView(options.getState?.(), 'repair');
    const view = repair.personnel;
    if (!view || repair.repair_auto || repair.personnel_controllable !== true) return false;
    const request = personnelPosting(view, detail);
    return request ? send(actionId, correlation, inputMs, 'set_duty_officer_posting',
      { ...request, control_system_id: view.system_id }) : false;
  });
  registry.register(PERSONNEL_ACTIONS[1],
    ({ actionId, correlation, inputMs, detail } = {}) => {
      const repair = familyView(options.getState?.(), 'repair');
      if (repair.repair_auto || repair.personnel_controllable !== true) return false;
      const request = personnelTreatment(repair.personnel, detail);
      return request ? send(actionId, correlation, inputMs, 'treat_duty_officer', { ...request, control_system_id: repair.personnel.system_id }) : false;
    });
  for (const [action, validate, wire] of [[PERSONNEL_ACTIONS[2],personnelDestination,'set_duty_officer_destination'],[PERSONNEL_ACTIONS[3],personnelTransfer,'dispatch_officer_transfer']]) {
    registry.register(action, ({actionId,correlation,inputMs,detail} = {}) => {
      const repair = familyView(options.getState?.(), 'repair');
      if (repair.repair_auto || repair.personnel_controllable !== true) return false;
      const request = validate(repair.personnel, detail);
      return request ? send(actionId,correlation,inputMs,wire,{...request,control_system_id:repair.personnel.system_id}) : false;
    });
  }
  return registry;
}
