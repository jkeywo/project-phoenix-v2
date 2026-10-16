import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
export const CAPTURE_SELECT_ACTION = defineStationAction({ id: 'comms.capture-select', contexts: ['comms-capture'], labelKey: 'comms.capture.select', bindings: [null, null] });
export function registerCaptureActions(registry, { getState, sendAction } = {}) {
  const send = createCorrelatedActionSender(sendAction);
  registry.register(CAPTURE_SELECT_ACTION, ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = getState?.()?.comms_capture, capture = detail?.capture;
    if (!view?.controllable || typeof view.system !== 'string' || !view.system
      || !(capture === null || Number.isSafeInteger(capture) && capture > 0
        && view.rows.some(row => row.capture === capture && row.feasible && row.evidence == null))) return false;
    return send(actionId, correlation, inputMs, 'comms_capture_select', { control_system_id: view.system, capture });
  });
  return registry;
}
