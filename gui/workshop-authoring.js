/** Standalone browser Authoring adapter. Only user-selected pack bytes enter;
 * no filesystem provider, GM connection, simulation or live-state capture.
 */
import { WorkshopDocument } from '../editor/workshop-document.js';
import { createWorkshopRuntime } from '../editor/workshop-runtime.js';
import { createWorkshopRecovery } from '../editor/workshop-recovery.js';
import { createModActionRegistry, MOD_ACTION_CONTEXT, MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION_ID, MOD_EXPORT_ACTION_ID } from '../editor/mod-actions.js';
import { ACTION_FEEDBACK_STATE, ActionFeedbackLifecycle, emitActionFeedbackTransition } from './action-feedback.js';
import { applyAccessibilityProfile } from './accessibility-profile.js';
import { loadOperatorProfile, applyOperatorProfile, saveOperatorProfile } from './operator-profile.js';
import { createSemanticControlsRemapper } from './semantic-controls-remapper.js';
import { t } from './strings.js';

// wasm-bindgen may reject with a string JsValue rather than an Error object.
const errorText = error => String(error?.message ?? error);

export function mountWorkshopAuthoring({ root, win = window, download = downloadZip,
  runtime = createWorkshopRuntime(), recovery = createWorkshopRecovery({ indexedDB: win.indexedDB }) } = {}) {
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
  const inspector = el('details', null, { class: 'workshop-inspector' });
  inspector.append(el('summary', 'workshop.inspector'));
  const inspectButton = button('workshop.inspect', 'workshop-inspect', () => inspect());
  const fieldSelect = el('select', null, { id: 'workshop-field' });
  const fieldValue = el('textarea', null, { id: 'workshop-field-value', rows: '3', spellcheck: 'false' });
  const fieldInfo = el('p', null, { id: 'workshop-field-info' });
  const applyField = button('workshop.apply_field', 'workshop-apply-field', () => patchField());
  inspector.append(inspectButton, el('label', 'workshop.field', { for: 'workshop-field' }), fieldSelect,
    fieldInfo, el('label', 'workshop.field_value', { for: 'workshop-field-value' }), fieldValue, applyField);
  filesPanel.append(inspector);
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
  const recoveryPanel = el('section', null, { class: 'workshop-recovery', 'aria-live': 'polite' });
  const recoveryStatus = el('p', 'workshop.recovery_loading', { id: 'workshop-recovery-status' });
  const restoreButton = button('workshop.recovery_restore', 'workshop-restore', () => restoreDraft());
  const discardButton = button('workshop.recovery_discard', 'workshop-discard', () => discardRecovery());
  restoreButton.hidden = discardButton.hidden = true;
  recoveryPanel.append(recoveryStatus, restoreButton, discardButton);
  root.append(toolbar, recoveryPanel, layout, feedback, findings, settings);
  let draft = null;
  let selected = null;
  let pendingImport = null;
  let pendingValidation = false;
  let inspected = null;
  let disposed = false;
  let pendingRecovery = true;
  let recoveredDraft = null;
  let persistenceGeneration = 0;
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
      if (pendingImport || pendingValidation || pendingRecovery) return false;
      if (draft?.isDirty() && !win.confirm(t('workshop.replace_confirm'))) return false;
      pendingImport = activation;
      fileInput.value = '';
      try { fileInput.click(); }
      catch (error) {
        pendingImport = null;
        show('editor.mod.import.previous_workspace_preserved', [errorText(error)], true);
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
    const busy = Boolean(pendingImport || pendingValidation || pendingRecovery);
    files.disabled = !draft || busy;
    source.disabled = !draft || busy;
    checkButton.disabled = !draft || busy;
    exportButton.disabled = !draft || busy;
    importButton.disabled = busy;
    undoButton.disabled = !draft?.canUndo() || busy;
    redoButton.disabled = !draft?.canRedo() || busy;
    inspectButton.disabled = !draft || busy || !selected?.endsWith('.toml');
    const currentInspector = inspected && inspected.path === selected && inspected.source === draft?.read(selected);
    fieldSelect.disabled = !currentInspector || busy;
    fieldValue.disabled = !currentInspector || busy;
    applyField.disabled = !currentInspector || busy || !inspected.fields.length;
    if (!currentInspector) {
      inspected = null;
      fieldSelect.replaceChildren();
      fieldValue.value = '';
      fieldInfo.textContent = t('workshop.inspector_stale');
    }
    dirty.textContent = t(!draft ? 'workshop.empty' : draft.isDirty() ? 'workshop.dirty' : 'workshop.saved');
  }
  function travel(redo) {
    if (pendingImport || pendingValidation || pendingRecovery) return;
    const path = redo ? draft?.redo() : draft?.undo();
    if (!path) return;
    selected = path;
    refresh({ selection: true });
    show(redo ? 'workshop.redone' : 'workshop.undone', [path]);
    persistDraft();
  }
  async function inspect() {
    if (!draft || pendingImport || pendingValidation || pendingRecovery) return;
    const candidate = draft;
    const path = selected;
    const text = candidate.read(path);
    pendingValidation = true;
    refresh();
    try {
      const fields = await runtime.inspect(text, path);
      if (disposed || draft !== candidate || selected !== path || draft.read(path) !== text) return;
      inspected = { path, source: text, fields };
      fieldSelect.replaceChildren(...fields.map((field, index) => {
        const label = field.path.map(part => typeof part === 'number' ? `[${part}]` : part).join('.');
        const option = el('option', null, { value: String(index) });
        option.textContent = label;
        return option;
      }));
      renderField();
    } catch (error) {
      if (!disposed) show('workshop.inspector_refused', [errorText(error)], true);
    } finally { pendingValidation = false; if (!disposed) refresh(); }
  }
  function renderField() {
    const field = inspected?.fields[Number(fieldSelect.value)];
    fieldValue.value = field?.source || '';
    fieldInfo.textContent = field ? t(field.runtime_owned ? 'workshop.field_runtime' : 'workshop.field_fallback', {
      type: field.kind, line: String(field.line),
    }) : t('workshop.inspector_empty');
    if (field?.default_source != null) fieldInfo.textContent += ` ${t('workshop.field_default', { value: field.default_source })}`;
  }
  fieldSelect.addEventListener('change', renderField);
  async function patchField() {
    if (!inspected || pendingValidation || pendingImport) return;
    const snapshot = inspected;
    const field = snapshot.fields[Number(fieldSelect.value)];
    if (!field) return;
    pendingValidation = true;
    refresh();
    try {
      const patched = await runtime.patch(draft.read(snapshot.path), {
        document_path: snapshot.path, path: field.path, expected_source: snapshot.source, value_source: fieldValue.value,
      });
      if (disposed) return;
      if (draft.read(snapshot.path) !== snapshot.source) throw new Error(t('workshop.inspector_stale'));
      if (draft.edit(snapshot.path, patched)) {
        selected = snapshot.path;
        refresh({ selection: true });
        show('workshop.changed');
        persistDraft();
      }
    } catch (error) {
      if (!disposed) show('workshop.inspector_refused', [errorText(error)], true);
    } finally { pendingValidation = false; if (!disposed) refresh(); }
  }
  function evaluate(exporting, activation) {
    if (!draft || pendingImport || pendingValidation || pendingRecovery) return false;
    pendingValidation = true;
    const candidate = draft;
    show('workshop.runtime_checking');
    refresh();
    void (async () => {
      try {
        const zip = candidate.archive();
        const result = await runtime.validate(zip);
        if (disposed) return;
        if (!result.accepted) {
          showRuntimeFindings('workshop.check_refused', result.findings, true);
          activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
          return;
        }
        if (exporting) {
          try { download(zip, 'mod-pack.zip', doc, win); }
          catch (error) {
            show('editor.mod.export.download_refused', [errorText(error)], true);
            activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
            return;
          }
          candidate.markExported();
          persistDraft();
        }
        showRuntimeFindings(exporting ? 'workshop.exported' : 'workshop.runtime_checked', result.findings);
        activation.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
      } catch (error) {
        if (!disposed) {
          show('workshop.runtime_unavailable', [errorText(error)], true);
          activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
        }
      } finally {
        pendingValidation = false;
        if (!disposed) refresh();
      }
    })();
    return true;
  }
  function showRuntimeFindings(title, records, refused = false) {
    show(title, [], refused);
    for (const record of records) {
      const row = el('p');
      const location = `${record.file}${record.line ? `:${record.line}` : ''}`;
      if (draft.paths().includes(record.file)) {
        const target = button(null, '', () => {
          selected = record.file;
          refresh({ selection: true });
          source.focus();
          const text = source.value;
          const lines = text.split('\n');
          const line = Math.max(0, Math.min(lines.length - 1, (record.line || 1) - 1));
          const start = lines.slice(0, line).reduce((total, part) => total + part.length + 1, 0);
          source.setSelectionRange(start, start + lines[line].length);
        });
        target.textContent = location;
        row.append(target);
      } else row.append(doc.createTextNode(location));
      row.append(doc.createTextNode(` — ${t(`workshop.severity.${record.severity}`)}: ${record.message}`));
      findings.append(row);
    }
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
      persistDraft();
      pending.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    } catch (error) {
      show('editor.mod.import.previous_workspace_preserved', [
        error?.code === 'workshop-missing-manifest' ? t('workshop.missing_manifest') : errorText(error),
      ], true);
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
    if (pendingImport || pendingValidation || pendingRecovery) return;
    if (draft?.edit(selected, source.value)) {
      findings.textContent = t('workshop.changed');
      findings.setAttribute('role', 'status');
      delete findings.dataset.outcome;
      refresh();
      persistDraft();
    }
  });
  function persistDraft() {
    if (!draft || pendingRecovery) return;
    const generation = ++persistenceGeneration;
    recoveryStatus.textContent = t('workshop.recovery_saving');
    void recovery.save({ version: 1, selected, draft: draft.snapshot() }).then(() => {
      if (!disposed && generation === persistenceGeneration) recoveryStatus.textContent = t('workshop.recovery_saved');
    }, () => {
      if (!disposed && generation === persistenceGeneration) recoveryStatus.textContent = t('workshop.recovery_failed');
    });
  }
  function restoreDraft() {
    if (!recoveredDraft) return;
    draft = recoveredDraft.draft;
    selected = recoveredDraft.selected;
    recoveredDraft = null;
    pendingRecovery = false;
    restoreButton.hidden = discardButton.hidden = true;
    recoveryStatus.textContent = t('workshop.recovery_restored');
    refresh({ selection: true });
    source.focus();
  }
  async function discardRecovery() {
    discardButton.disabled = restoreButton.disabled = true;
    try {
      await recovery.clear();
      if (disposed) return;
      recoveredDraft = null;
      pendingRecovery = false;
      restoreButton.hidden = discardButton.hidden = true;
      recoveryStatus.textContent = t('workshop.recovery_discarded');
      refresh();
      importButton.focus();
    } catch {
      if (!disposed) recoveryStatus.textContent = t('workshop.recovery_failed');
    } finally { discardButton.disabled = restoreButton.disabled = false; }
  }
  function keydown(event) {
    if (event.defaultPrevented || event.isComposing) return;
    const editable = event.target?.matches?.('input, textarea, select') || event.target?.isContentEditable;
    // A saved binding wins over conventional history shortcuts. Text fields
    // keep the same typing exclusion as every other semantic-action surface.
    if (!editable && actions.dispatchKeyboardEvent(event, MOD_ACTION_CONTEXT).claimed) return;
    const historyKey = (event.ctrlKey || event.metaKey) && !event.altKey
      && (event.code === 'KeyZ' || event.code === 'KeyY');
    if (historyKey && !settings.contains(event.target) && (!editable || event.target === source)) {
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
  const ready = (async () => {
    let record;
    try { record = await recovery.load(); }
    catch {
      if (!disposed) {
        pendingRecovery = false;
        recoveryStatus.textContent = t('workshop.recovery_failed');
        refresh();
      }
      return;
    }
    if (disposed) return;
    if (!record) {
      pendingRecovery = false;
      recoveryStatus.textContent = t('workshop.recovery_empty');
      refresh();
      return;
    }
    discardButton.hidden = false;
    try {
      if (record.version !== 1) throw new Error('Unsupported recovery version');
      const restored = WorkshopDocument.restore(record.draft);
      recoveredDraft = { draft: restored, selected: restored.paths().includes(record.selected) ? record.selected : restored.paths()[0] };
      restoreButton.hidden = false;
      recoveryStatus.textContent = t('workshop.recovery_available');
    } catch { recoveryStatus.textContent = t('workshop.recovery_invalid'); }
  })();
  return { ready, dispose() {
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
