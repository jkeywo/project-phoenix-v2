import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';
import { familyView } from '../console-payload.js';

export const RADAR_SHARING_ACTION_ID = 'sensing.radar-sharing';
export const RADAR_SHARING_CONTEXTS = ['sensors', 'science', 'captain', 'tactical'];
const definition = defineStationAction({ id: RADAR_SHARING_ACTION_ID, contexts: RADAR_SHARING_CONTEXTS,
  labelKey: 'sensing.sharing.title', bindings: [null, null] });

export function radarSharingRows(payload) {
  const rows = [payload, familyView(payload, 'sensors'), familyView(payload, 'tactical')]
    .flatMap(view => Array.isArray(view?.radar_sharing) ? view.radar_sharing : []);
  const result = new Map();
  for (const row of rows) if (typeof row?.owner === 'string' && row.owner
    && ['sensor_radar', 'tactical_radar'].includes(row.role)
    && typeof row.share_own_faction === 'boolean' && typeof row.receive === 'boolean'
    && !result.has(row.owner)) result.set(row.owner, row);
  return [...result.values()];
}
export function registerRadarSharingActions(registry, options = {}) {
  const send = createCorrelatedActionSender(options.sendAction);
  registry.register(definition, ({ actionId, correlation, inputMs, detail } = {}) => {
    const row = radarSharingRows(options.getState?.()).find(row => row.owner === detail?.owner);
    if (!row?.controllable || !row.can_operate || typeof detail?.share_own_faction !== 'boolean' || typeof detail?.receive !== 'boolean') return false;
    return send(actionId, correlation, inputMs, 'set_radar_sharing', {
      control_system_id: row.owner, share_own_faction: detail.share_own_faction, receive: detail.receive,
    });
  });
  return registry;
}
export function createRadarSharingActionRegistry(options = {}) {
  return registerRadarSharingActions(createSemanticActionRegistry({ actionFeedback: options.actionFeedback }), options);
}
