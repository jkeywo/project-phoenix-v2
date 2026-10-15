import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView } from '../console-payload.js';

export const WEAPON_TARGET_ACTION_IDS = Object.freeze({ target: 'tactical.weapon-target', group: 'tactical.weapon-group-target',
  rejoin: 'tactical.follow-weapon-group', primary: 'tactical.follow-primary-target' });
export const WEAPON_TARGET_ACTIONS = Object.freeze(Object.entries(WEAPON_TARGET_ACTION_IDS).map(([name, id]) => defineStationAction({
  id, contexts: ['tactical'], labelKey: 'weapon_targets.' + name, bindings: [null, null],
})));
export function registerWeaponTargetActions(registry, options) {
  const send = createCorrelatedActionSender(options.sendAction);
  for (const [index, name] of Object.keys(WEAPON_TARGET_ACTION_IDS).entries()) {
    registry.register(WEAPON_TARGET_ACTIONS[index], ({ actionId, correlation, inputMs, detail } = {}) => {
      const state = options.getState?.(), family = familyView(state, 'tactical');
      const view = (Object.keys(family).length ? family : state)?.weapon_targets;
      if (!view || !detail) return false;
      const contact = detail.uuid === null ? null : view.contacts?.find(row => row.id === detail.uuid)?.id;
      if (['target', 'group'].includes(name) && contact === undefined) return false;
      const emit = (row, action, extra) => send(actionId, correlation, inputMs, action, { ...extra, control_system_id: row.system });
      if (name === 'group') {
        const group = view.groups?.find(group => group.id === detail.group);
        const row = view.rows?.find(row => row.system === detail.system && row.controllable && group?.members.includes(row.system));
        return row ? emit(row, 'set_weapon_group_target', { group: group.id, uuid: contact }) : false;
      }
      const row = view.rows?.find(row => row.system === detail.system && row.controllable);
      if (!row) return false;
      return emit(row, { target: 'set_weapon_target', rejoin: 'follow_weapon_group', primary: 'follow_primary_target' }[name],
        name === 'target' ? { uuid: contact } : {});
    });
  }
}
