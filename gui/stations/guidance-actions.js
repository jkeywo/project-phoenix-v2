import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
export const GUIDANCE_ACTION_IDS = Object.freeze({ target: 'weapons.guidance-target', stop: 'weapons.guidance-stop' });
export const GUIDANCE_ACTIONS = Object.freeze(Object.entries(GUIDANCE_ACTION_IDS).map(([name, id]) => defineStationAction({
  id, contexts: ['guidance'], labelKey: 'guidance.' + name, bindings: [null, null],
})));
/** Own emitted identities are capabilities only at this exact tube's row. */
export function registerGuidanceActions(registry, { getState, sendAction } = {}) {
  const send = createCorrelatedActionSender(sendAction);
  for (const [index, name] of Object.keys(GUIDANCE_ACTION_IDS).entries()) registry.register(GUIDANCE_ACTIONS[index],
    ({ actionId, correlation, inputMs, detail } = {}) => {
      if (!detail || typeof detail.owner !== 'string') return false;
      const row = getState?.()?.system_status?.find(row => row.system_id === detail.owner && !row.delegated);
      const view = row?.guidance, round = view?.rounds?.find(round => round.round_uuid === detail.round_uuid && round.tube === row.system_id);
      if (!round) return false;
      if (name === 'target' && round.can_retarget && view.targets?.some(target => target.contact === detail.target_uuid))
        return send(actionId, correlation, inputMs, 'guidance_target', { control_system_id: row.system_id, round_uuid: round.round_uuid, target_uuid: detail.target_uuid });
      if (name === 'stop' && round.can_stop)
        return send(actionId, correlation, inputMs, 'guidance_stop', { control_system_id: row.system_id, round_uuid: round.round_uuid });
      return false;
    });
  return registry;
}
