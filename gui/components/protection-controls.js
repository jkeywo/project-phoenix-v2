import {t, wireText} from '../strings.js';
import {stationDisplayName} from '../console-state.js';
import {PROTECTION_ACTION_IDS} from '../stations/protection-actions.js';

/** An operator's finite ordered choices. Actual grants come only from the host. */
export function mountProtectionControls(doc) {
  const node = doc.createElement('fieldset'), title = doc.createElement('legend');
  node.dataset.signalProtection = ''; title.textContent = t('protection.title');
  const status = doc.createElement('p'), capacity = doc.createElement('p'), funded = doc.createElement('p');
  const choices = doc.createElement('select'), priorities = doc.createElement('select'), stations = doc.createElement('select');
  choices.setAttribute('aria-label', t('protection.compatible')); priorities.setAttribute('aria-label', t('protection.priorities'));
  priorities.size = 4; stations.setAttribute('aria-label', t('protection.station'));
  const button = (key, action) => {
    const control = doc.createElement('button'); control.type = 'button'; control.textContent = t(`protection.${key}`);
    control.dataset.protectionAction = key; control.onclick = () => {if (!control.disabled) action();}; return control;
  };
  let owner = null, view = null, readonly = true, draft = [], authoritative = '', choiceSignature = '';
  const emit = (kind, fields) => {
    if (!readonly) doc.defaultView.activateSemanticAction?.(PROTECTION_ACTION_IDS[kind], {
      context: 'protection', source: 'control', detail: {owner, ...fields},
    });
  };
  const add = button('add', () => {if (view.systems.some(row => row.system === choices.value) && !draft.includes(choices.value)) draft.push(choices.value); paint();});
  const remove = button('remove', () => {draft = draft.filter(id => id !== priorities.value); paint();});
  const move = direction => {
    const index = draft.indexOf(priorities.value), next = index + direction;
    if (index < 0 || next < 0 || next >= draft.length) return;
    const selected = draft[index]; [draft[index], draft[next]] = [draft[next], draft[index]]; paint(); priorities.value = selected; availability();
  };
  const up = button('up', () => move(-1)), down = button('down', () => move(1));
  const preset = button('preset', () => {draft = view.systems.filter(row => row.home_station === stations.value).map(row => row.system); paint();});
  const clear = button('clear', () => {draft = []; paint();});
  const apply = button('apply', () => emit('priorities', {systems: [...draft]}));
  const toggle = button('enable', () => emit('enabled', {enabled: !view.enabled}));
  const edits = [choices, priorities, stations, add, remove, up, down, preset, clear, apply, toggle];
  function availability() {
    const index = draft.indexOf(priorities.value);
    for (const control of edits) control.disabled = readonly;
    add.disabled ||= !view?.systems.some(row => row.system === choices.value) || draft.includes(choices.value);
    remove.disabled ||= index < 0; up.disabled ||= index <= 0; down.disabled ||= index < 0 || index >= draft.length - 1;
    preset.disabled ||= !stations.value; clear.disabled ||= !draft.length;
    apply.disabled ||= JSON.stringify(draft) === authoritative || draft.length > 64 || draft.some(id => !view?.systems.some(row => row.system === id));
  }
  function paint() {
    const selected = priorities.value;
    priorities.replaceChildren(...draft.map(id => {
      const option = doc.createElement('option'), row = view.systems.find(row => row.system === id);
      option.value = id; option.textContent = `${wireText(row?.display_name || id)} (${row?.cost ?? '?'})`; return option;
    }));
    if (draft.includes(selected)) priorities.value = selected;
    availability();
  }
  for (const select of [choices, priorities, stations]) {
    select.onchange = availability;
    select.addEventListener('keydown', event => {if (!['Tab', 'Escape'].includes(event.key)) event.stopPropagation();});
  }
  node.append(title, status, capacity, funded, choices, add, priorities, remove, up, down, stations, preset, clear, apply, toggle);
  return {node, update(system, next, delegated = false) {
    const changedOwner = owner !== system; owner = system; view = next; node.hidden = !next;
    if (!next) {readonly = true; for (const control of edits) control.disabled = true; return;}
    readonly = delegated || !next.controllable;
    const signature = JSON.stringify(next.priorities);
    if (changedOwner || signature !== authoritative) {authoritative = signature; draft = [...next.priorities];}
    const nextChoices = JSON.stringify(next.systems);
    if (changedOwner || nextChoices !== choiceSignature) {
      choiceSignature = nextChoices; const previous = choices.value;
      choices.replaceChildren(...next.systems.map(row => {const option = doc.createElement('option'); option.value = row.system; option.textContent = `${wireText(row.display_name)} (${row.cost})`; return option;}));
      if (next.systems.some(row => row.system === previous)) choices.value = previous;
      const ids = [...new Set(next.systems.map(row => row.home_station).filter(Boolean))];
      stations.replaceChildren(...ids.map(id => {const option = doc.createElement('option'); option.value = id; option.textContent = stationDisplayName(id); return option;}));
    }
    status.textContent = t(next.active ? 'protection.active' : next.enabled ? 'protection.waiting' : 'protection.off');
    capacity.textContent = t('protection.capacity', {capacity: next.capacity});
    funded.textContent = t('protection.funded', {systems: next.protected_systems.map(id => wireText(next.systems.find(row => row.system === id)?.display_name || id)).join(', ') || t('protection.none')});
    toggle.textContent = t(next.enabled ? 'protection.disable' : 'protection.enable');
    for (const control of edits) control.hidden = readonly;
    paint();
  }, dispose() {node.remove();}};
}
