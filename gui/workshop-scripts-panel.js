import { runWorkshopMutation } from './workshop-edit-session.js';
import { mountScriptEditor, renderScriptList } from '../editor/script-editor-view.js';
import { applyWorkshopScript, createWorkshopScript, workshopScriptUnits, workshopScriptWorlds } from '../editor/workshop-scripts.js';
import { addressedModifierSnippet, upsertObjectiveInstanceSnippet } from '../editor/workshop-objective-snippet.js';
import { has, t } from './strings.js';

export function mountWorkshopScripts({ root, provider, runtime, draft, busy, setBusy, changed, attach = true }) {
  const doc = root.ownerDocument;
  const node = (tag, id, attrs = {}) => {
    const value = doc.createElement(tag); if (id) { value.dataset.i18n = id; value.textContent = t(id); }
    for (const [key, item] of Object.entries(attrs)) value.setAttribute(key, item);
    return value;
  };
  const section = node('section', null, { id: 'workshop-scripts', class: 'workshop-scripts' });
  const world = node('select', null, { id: 'workshop-script-world' });
  const create = node('button', 'workshop.scripts.create', { id: 'workshop-script-create', type: 'button' });
  const list = node('div', null, { id: 'workshop-script-list', role: 'list', 'aria-label': t('workshop.scripts.units') });
  const editor = node('div', null, { id: 'workshop-script-editor' });
  const status = node('p', null, { id: 'workshop-script-status', role: 'status', tabindex: '-1' });
  const scope = node('p', 'workshop.scripts.scope');
  const worldLabel = node('label', 'workshop.scripts.world', { for: world.id });
  const objective = node('fieldset', null, { id: 'workshop-objective-instance' });
  const objectiveId = node('input', null, { id: 'workshop-objective-id', type: 'text' });
  const instanceId = node('input', null, { id: 'workshop-objective-instance-id', type: 'text' });
  const textId = node('input', null, { id: 'workshop-objective-text', type: 'text' });
  const shipSlots = node('input', null, { id: 'workshop-objective-slots', type: 'text' });
  const factions = node('input', null, { id: 'workshop-objective-factions', type: 'text' });
  const allShips = node('input', null, { id: 'workshop-objective-all', type: 'checkbox' });
  const delay = node('input', null, { id: 'workshop-objective-delay', type: 'number', min: '1', step: '1' });
  const insertObjective = node('button', 'workshop.objective.insert', { id: 'workshop-objective-insert', type: 'button' });
  const action = node('fieldset', null, { id: 'workshop-addressed-action' });
  const actionKind = node('select', null, { id: 'workshop-action-kind' });
  actionKind.append(optionFor('apply_modifier', 'workshop.objective.action_apply'),
    optionFor('remove_modifier', 'workshop.objective.action_remove'));
  const actionSlot = node('input', null, { id: 'workshop-action-slot', type: 'text' });
  const actionTag = node('input', null, { id: 'workshop-action-tag', type: 'text' });
  const actionBonus = node('input', null, { id: 'workshop-action-bonus', type: 'number', step: 'any' });
  const instanceMembers = node('input', null, { id: 'workshop-action-members', type: 'checkbox' });
  const insertAction = node('button', 'workshop.objective.action_insert', { id: 'workshop-action-insert', type: 'button' });
  function optionFor(value, labelId) { const item = node('option', labelId, { value }); return item; }
  const field = (control, labelId) => { const label = node('label', labelId, { for: control.id });
    objective.append(label, control); };
  objective.append(node('legend', 'workshop.objective.title'), node('p', 'workshop.objective.scope'));
  field(objectiveId, 'workshop.objective.id'); field(instanceId, 'workshop.objective.instance');
  field(textId, 'workshop.objective.text'); field(shipSlots, 'workshop.objective.slots');
  field(factions, 'workshop.objective.factions'); field(allShips, 'workshop.objective.all');
  field(delay, 'workshop.objective.delay'); objective.append(insertObjective);
  const actionField = (control, labelId) => { const label = node('label', labelId, { for: control.id });
    action.append(label, control); };
  action.append(node('legend', 'workshop.objective.action_title'), node('p', 'workshop.objective.action_scope'));
  actionField(actionKind, 'workshop.objective.action_kind'); actionField(actionSlot, 'workshop.objective.action_slot');
  actionField(actionTag, 'workshop.objective.action_tag'); actionField(actionBonus, 'workshop.objective.action_bonus');
  actionField(instanceMembers, 'workshop.objective.action_members'); action.append(insertAction);
  section.append(scope, worldLabel,
    world, create, list, objective, action, status, editor);
  if (attach) root.append(section);
  let disposed = false, controller = null, hostFns = null, active = null;
  let previousDraft = null, previousRevision = -1, previousWorld = null;
  const option = (value, label) => { const item = node('option', null, { value }); item.textContent = label; return item; };
  const show = (id, error = false, detail = '') => {
    // Runtime/compiler text is source detail, never a String Id. Keep a
    // translated outcome around it so a refusal has meaning in every locale.
    const messageId = has(id) ? id : 'workshop.scripts.operation_refused';
    const literalDetail = has(id) ? detail : [id, detail].filter(Boolean).join(' ');
    status.dataset.messageId = messageId;
    status.dataset.detail = literalDetail;
    status.textContent = [t(messageId), literalDetail].filter(Boolean).join(' ');
    status.setAttribute('role', error ? 'alert' : 'status'); if (error) status.focus();
  };
  create.addEventListener('click', async () => {
    if (busy() || !world.value) return;
    const target = draft(), path = world.value;
    setBusy(true);
    try {
      await createWorkshopScript({ draft: target, provider, runtime, worldPath: path,
        current: () => !disposed && draft() === target && world.value === path });
      if (!disposed) { changed(path); setBusy(false); refresh(); await open(units()[0]); show('workshop.scripts.created'); }
    } catch (error) {
      if (!disposed) show(error?.message || 'workshop.scripts.validation_refused', true,
        error?.report?.findings?.map(row => `${row.file}${row.line ? `:${row.line}` : ''}: ${row.message}`).join(' ') || '');
    } finally { if (!disposed) { setBusy(false); refresh(); } }
  });
  const csv = value => value.split(',').map(item => item.trim()).filter(Boolean);
  insertObjective.addEventListener('click', () => {
    if (!controller || busy()) { show('workshop.objective.error_open', true); return; }
    try {
      const source = controller.getSource();
      const next = upsertObjectiveInstanceSnippet({ id: objectiveId.value.trim(),
        instanceId: instanceId.value.trim(), text: textId.value.trim(),
        shipSlots: csv(shipSlots.value), factions: csv(factions.value), allPlayerShips: allShips.checked,
        completeAfterSeconds: delay.value.trim() ? Number(delay.value) : null }, source);
      controller.setSource(next);
      controller.textarea.focus();
      show('workshop.objective.inserted');
    } catch (error) { show(error?.message || 'workshop.objective.error_fields', true); }
  });
  insertAction.addEventListener('click', () => {
    if (!controller || busy()) { show('workshop.objective.error_open', true); return; }
    try {
      const source = controller.getSource();
      const snippet = addressedModifierSnippet({ kind: actionKind.value, slot: actionSlot.value,
        tag: actionTag.value, bonus: Number(actionBonus.value), shipSlots: csv(shipSlots.value),
        factions: csv(factions.value), allPlayerShips: allShips.checked,
        objectiveId: objectiveId.value, instanceId: instanceId.value,
        instanceMembers: instanceMembers.checked }, source);
      controller.setSource(source + (source.endsWith('\n') ? '\n' : '\n\n') + snippet + '\n');
      controller.textarea.focus();
      show('workshop.objective.inserted');
    } catch (error) { show(error?.message || 'workshop.objective.error_action', true); }
  });
  async function open(unit) {
    if (!unit || busy()) return;
    active = unit;
    if (hostFns == null) {
      try { hostFns = await runtime.scriptHostFunctions(); }
      catch { hostFns = []; }
    }
    if (disposed || busy() || !currentUnit(unit)) return;
    controller?.destroy();
    controller = mountScriptEditor({ host: editor, source: unit.source,
      t,
      title: `${unit.documentPath}${unit.kind === 'inline' ? ` — [script.${unit.key}]` : ''}`,
      hostFns, lineOffset: unit.lineOffset,
      getDiagnostics: async (source, lineOffset) => {
        const revision = draft()?.sourceRevision;
        const results = await runtime.scriptDiagnostics(source, lineOffset);
        if (disposed || draft()?.sourceRevision !== revision || active?.id !== unit.id) return [];
        return results.map(row => ({ ...row, file: unit.documentPath, revision }));
      },
      isDiagnosticsAvailable: () => typeof runtime.scriptDiagnostics === 'function',
      onSave: source => void save(unit, source),
    });
    paintList();
    refresh();
  }
  function currentUnit(unit) {
    return workshopScriptUnits(draft(), world.value).some(row => row.id === unit.id
      && row.documentPath === unit.documentPath && row.source === unit.source);
  }
  async function save(unit, source) {
    if (busy() || !currentUnit(unit)) { show('workshop.scripts.stale', true); return; }
    const target = draft();
    return runWorkshopMutation({ setBusy, current: () => !disposed, successCurrent: () => true,
      invoke: () => applyWorkshopScript({ draft: target, provider, runtime, unit, source,
        current: () => !disposed && draft() === target && active?.id === unit.id && currentUnit(unit) }),
      success: edited => {
        if (edited) { changed(unit.documentPath); show('workshop.scripts.applied'); }
        else show('workshop.scripts.unchanged');
      },
      error: error => show(error?.message || 'workshop.scripts.validation_refused', true,
        error?.report?.findings?.map(row => `${row.file}${row.line ? `:${row.line}` : ''}: ${row.message}`).join(' ') || ''),
      release: refresh,
    });
  }
  function units() { return workshopScriptUnits(draft(), world.value); }
  function paintList() {
    const focused = list.contains(doc.activeElement) ? doc.activeElement.closest('[data-script-id]')?.dataset.scriptId : null;
    const rows = units();
    renderScriptList(list, rows, { selectedId: active?.id, onSelect: open, t });
    for (const row of list.querySelectorAll('.script-list-row')) row.setAttribute('role', 'button');
    if (focused) [...list.querySelectorAll('[data-script-id]')]
      .find(row => row.dataset.scriptId === focused)?.focus();
  }
  function refresh({ hidden = false } = {}) {
    section.hidden = hidden;
    const current = draft(), revision = current?.sourceRevision ?? -1;
    if (current !== previousDraft || revision !== previousRevision) {
      const old = world.value || previousWorld;
      const worlds = workshopScriptWorlds(current);
      world.replaceChildren(...worlds.map(row => option(row.path, row.path)));
      if (worlds.some(row => row.path === old)) world.value = old;
      previousDraft = current; previousRevision = revision; previousWorld = world.value;
      if (active && !currentUnit(active)) { active = null; controller?.destroy(); controller = null; }
      paintList();
    }
    world.disabled = hidden || busy() || !world.options.length;
    create.disabled = hidden || busy() || !world.value || units().length > 0;
    for (const input of section.querySelectorAll('#workshop-objective-instance input, #workshop-objective-instance button, #workshop-addressed-action input, #workshop-addressed-action select, #workshop-addressed-action button')) input.disabled = hidden || busy() || !controller;
    actionBonus.disabled = hidden || busy() || !controller || actionKind.value !== 'apply_modifier';
    controller?.textarea && (controller.textarea.disabled = hidden || busy());
  }
  actionKind.addEventListener('change', () => refresh());
  world.addEventListener('change', () => { previousWorld = world.value; active = null; controller?.destroy(); controller = null; paintList(); refresh(); });
  refresh();
  return { node: section, refresh,
    refreshLanguage() {
      for (const item of section.querySelectorAll('[data-i18n]')) item.textContent = t(item.dataset.i18n);
      list.setAttribute('aria-label', t('workshop.scripts.units'));
      if (status.dataset.messageId) status.textContent = [t(status.dataset.messageId), status.dataset.detail].filter(Boolean).join(' ');
      paintList();
      controller?.refreshLanguage();
    },
    dispose() { disposed = true; controller?.destroy(); section.remove(); } };
}
