import {defineStationAction, createCorrelatedActionSender} from './action-support.js';

export const PROTECTION_ACTION_IDS = Object.freeze({
  enabled: 'sensors.signal-protection-enabled',
  priorities: 'sensors.signal-protection-priorities',
});
export const PROTECTION_ACTIONS = Object.freeze(Object.entries(PROTECTION_ACTION_IDS).map(([name, id]) =>
  defineStationAction({id, contexts: ['protection'], labelKey: `protection.${name}`, bindings: [null, null]})));

export function registerProtectionActions(registry, {getState, sendAction} = {}) {
  const send = createCorrelatedActionSender(sendAction);
  for (const [index, name] of Object.keys(PROTECTION_ACTION_IDS).entries()) {
    registry.register(PROTECTION_ACTIONS[index], ({actionId, correlation, inputMs, detail} = {}) => {
      const row = getState?.()?.system_status?.find(row => row.system_id === detail?.owner && !row.delegated);
      const view = row?.signal_protection;
      if (!view?.controllable) return false;
      if (name === 'enabled' && typeof detail.enabled === 'boolean') {
        return send(actionId, correlation, inputMs, 'signal_protection_enabled', {control_system_id: row.system_id, enabled: detail.enabled});
      }
      const systems = detail?.systems;
      if (name === 'priorities' && Array.isArray(systems) && systems.length <= 64
          && new Set(systems).size === systems.length
          && systems.every(system => view.systems?.some(choice => choice.system === system))) {
        return send(actionId, correlation, inputMs, 'signal_protection_priorities', {control_system_id: row.system_id, systems: [...systems]});
      }
      return false;
    });
  }
  return registry;
}
