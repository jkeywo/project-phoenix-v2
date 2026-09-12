/** Standalone browser Authoring adapter. Only user-selected pack bytes enter;
 * no filesystem provider, GM connection, simulation or live-state capture.
 */
import { WorkshopDocument } from '../editor/workshop-document.js';
import { createModActionRegistry, MOD_ACTION_CONTEXT, MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION_ID, MOD_EXPORT_ACTION_ID } from '../editor/mod-actions.js';
import { ACTION_FEEDBACK_STATE, ActionFeedbackLifecycle, emitActionFeedbackTransition } from './action-feedback.js';
import { applyAccessibilityProfile } from './accessibility-profile.js';
import { loadOperatorProfile, applyOperatorProfile, saveOperatorProfile } from './operator-profile.js';
import { createSemanticControlsRemapper } from './semantic-controls-remapper.js';
import { t } from './strings.js';

export function mountWorkshopAuthoring({ root, win = window, download = downloadZip } = {}) {
  const doc = root.ownerDocument;
  function el(tag, textId, attrs = {}) {
    const node = doc.createElement(tag);
    if (textId) node.textContent = t(textId);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    return node;
  }
  function button(textId, id, callback) {
    const node = el('button', textId, { id, type: 'button' });
    node.addEventListener('click', callback);
    return node;
  }
  root.replaceChildren();
  root.append(el('h1', 'workshop.title'), el('p', 'workshop.authoring', { class: 'workshop-mode' }),
    el('p', 'workshop.scope'));
  const toolbar = el('div', null, { class: 'workshop-toolbar' });
  const fileInput = el('input', null, { type: 'file', accept: '.zip,application/zip', hidden: '' });
  const activate = id => actions.activate(id, { context: MOD_ACTION_CONTEXT, source: 'control' });
  const importButton = button('editor.mod.import.button', 'workshop-import', () => activate(MOD_IMPORT_ACTION_ID));
  const undoButton = button('workshop.undo', 'workshop-undo', () => travel(false));
  const redoButton = button('workshop.redo', 'workshop-redo', () => travel(true));
  const checkButton = button('workshop.check', 'workshop-check', () => activate(MOD_VALIDATE_ACTION_ID));
  const exportButton = button('editor.mod.export.button', 'workshop-export', () => activate(MOD_EXPORT_ACTION_ID));
  const dirty = el('span', null, { role: 'status', id: 'workshop-dirty' });
  toolbar.append(importButton, undoButton, redoButton, checkButton, exportButton, dirty, fileInput);
  const layout = el('div', null, { class: 'workshop-layout' });
  const filesPanel = el('div', null, { class: 'workshop-files' });
  const filesLabel = el('label', 'workshop.files', { for: 'workshop-files' });
  const files = el('select', null, { id: 'workshop-files' });
  filesPanel.append(filesLabel, files);
  const sourcePanel = el('div', null, { class: 'workshop-source' });
  const sourceLabel = el('label', 'workshop.source', { for: 'workshop-source' });
  const source = el('textarea', null, { id: 'workshop-source', spellcheck: 'false', 'aria-describedby': 'workshop-source-hint' });
  sourcePanel.append(sourceLabel, source, el('p', 'workshop.source_hint', { id: 'workshop-source-hint' }));
  layout.append(filesPanel, sourcePanel);
  const feedback = el('div', null, { class: 'workshop-feedback', 'aria-live': 'polite' });
  const findings = el('div', null, { class: 'workshop-findings', role: 'status', tabindex: '-1' });
  const settings = el('details');
  settings.append(el('summary', 'editor.mod.settings.heading'));
  const settingsBody = el('div');
  settings.append(settingsBody);
  root.append(toolbar, layout, feedback, findings, settings);
  let draft = null;
  let selected = null;
  let pendingImport = null;
  let disposed = false;
  const feedbackRows = new Map();
  const lifecycle = new ActionFeedbackLifecycle({ onTransition(value) {
    emitActionFeedbackTransition(win, value);
    if (!value.isCurrent) return;
    if (value.cancelled || !value.state) feedbackRows.delete(value.actionId);
    else feedbackRows.set(value.actionId, value);
    feedback.replaceChildren(...[...feedbackRows.values()].map(entry => {
      const row = el('span', null, { 'data-action-id': entry.actionId, 'data-state': entry.state });
      row.textContent = t('action_feedback.summary', {
        action: t(actions.action(entry.actionId).labelId), status: t(entry.statusId),
      });
      return row;
    }));
  } });
  const actions = createModActionRegistry({
    actionFeedback: lifecycle,
    openImport(activation) {
      if (pendingImport) return false;
      if (draft?.isDirty() && !win.confirm(t('workshop.replace_confirm'))) return false;
      pendingImport = activation;
      fileInput.value = '';
      try { fileInput.click(); }
      catch (error) {
        pendingImport = null;
        show('editor.mod.import.previous_workspace_preserved', [String(error.message)], true);
        activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
      }
      return true;
    },
    validatePack: activation => evaluate(false, activation),
    exportPack: activation => evaluate(true, activation),
  });
  let storage = null;
  try { storage = win.localStorage; } catch { /* private mode */ }
  const loaded = loadOperatorProfile(storage, { registry: actions });
  let profile = loaded.profile;
  applyOperatorProfile(profile, actions);
  applyAccessibilityProfile(profile.accessibility, { doc, win });

  function persistBindings(result) {
    if (result.status !== 'applied') return result;
    profile = { ...profile, bindings: actions.bindingProfile() };
    const saved = saveOperatorProfile(storage, profile);
    if (saved.status !== 'saved') show('editor.mod.settings.storage_refused', [], true);
    return result;
  }
  const controls = createSemanticControlsRemapper({
    doc, root: settings,
    setBinding: (...args) => persistBindings(actions.setBinding(...args)),
    resetAction: id => persistBindings(actions.resetAction(id)),
    resetAll: () => persistBindings(actions.resetAllBindings()),
    rebuild: renderSettings,
  });
  function renderSettings() {
    settingsBody.replaceChildren();
    controls.render(settingsBody, {
      actions: actions.list(MOD_ACTION_CONTEXT),
      section(id) { const section = el('section'); section.append(el('h2', id)); return section; },
      hint: id => el('p', id),
      row: className => el('div', null, { class: className }),
      action(label, callback) {
        const control = button(null, '', callback); control.textContent = label; return control;
      },
    });
  }
  renderSettings();

  function show(id, details = [], refused = false) {
    findings.textContent = [t(id), ...details].join('\n');
    findings.setAttribute('role', refused ? 'alert' : 'status');
    findings.dataset.outcome = refused ? 'refused' : 'applied';
    if (refused) findings.focus();
  }
  function refresh({ selection = false } = {}) {
    if (selection) {
      files.replaceChildren(...(draft?.paths() || []).map(path => {
        const option = el('option', null, { value: path }); option.textContent = path; return option;
      }));
      files.value = selected || '';
      source.value = draft?.read(selected) || '';
      sourceLabel.textContent = selected ? t('workshop.source_path', { path: selected }) : t('workshop.source');
    }
    const busy = Boolean(pendingImport);
    files.disabled = !draft || busy;
    source.disabled = !draft || busy;
    checkButton.disabled = !draft || busy;
    exportButton.disabled = !draft || busy;
    importButton.disabled = busy;
    undoButton.disabled = !draft?.canUndo() || busy;
    redoButton.disabled = !draft?.canRedo() || busy;
    dirty.textContent = t(!draft ? 'workshop.empty' : draft.isDirty() ? 'workshop.dirty' : 'workshop.saved');
  }
  function travel(redo) {
    if (pendingImport) return;
    const path = redo ? draft?.redo() : draft?.undo();
    if (!path) return;
    selected = path;
    refresh({ selection: true });
    show(redo ? 'workshop.redone' : 'workshop.undone', [path]);
  }
  function evaluate(exporting, activation) {
    if (!draft || pendingImport) return false;
    const result = draft.check();
    if (!result.ok) {
      show('workshop.check_refused', result.errors, true);
      activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
      return true;
    }
    if (exporting) {
      try { download(result.zip, `${result.packId || 'mod-pack'}.zip`, doc, win); }
      catch (error) {
        show('editor.mod.export.download_refused', [String(error.message)], true);
        activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
        return true;
      }
      draft.markExported();
    }
    show(exporting ? 'workshop.exported' : 'workshop.checked', result.warnings);
    activation.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    refresh();
    exportButton.focus();
    return true;
  }
  fileInput.addEventListener('change', async () => {
    const file = fileInput.files?.[0];
    const pending = pendingImport;
    if (!file || !pending) { cancelImport(); return; }
    refresh();
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (disposed) return;
      const replacement = new WorkshopDocument(bytes);
      draft = replacement;
      selected = draft.paths()[0];
      show('workshop.imported');
      pending.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    } catch (error) {
      show('editor.mod.import.previous_workspace_preserved', [String(error.message)], true);
      pending.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
    } finally {
      pendingImport = null;
      fileInput.value = '';
      if (!disposed) refresh({ selection: true });
    }
  });
  function cancelImport() {
    pendingImport?.cancelFeedback();
    pendingImport = null;
    fileInput.value = '';
    refresh();
    importButton.focus();
  }
  fileInput.addEventListener('cancel', cancelImport);
  files.addEventListener('change', () => { selected = files.value; refresh({ selection: true }); });
  source.addEventListener('input', () => {
    if (draft?.edit(selected, source.value)) {
      findings.textContent = t('workshop.changed');
      findings.setAttribute('role', 'status');
      delete findings.dataset.outcome;
      refresh();
    }
  });
  function keydown(event) {
    if (event.defaultPrevented || event.isComposing) return;
    const editable = event.target?.matches?.('input, textarea, select') || event.target?.isContentEditable;
    // A saved binding wins over conventional history shortcuts. Text fields
    // keep the same typing exclusion as every other semantic-action surface.
    if (!editable && actions.dispatchKeyboardEvent(event, MOD_ACTION_CONTEXT).claimed) return;
    const historyKey = (event.ctrlKey || event.metaKey) && !event.altKey
      && (event.code === 'KeyZ' || event.code === 'KeyY');
    if (historyKey && !settings.contains(event.target)) {
      event.preventDefault();
      travel(event.code === 'KeyY' || event.shiftKey);
      return;
    }
  }
  function beforeUnload(event) {
    if (draft?.isDirty()) { event.preventDefault(); event.returnValue = ''; }
  }
  doc.addEventListener('keydown', keydown);
  win.addEventListener('beforeunload', beforeUnload);
  refresh({ selection: true });
  show('workshop.start');
  return { dispose() {
    disposed = true;
    controls.destroy();
    doc.removeEventListener('keydown', keydown);
    win.removeEventListener('beforeunload', beforeUnload);
  } };
}

function downloadZip(bytes, filename, doc, win) {
  const url = win.URL.createObjectURL(new win.Blob([bytes], { type: 'application/zip' }));
  const link = doc.createElement('a');
  link.href = url; link.download = filename;
  doc.body.append(link);
  try { link.click(); }
  finally { link.remove(); win.setTimeout(() => win.URL.revokeObjectURL(url), 0); }
}
