import { t, wireText } from '../strings.js';
import { FITTING_ACTION_IDS } from '../stations/fitting-actions.js';

/** Retains the native pickers while paid work and status snapshots advance. */
export function mountFittingControls(doc) {
  const node = doc.createElement('div'), entries = new Map();
  const make = tag => doc.createElement(tag);
  const fill = (select, values) => {
    const current = select.value, ids = values.map(row => String(row.id));
    if (JSON.stringify(ids) !== select.dataset.options) {
      select.dataset.options = JSON.stringify(ids); select.replaceChildren();
      for (const row of values) { const option = make('option'); option.value = row.id; select.append(option); }
      if (ids.includes(current)) select.value = current;
    }
    for (const [index, value] of values.entries()) select.options[index].textContent = value.label;
  };
  function add(key) {
    const field = make('fieldset'), heading = make('legend'), info = make('p'), forecast = make('p'), phase = make('p');
    const equipment = make('select'), team = make('select'), request = make('button'), dispatch = make('button'), cancel = make('button');
    const label = (control, key) => { const label = make('label'); label.textContent = t('fitting.' + key); label.append(control); return label; };
    for (const [button, name] of [[request, 'request'], [dispatch, 'dispatch'], [cancel, 'cancel']]) { button.type = 'button'; button.textContent = t('fitting.' + name); }
    field.dataset.fittingSlot = key; field.append(heading, info, label(equipment, 'equipment'), forecast, request, label(team, 'team'), dispatch, cancel, phase); node.append(field);
    const entry = { field, heading, info, forecast, phase, equipment, team, request, dispatch, cancel };
    const activate = (name, extra = {}) => typeof window !== 'undefined' && window.activateSemanticAction?.(FITTING_ACTION_IDS[name], { context: 'fitting', source: 'control', detail: { owner: entry.owner, slot: entry.view.slot, ...extra } });
    request.addEventListener('click', () => activate('request', { equipment: equipment.value }));
    dispatch.addEventListener('click', () => activate('dispatch', { team_idx: Number(team.value) }));
    cancel.addEventListener('click', () => activate('cancel'));
    const paintForecast = () => {
      const choice = entry.view?.choices?.find(choice => choice.equipment === equipment.value);
      forecast.textContent = choice && ['beam', 'reactor'].includes(choice.family)
        ? t('fitting.forecast.' + choice.family, choice.statistics || {}) + ' ' + t('fitting.forecast.assumptions') : '';
      request.disabled = !entry.editable || !entry.view.can_request || !choice;
    };
    equipment.addEventListener('change', paintForecast); entry.paintForecast = paintForecast;
    for (const picker of [equipment, team]) picker.addEventListener('keydown', event => { if (!['Tab', 'Escape'].includes(event.key)) event.stopPropagation(); });
    return entry;
  }
  return { node, update(owner, slots = [], delegated = false) {
    const seen = new Set();
    for (const view of slots) {
      const key = JSON.stringify([owner, view.slot]); seen.add(key);
      let entry = entries.get(key); if (!entry) { entry = add(key); entries.set(key, entry); }
      entry.owner = owner; entry.view = view; entry.editable = !delegated;
      entry.heading.textContent = wireText(view.slot);
      entry.info.textContent = t('fitting.installed', { installed: view.installed ? wireText(view.installed) : t('fitting.vacant'),
        desired: view.desired ? wireText(view.desired) : t('fitting.none') });
      const oldChoice = entry.equipment.value;
      fill(entry.equipment, (view.choices || []).map(choice => ({ id: choice.equipment,
        label: t('fitting.choice', { equipment: wireText(choice.equipment), count: choice.available }) + (choice.facility_required ? ' · ' + t('fitting.facility-required') : '') })));
      if (!oldChoice) entry.equipment.value = view.desired || view.installed || entry.equipment.value;
      fill(entry.team, (view.free_teams || []).map(index => ({ id: index, label: t('fitting.team-number', { number: index + 1 }) })));
      entry.equipment.disabled = delegated || !view.can_request;
      entry.team.disabled = delegated || !view.can_dispatch || !view.free_teams?.length;
      entry.dispatch.disabled = entry.team.disabled || view.phase !== 'idle';
      entry.cancel.disabled = delegated || !view.can_dispatch || (!view.desired && view.phase === 'idle');
      entry.request.hidden = entry.equipment.hidden = delegated || !view.can_request;
      entry.dispatch.hidden = entry.team.hidden = entry.cancel.hidden = delegated || !view.can_dispatch;
      entry.phase.textContent = t('fitting.progress', { phase: t('fitting.phase.' + view.phase), progress: Math.round((view.progress || 0) / 10) });
      entry.paintForecast();
    }
    for (const [key, entry] of entries) if (!seen.has(key)) { entry.field.remove(); entries.delete(key); }
    node.hidden = !slots.length;
  }, dispose() { entries.clear(); node.remove(); } };
}
