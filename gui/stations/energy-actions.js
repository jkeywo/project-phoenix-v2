/** Numeric energy controls use the ordinary correlated Engineering dispatcher. */
import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familySystemId, familyView } from '../console-payload.js';

export const ENERGY_ACTION_IDS = Object.freeze({ reactor: 'power.energy-reactor', group: 'power.energy-group',
  priority: 'power.energy-priority', mode: 'power.energy-mode', system: 'power.energy-system-flow' });
export const ENERGY_ACTIONS = Object.freeze(Object.entries(ENERGY_ACTION_IDS).map(([name, id]) => defineStationAction({
  id, contexts: ['power', 'engineering', 'captain'], labelKey: 'power.energy_' + name, bindings: [null, null],
})));
const bounded = (value, maximum) => Number.isFinite(value) && value >= 0 && value <= maximum;
export function energyActionView(state) {
  const projected = familyView(state, 'power'); return projected?.energy ? projected : state?.energy ? state : null;
}
export function registerEnergyActions(registry, options = {}) {
  const send = createCorrelatedActionSender(options.sendAction);
  for (const [index, name] of Object.keys(ENERGY_ACTION_IDS).entries()) registry.register(ENERGY_ACTIONS[index], ({ actionId, correlation, inputMs, detail } = {}) => {
    const state = options.getState?.(), view = energyActionView(state), model = view?.energy;
    const target = view?.system_id || familySystemId(state, 'power');
    if (!model || view.power_auto || !target || !detail) return false;
    let payload;
    if (name === 'reactor' && bounded(detail.output_per_sec, model.reactor.max_output_per_sec) && bounded(detail.cooling_per_sec, model.reactor.max_cooling_per_sec))
      payload = { output_per_sec: detail.output_per_sec, cooling_per_sec: detail.cooling_per_sec };
    else if (name === 'mode' && ['manual', 'minimal'].includes(detail.mode) && ['none', 'low', 'high'].includes(detail.cooling_usage))
      payload = { mode: detail.mode, cooling_usage: detail.cooling_usage };
    else if (name === 'system') {
      const consumer = model.consumers.find(row => row.system === detail.system);
      if (consumer && (detail.flow_per_sec === null || bounded(detail.flow_per_sec, consumer.max_intake_per_sec)))
        payload = { system: detail.system, flow_per_sec: detail.flow_per_sec };
    } else {
      const group = model.groups.find(row => row.id === detail.group);
      if (group && name === 'group' && bounded(detail.flow_per_sec, group.max_flow_per_sec) && bounded(detail.cooling_per_sec, group.max_cooling_per_sec))
        payload = { group: detail.group, flow_per_sec: detail.flow_per_sec, cooling_per_sec: detail.cooling_per_sec };
      else if (group && name === 'priority' && Number.isInteger(detail.priority) && bounded(detail.priority, model.groups.filter(row => row.parent === group.parent).length - 1))
        payload = { group: detail.group, priority: detail.priority };
    }
    return payload ? send(actionId, correlation, inputMs, 'set_energy_' + name, { ...payload, control_system_id: target }) : false;
  });
  return registry;
}
