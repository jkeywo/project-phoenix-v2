import { defineStationAction, createCorrelatedActionSender } from './action-support.js';

export const FITTING_ACTION_IDS = Object.freeze({ request: 'fitting.request', dispatch: 'fitting.dispatch', cancel: 'fitting.cancel', recover: 'fitting.recover', 'recover-cancel': 'fitting.recover-cancel' });
export const FITTING_ACTIONS = Object.freeze(Object.entries(FITTING_ACTION_IDS).map(([name, id]) => defineStationAction({
  id, contexts: ['fitting'], labelKey: 'fitting.' + name, bindings: [null, null],
})));

/** A visible fitting report never grants the reporting Console machinery control. */
export function registerFittingActions(registry, { getState, sendAction } = {}) {
  const send = createCorrelatedActionSender(sendAction);
  for (const [index, name] of Object.keys(FITTING_ACTION_IDS).entries()) {
    registry.register(FITTING_ACTIONS[index], ({ actionId, correlation, inputMs, detail } = {}) => {
      if (!detail || typeof detail.owner !== 'string') return false;
      const row = getState?.()?.system_status?.find(row => row.system_id === detail.owner && !row.delegated);
      if (name === 'recover' || name === 'recover-cancel') {
        const recovery = row?.equipment_recovery;
        if (!recovery) return false;
        let payload;
        if (name === 'recover' && recovery.can_start && recovery.options?.some(option => !option.blocked_reason && option.target_uuid === detail.target_uuid && option.slot === detail.slot && option.destination_locker === detail.destination_locker))
          payload = { target_uuid: detail.target_uuid, slot: detail.slot, destination_locker: detail.destination_locker };
        else if (name === 'recover-cancel' && recovery.can_cancel) payload = {};
        return payload ? send(actionId, correlation, inputMs, name === 'recover' ? 'fitting_recover' : 'fitting_recover_cancel', { ...payload, control_system_id: row.system_id }) : false;
      }
      const slot = row?.fitting?.find(slot => slot.slot === detail.slot);
      if (!slot) return false;
      let target, payload;
      if (name === 'request' && slot.can_request && slot.system === row.system_id
        && slot.choices?.some(choice => choice.equipment === detail.equipment)) {
        target = slot.system; payload = { slot: slot.slot, equipment: detail.equipment };
      } else if (name === 'dispatch' && slot.can_dispatch && Number.isInteger(detail.team_idx)
        && slot.free_teams?.includes(detail.team_idx)) {
        target = row.system_id; payload = { slot: slot.slot, team_idx: detail.team_idx };
      } else if (name === 'cancel' && slot.can_dispatch && (slot.desired || slot.phase !== 'idle')) {
        target = row.system_id; payload = { slot: slot.slot };
      }
      return payload ? send(actionId, correlation, inputMs, 'fitting_' + name, { ...payload, control_system_id: target }) : false;
    });
  }
  return registry;
}
