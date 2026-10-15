import {plannedDeliveryDispatch} from '../planned-requests.js';
import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView, familySystemId } from '../console-payload.js';
export const LOGISTICS_ACTION_IDS = Object.freeze({ enabled: 'logistics.enabled', target: 'stock.target', reserve: 'stock.reserve', priority: 'stock.priority', dispatch: 'logistics.dispatch', return: 'logistics.return' });
export const LOGISTICS_ACTIONS = Object.freeze(Object.entries(LOGISTICS_ACTION_IDS).map(([name, id]) => defineStationAction({ id,
  contexts: name === 'target' ? ['engineering', 'tactical', 'captain', 'stock'] : ['engineering', 'captain'],
  labelKey: 'logistics.action.' + name, bindings: [null, null] })));
const quantity = value => Number.isSafeInteger(value) && value >= 0;
export function registerLogisticsActions(registry, options = {}) {
  const send = createCorrelatedActionSender(options.sendAction);
  for (const [index, name] of Object.keys(LOGISTICS_ACTION_IDS).entries()) {
    // Composite screens can install several family adapters. Stock is shared.
    if (registry.action(LOGISTICS_ACTIONS[index].id)) continue;
    registry.register(LOGISTICS_ACTIONS[index], ({ actionId, correlation, inputMs, detail } = {}) => {
    const state = options.getState?.(), view = familyView(state, 'logistics'), tactical = familyView(state, 'tactical');
    if (!detail) return false;
    let target = view.system_id || familySystemId(state, 'logistics'), payload;
    if (name === 'target' && detail.system) {
      const local = state?.local_stock?.find(row => row.system_id === detail.system && row.store === detail.store);
      if (!local?.controllable || local.auto || !quantity(detail.amount) || detail.amount > local.ceiling) return false;
      target = local.system_id; payload = { store: local.store, amount: detail.amount };
    } else if (name === 'target' && tactical.stock_control?.store === detail.store) {
      const local = tactical.stock_control;
      if (local.auto || !local.system_id || !quantity(detail.amount) || detail.amount > local.ceiling) return false;
      target = local.system_id; payload = { store: detail.store, amount: detail.amount };
    } else {
      if (!target || view.logistics_auto) return false;
      if(name==='dispatch'&&detail.planned_request!=null){
        payload=plannedDeliveryDispatch(view,detail.planned_request);
        return payload?send(actionId,correlation,inputMs,'logistics_dispatch',{...payload,control_system_id:target}):false;
      }
      const store = view.stores?.find(row => row.id === detail.store);
      if (name === 'enabled' && typeof detail.enabled === 'boolean') payload = { enabled: detail.enabled };
      else if (name === 'priority' && store && Number.isInteger(detail.priority) && detail.priority >= 0 && detail.priority <= 65535)
        payload = { store: store.id, priority: detail.priority };
      else if (['target', 'reserve'].includes(name) && store?.role === 'main' && quantity(detail.amount) && detail.amount <= store.ceiling)
        payload = { store: store.id, amount: detail.amount };
      else if (name === 'dispatch' && quantity(detail.amount) && detail.amount > 0 && (!view.max_batch || detail.amount <= view.max_batch)) {
        const source = view.stores?.find(row => row.id === detail.source), destination = view.stores?.find(row => row.id === detail.destination);
        if (source?.role === 'main' && destination?.role === 'local' && source.resource === destination.resource)
          payload = { source: source.id, destination: destination.id, amount: detail.amount };
      } else if (name === 'return' && Number.isSafeInteger(detail.job) && view.deliveries?.some(row => row.id === detail.job && !row.return_requested)) payload = { job: detail.job };
    }
    return payload ? send(actionId, correlation, inputMs, 'logistics_' + name, { ...payload, control_system_id: target }) : false;
  });
  }
  return registry;
}
