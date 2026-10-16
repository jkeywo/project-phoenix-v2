import {defineStationAction, createCorrelatedActionSender} from './action-support.js';
import {sameOrder, exactOrder} from '../comms-traffic.js';
export const LISTEN_ORDER_ACTION = defineStationAction({id:'comms.listening-order', contexts:['comms-traffic'], labelKey:'comms.traffic.order', bindings:[null,null]});
export const REPORT_ALERT_ACTION = defineStationAction({id:'comms.report-alert', contexts:['comms-traffic'], labelKey:'comms.traffic.report', bindings:[null,null]});
export function registerTrafficActions(registry, {getState, sendAction} = {}) {
  const send = createCorrelatedActionSender(sendAction);
  const current = detail => {
    const view = getState?.()?.comms_traffic;
    return view?.controllable && detail?.system === view.system ? view : null;
  };
  registry.register(LISTEN_ORDER_ACTION, ({actionId, correlation, inputMs, detail} = {}) => {
    const view = current(detail);
    if (!view || !view.priority.length || !sameOrder(detail.expected_priority, view.priority)
      || !exactOrder(detail.priority, view.channels)) return false;
    return send(actionId, correlation, inputMs, 'set_comms_listening_order', {control_system_id:view.system, priority:[...detail.priority]});
  });
  registry.register(REPORT_ALERT_ACTION, ({actionId, correlation, inputMs, detail} = {}) => {
    const view = current(detail);
    if (!view?.report_supported) return false;
    return send(actionId, correlation, inputMs, 'report_comms_alert', {control_system_id:view.system});
  });
  return registry;
}
