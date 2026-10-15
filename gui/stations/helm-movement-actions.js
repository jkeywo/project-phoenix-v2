import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView } from '../console-payload.js';

const action = (id, label) => defineStationAction({ id: `helm.${id}`, contexts: ['helm'],
  labelKey: `helm.${label}`, bindings: [null, null] });
export const HELM_MODE_ACTION = action('mode', 'mode');
export const HELM_SPEED_ACTION = action('desired-speed', 'desired_speed');
export const HELM_HEADING_ACTION = action('desired-heading', 'desired_heading');
export const HELM_MATCH_ACTION = action('match-speed', 'match_speed');
export const HELM_FOLLOW_NAVIGATION_ACTION = action('follow-navigation', 'follow_navigation');
export const HELM_MOVEMENT_ACTIONS = Object.freeze([HELM_MODE_ACTION, HELM_SPEED_ACTION, HELM_HEADING_ACTION, HELM_MATCH_ACTION, HELM_FOLLOW_NAVIGATION_ACTION]);

export function registerHelmMovementActions(registry, { getState = () => null, sendAction } = {}) {
  const send = createCorrelatedActionSender(sendAction);
  const current = detail => {
    const state = getState(), projected = familyView(state || {}, 'helm');
    const view = Object.keys(projected).length ? projected : state;
    const movement = view?.helm_movement;
    const axis = movement?.axes?.find(axis => axis.system_id === detail.systemId && axis.controllable === true);
    return axis ? { axis, movement } : null;
  };
  for (const [definition, name, field, role] of [
    [HELM_MODE_ACTION, 'set_helm_mode', 'mode', null],
    [HELM_SPEED_ACTION, 'set_helm_desired_speed', 'speed', 'thrust'],
    [HELM_HEADING_ACTION, 'set_helm_desired_heading', 'heading_rad', 'steering'],
    [HELM_MATCH_ACTION, 'set_helm_match_speed', 'contact', 'thrust'],
  ]) registry.register(definition, ({ actionId, correlation, inputMs, detail } = {}) => {
    const held = current(detail || {});
    if (!held || (role && held.axis.role !== role)) return false;
    const value = detail[field];
    if (field === 'mode' && !['assisted', 'direct'].includes(value)) return false;
    if (['speed', 'heading_rad'].includes(field) && (typeof value !== 'number' || !Number.isFinite(Math.fround(value)))) return false;
    if (field === 'contact' && value !== null && !held.movement.contacts.some(row => row.contact === value)) return false;
    return send(actionId, correlation, inputMs, name,
      { control_system_id: held.axis.system_id, [field]: value });
  });
  registry.register(HELM_FOLLOW_NAVIGATION_ACTION, ({actionId,correlation,inputMs,detail}={})=>{
    const held=current(detail||{});if(!held)return false;
    const generation=detail.generation;
    if(generation===null ? !Number.isSafeInteger(held.axis.following_generation)||held.axis.following_generation<=0
      : !Number.isSafeInteger(generation)||generation<=0||generation!==held.movement.navigation_generation||held.axis.navigation_available!==true||held.axis.available!==true)return false;
    return send(actionId,correlation,inputMs,'set_helm_follow_navigation',{control_system_id:held.axis.system_id,generation});
  });
  return registry;
}
