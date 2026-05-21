/**
 * entity-behaviour-view.js
 *
 * DOM renderer for the [behaviour] component card. Wraps `BehaviourEditor`
 * to provide:
 *   - initial_state <select> over getStateNames()
 *   - State list: each row = name + kind dropdown + raw parameters textarea.
 *   - Transition list: from-multiselect (state names), to-select,
 *     condition.kind-select.
 *   - Inline validation banner from editor.validate().
 *
 * `onEdit(behaviourData)` is invoked on every mutation with the current
 * `editor.toBehaviour()` value, so the caller can snapshot + setSection.
 */

import { BehaviourEditor } from './behaviour-editor.js';

const STATE_KINDS = [
  'idle', 'patrol', 'attack',
  'patrolling', 'pursuing', 'attacking', 'fleeing', 'warping_out',
];

const CONDITION_KINDS = [
  'on_attacked', 'enemy_in_range', 'in_weapons_range',
  'target_destroyed', 'hull_below', 'on_timer', 'on_scenario_unloaded',
];

export function renderEntityBehaviourView(host, behaviour, { onEdit }) {
  if (!host) return;
  host.innerHTML = '';

  const editor = new BehaviourEditor();
  editor.load(behaviour || {});

  const rerender = () => renderEntityBehaviourView(host, editor.toBehaviour(), { onEdit });

  const mutate = (fn) => {
    fn(editor);
    onEdit(editor.toBehaviour());
    rerender();
  };

  const root = document.createElement('div');
  root.className = 'entity-behaviour';
  host.appendChild(root);

  // Validation banner.
  const validation = editor.validate();
  if (!validation.valid) {
    const banner = document.createElement('div');
    banner.className = 'entity-behaviour-error';
    banner.textContent = validation.errors.join(' • ');
    root.appendChild(banner);
  }

  // initial_state select.
  const initialRow = document.createElement('div');
  initialRow.className = 'entity-behaviour-initial';
  const initialLabel = document.createElement('label');
  initialLabel.textContent = 'initial_state';
  initialRow.appendChild(initialLabel);
  const initialSelect = document.createElement('select');
  initialSelect.className = 'entity-behaviour-initial-select';
  const blank = document.createElement('option');
  blank.value = '';
  blank.textContent = '(none)';
  initialSelect.appendChild(blank);
  for (const name of editor.getStateNames()) {
    const o = document.createElement('option');
    o.value = name;
    o.textContent = name;
    initialSelect.appendChild(o);
  }
  initialSelect.value = editor.getInitialState() || '';
  initialSelect.addEventListener('change', (e) => {
    mutate((ed) => ed.setInitialState(e.target.value || null));
  });
  initialRow.appendChild(initialSelect);
  root.appendChild(initialRow);

  // States section.
  const statesH = document.createElement('h5');
  statesH.textContent = 'States';
  root.appendChild(statesH);

  const statesList = document.createElement('div');
  statesList.className = 'entity-behaviour-states';
  root.appendChild(statesList);

  for (const s of editor.getStates()) {
    statesList.appendChild(renderStateRow(s, mutate));
  }

  const addStateBtn = document.createElement('button');
  addStateBtn.type = 'button';
  addStateBtn.className = 'entity-behaviour-add-state';
  addStateBtn.textContent = '+ Add State';
  addStateBtn.addEventListener('click', () => {
    mutate((ed) => {
      const baseName = `state_${ed.getStates().length + 1}`;
      ed.addState(baseName, 'idle');
    });
  });
  root.appendChild(addStateBtn);

  // Transitions.
  const transH = document.createElement('h5');
  transH.textContent = 'Transitions';
  root.appendChild(transH);

  const transList = document.createElement('div');
  transList.className = 'entity-behaviour-transitions';
  root.appendChild(transList);

  const stateNames = editor.getStateNames();
  editor.getTransitions().forEach((t, idx) => {
    transList.appendChild(renderTransitionRow(t, idx, stateNames, mutate));
  });

  const addTransBtn = document.createElement('button');
  addTransBtn.type = 'button';
  addTransBtn.className = 'entity-behaviour-add-transition';
  addTransBtn.textContent = '+ Add Transition';
  addTransBtn.addEventListener('click', () => {
    mutate((ed) => {
      const names = ed.getStateNames();
      ed.addTransition(
        names.length > 0 ? [names[0]] : [],
        names.length > 0 ? names[0] : '',
        { kind: CONDITION_KINDS[0], parameters: {} },
      );
    });
  });
  root.appendChild(addTransBtn);
}

function renderStateRow(state, mutate) {
  const row = document.createElement('div');
  row.className = 'entity-behaviour-state-row';

  const nameInput = document.createElement('input');
  nameInput.type = 'text';
  nameInput.className = 'entity-behaviour-state-name';
  nameInput.value = state.name;
  nameInput.disabled = true; // rename via remove+add to keep things simple
  row.appendChild(nameInput);

  const kindSelect = document.createElement('select');
  kindSelect.className = 'entity-behaviour-state-kind';
  for (const k of STATE_KINDS) {
    const o = document.createElement('option');
    o.value = k;
    o.textContent = k;
    kindSelect.appendChild(o);
  }
  if (!STATE_KINDS.includes(state.kind) && state.kind) {
    const o = document.createElement('option');
    o.value = state.kind;
    o.textContent = `${state.kind} (custom)`;
    kindSelect.appendChild(o);
  }
  kindSelect.value = state.kind || STATE_KINDS[0];
  kindSelect.addEventListener('change', (e) => {
    mutate((ed) => ed.updateState(state.name, { kind: e.target.value }));
  });
  row.appendChild(kindSelect);

  const removeBtn = document.createElement('button');
  removeBtn.type = 'button';
  removeBtn.className = 'entity-behaviour-state-remove';
  removeBtn.textContent = '✕';
  removeBtn.addEventListener('click', () => {
    mutate((ed) => ed.removeState(state.name));
  });
  row.appendChild(removeBtn);

  return row;
}

function renderTransitionRow(t, idx, stateNames, mutate) {
  const row = document.createElement('div');
  row.className = 'entity-behaviour-transition-row';

  // from multi-select (rendered as a select with multiple).
  const fromSelect = document.createElement('select');
  fromSelect.className = 'entity-behaviour-transition-from';
  fromSelect.multiple = true;
  for (const name of stateNames) {
    const o = document.createElement('option');
    o.value = name;
    o.textContent = name;
    if ((t.from || []).includes(name)) o.selected = true;
    fromSelect.appendChild(o);
  }
  fromSelect.addEventListener('change', (e) => {
    const opts = e.target.children || [];
    const selected = Array.from(opts).filter((o) => o.selected).map((o) => o.value);
    mutate((ed) => ed.updateTransition(idx, { from: selected }));
  });
  row.appendChild(fromSelect);

  // to select.
  const toSelect = document.createElement('select');
  toSelect.className = 'entity-behaviour-transition-to';
  for (const name of stateNames) {
    const o = document.createElement('option');
    o.value = name;
    o.textContent = name;
    toSelect.appendChild(o);
  }
  toSelect.value = t.to || '';
  toSelect.addEventListener('change', (e) => {
    mutate((ed) => ed.updateTransition(idx, { to: e.target.value }));
  });
  row.appendChild(toSelect);

  // condition kind select.
  const condSelect = document.createElement('select');
  condSelect.className = 'entity-behaviour-transition-condition';
  for (const k of CONDITION_KINDS) {
    const o = document.createElement('option');
    o.value = k;
    o.textContent = k;
    condSelect.appendChild(o);
  }
  const cKind = t.condition?.kind;
  if (cKind && !CONDITION_KINDS.includes(cKind)) {
    const o = document.createElement('option');
    o.value = cKind;
    o.textContent = `${cKind} (custom)`;
    condSelect.appendChild(o);
  }
  condSelect.value = cKind || CONDITION_KINDS[0];
  condSelect.addEventListener('change', (e) => {
    mutate((ed) => ed.updateTransition(idx, {
      condition: { kind: e.target.value, parameters: t.condition?.parameters || {} },
    }));
  });
  row.appendChild(condSelect);

  // remove button.
  const removeBtn = document.createElement('button');
  removeBtn.type = 'button';
  removeBtn.className = 'entity-behaviour-transition-remove';
  removeBtn.textContent = '✕';
  removeBtn.addEventListener('click', () => {
    mutate((ed) => ed.removeTransition(idx));
  });
  row.appendChild(removeBtn);

  return row;
}
