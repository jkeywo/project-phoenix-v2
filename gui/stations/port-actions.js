import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView, familySystemId } from '../console-payload.js';
export const PORT_ACTION_IDS = Object.freeze({ maintenance: 'port.maintenance', start: 'port.start-order', stop: 'port.stop-order', store: 'port.transfer-store' });
export const PORT_ACTIONS = Object.freeze(Object.entries(PORT_ACTION_IDS).map(([name, id]) => defineStationAction({ id,
  contexts: ['engineering', 'umbilical', 'repair', 'captain'], labelKey: 'port.action.' + name, bindings: [null, null] })));
const quantity = value => Number.isSafeInteger(value) && value >= 0;
export function registerPortActions(registry, options = {}) {
  const send = createCorrelatedActionSender(options.sendAction);
  for (const [i, name] of Object.keys(PORT_ACTION_IDS).entries()) {
    if (registry.action(PORT_ACTIONS[i].id)) continue;
    registry.register(PORT_ACTIONS[i], ({ actionId, correlation, inputMs, detail } = {}) => {
      const state = options.getState?.(), view = familyView(state, 'umbilical'), owner = view.system_id || familySystemId(state, 'umbilical');
      if (!owner || view.umbilical_auto || view.umbilical_controllable === false) return false;
      const port = view.port; let payload;
      if (name === 'maintenance' && typeof detail?.enabled === 'boolean') payload = { enabled: detail.enabled };
      else if (name === 'stop' && port?.order?.active) payload = {};
      else if (name === 'store' && !view.running && (detail?.store === null
        || port?.stores?.some(row => row.id === detail?.store && row.resource === view.selected_resource && row.role === 'main'))) payload = { store: detail.store };
      else if (name === 'start' && port?.policy === 'manual_offer' && port.eligible && !port.order?.active
        && !view.running && detail?.partner_uuid === port.partner_uuid && detail.visit === port.visit && Array.isArray(detail.lines) && detail.lines.length) {
        const offers = new Set();
        const valid = detail.lines.every(line => {
          if (!line || typeof line !== 'object' || !quantity(line.lots) || !line.lots) return false;
          const offer = port.offers.find(row => row.id === line.offer), store = port.stores.find(row => row.id === line.store);
          const units = line.lots * (offer?.unit_amount || 0);
          const headroom = store ? Math.max(0, Math.min(store.ceiling - store.level - store.incoming - (store.held || 0), store.headroom ?? 0)) : 0;
          if (!offer || !store || store.role !== 'main' || store.resource !== offer.resource || offers.has(line.offer) || !quantity(line.lots) || !line.lots) return false;
          offers.add(line.offer);
          return quantity(units) && units <= headroom && line.lots <= offer.quota_remaining && line.lots <= offer.available_lots;
        });
        if (valid) payload = { partner_uuid: detail.partner_uuid, visit: detail.visit,
          lines: detail.lines.map(({ offer, store, lots }) => ({ offer, store, lots })) };
      }
      return payload ? send(actionId, correlation, inputMs, 'port_' + name, { ...payload, control_system_id: owner }) : false;
    });
  }
  return registry;
}
