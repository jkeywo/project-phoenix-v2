import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView, familySystemId } from '../console-payload.js';
export const RECOVERY_ACTION_IDS = Object.freeze({ enabled: 'recovery.enabled', selection: 'recovery.selection' });
export const RECOVERY_ACTIONS = Object.freeze(Object.entries(RECOVERY_ACTION_IDS).map(([name, id]) => defineStationAction({ id,
  contexts: ['engineering', 'recovery', 'captain'], labelKey: 'recovery.action.' + name, bindings: [null, null] })));
export function registerRecoveryActions(registry, options = {}) {
  const send = createCorrelatedActionSender(options.sendAction);
  for (const [i, name] of Object.keys(RECOVERY_ACTION_IDS).entries()) {
    if (registry.action(RECOVERY_ACTIONS[i].id)) continue;
    registry.register(RECOVERY_ACTIONS[i], ({ actionId, correlation, inputMs, detail } = {}) => {
      const state = options.getState?.(), view = familyView(state, 'recovery'), owner = view.system_id || familySystemId(state, 'recovery');
      if (!owner || view.recovery_auto || view.recovery_controllable === false || !detail) return false;
      let payload;
      if (name === 'enabled' && typeof detail.enabled === 'boolean') payload = { enabled: detail.enabled };
      else if (name === 'selection') {
        const { target_uuid, resource, destination_store } = detail;
        if ([target_uuid, resource, destination_store].every(value => value === null)) payload = { target_uuid, resource, destination_store };
        else if ([target_uuid, resource, destination_store].every(value => typeof value === 'string' && value.length > 0)
          && view.candidates?.find(row => row.uuid === target_uuid)?.resources?.includes(resource)
          && view.destinations?.some(row => row.id === destination_store && row.resource === resource && row.role === 'main'))
          payload = { target_uuid, resource, destination_store };
      }
      return payload ? send(actionId, correlation, inputMs, 'recovery_' + name, { ...payload, control_system_id: owner }) : false;
    });
  }
  return registry;
}
