/**
 * faction-form-view.js
 *
 * Centre pane of the Definitions Mode → Faction section. Renders a small
 * form for the currently open faction:
 *
 *   uuid     — read-only label (never edited; identity is path-bound).
 *   name     — single-line text input.
 *   enemies  — <select multiple> over other factions, displayed by NAME
 *              with `option.value = uuid` (AC3).
 *
 * Callbacks emit canonical values:
 *   onNameChange(newName: string)
 *   onEnemiesChange(uuidList: string[])
 */

export function renderFactionFormView(host, { formState, enemyOptions, onNameChange, onEnemiesChange }) {
  if (!host) return;
  host.innerHTML = '';

  if (!formState) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'Select a faction file from the list.';
    host.appendChild(p);
    return;
  }

  const form = document.createElement('div');
  form.className = 'faction-form';
  host.appendChild(form);

  // ── UUID (read-only) ──────────────────────────────────────────────────
  const uuidRow = document.createElement('div');
  uuidRow.className = 'def-form-row';
  form.appendChild(uuidRow);

  const uuidLabel = document.createElement('label');
  uuidLabel.textContent = 'UUID';
  uuidRow.appendChild(uuidLabel);

  const uuidValue = document.createElement('span');
  uuidValue.className = 'def-uuid-readonly';
  uuidValue.textContent = formState.uuid;
  uuidRow.appendChild(uuidValue);

  // ── Name (text) ───────────────────────────────────────────────────────
  const nameRow = document.createElement('div');
  nameRow.className = 'def-form-row';
  form.appendChild(nameRow);

  const nameLabel = document.createElement('label');
  nameLabel.textContent = 'Name';
  nameRow.appendChild(nameLabel);

  const nameInput = document.createElement('input');
  nameInput.type = 'text';
  nameInput.className = 'def-name-input';
  nameInput.value = formState.name;
  nameInput.addEventListener('input', (e) => {
    if (onNameChange) onNameChange(e.target.value);
  });
  nameRow.appendChild(nameInput);

  // ── Enemies (multi-select, displays NAMES) ────────────────────────────
  const enemiesRow = document.createElement('div');
  enemiesRow.className = 'def-form-row';
  form.appendChild(enemiesRow);

  const enemiesLabel = document.createElement('label');
  enemiesLabel.textContent = 'Enemies';
  enemiesRow.appendChild(enemiesLabel);

  const select = document.createElement('select');
  select.multiple = true;
  select.className = 'def-multi-select';
  const selectedUuids = new Set(formState.enemies || []);
  const opts = Array.isArray(enemyOptions) ? enemyOptions : [];
  for (const opt of opts) {
    const optionEl = document.createElement('option');
    optionEl.value = opt.uuid;
    optionEl.textContent = opt.name;
    optionEl.selected = selectedUuids.has(opt.uuid);
    select.appendChild(optionEl);
  }

  select.addEventListener('change', () => {
    // Collect every option currently flagged `selected`. The FakeElement
    // shim has no native HTMLSelectElement.selectedOptions, so iterate
    // children explicitly.
    const uuids = [];
    for (const child of select.children) {
      if (child && child.selected) uuids.push(child.value);
    }
    if (onEnemiesChange) onEnemiesChange(uuids);
  });

  enemiesRow.appendChild(select);
}
