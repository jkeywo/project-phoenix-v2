import { defineStationAction, createCorrelatedActionSender } from './action-support.js';
import { familyView, familySystemId } from '../console-payload.js';
export const PRODUCTION_ACTION_IDS = Object.freeze({ enabled: 'production.enabled', target: 'production.target', recipe: 'production.recipe' });
export const PRODUCTION_ACTIONS = Object.freeze(Object.entries(PRODUCTION_ACTION_IDS).map(([name, id]) => defineStationAction({ id,
  contexts: ['engineering', 'production', 'captain'], labelKey: 'production.action.' + name, bindings: [null, null] })));
export function registerProductionActions(registry, options = {}) {
  const send = createCorrelatedActionSender(options.sendAction);
  for (const [i, name] of Object.keys(PRODUCTION_ACTION_IDS).entries()) {
    if (registry.action(PRODUCTION_ACTIONS[i].id)) continue;
    registry.register(PRODUCTION_ACTIONS[i], ({ actionId, correlation, inputMs, detail } = {}) => {
      const state = options.getState?.(), view = familyView(state, 'production'), owner = view.system_id || familySystemId(state, 'production');
      if (!owner || view.production_auto || view.production_controllable === false || !detail) return false;
      let payload;
      if (name === 'enabled' && typeof detail.enabled === 'boolean') payload = { enabled: detail.enabled };
      else {
        const recipe = name === 'recipe' ? detail.recipe : view.selected_recipe;
        const amount = name === 'recipe' ? detail.target : detail.amount;
        const definition = view.recipes?.find(row => row.id === recipe);
        if (!Number.isSafeInteger(amount) || amount < 0 || (recipe == null ? amount !== 0 : !definition || amount > definition.ceiling)) return false;
        if (name === 'recipe') payload = { recipe, target: amount }; else if (name === 'target') payload = { amount };
      }
      return payload ? send(actionId, correlation, inputMs, 'production_' + name, { ...payload, control_system_id: owner }) : false;
    });
  }
  return registry;
}
