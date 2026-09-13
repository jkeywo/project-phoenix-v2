/** Standalone browser Authoring adapter. Only user-selected pack bytes enter;
 * no filesystem provider, GM connection, simulation or live-state capture.
 */
import { WorkshopDocument, isWorkshopBinary } from '../editor/workshop-document.js';
import { newWorkshopPack } from '../editor/workshop-provider.js';
import { createWorkshopRuntime } from '../editor/workshop-runtime.js';
import { createWorkshopRecovery } from '../editor/workshop-recovery.js';
import { mountWorkshopTestPanel } from './workshop-test-panel.js';
import { mountWorkshopSoundCues } from '../editor/workshop-sound-cues.js';
import { mountWorkshopModels } from './workshop-models-panel.js';
import { createModActionRegistry, MOD_ACTION_CONTEXT, MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION_ID, MOD_EXPORT_ACTION_ID } from '../editor/mod-actions.js';
import { ACTION_FEEDBACK_STATE, ActionFeedbackLifecycle, emitActionFeedbackTransition } from './action-feedback.js';
import { applyAccessibilityProfile } from './accessibility-profile.js';
import { loadOperatorProfile, applyOperatorProfile, saveOperatorProfile } from './operator-profile.js';
import { createSemanticControlsRemapper } from './semantic-controls-remapper.js';
import { t } from './strings.js';

// wasm-bindgen may reject with a string JsValue rather than an Error object.
const errorText = error => String(error?.message ?? error);
const NATIVE_COPY = {
  'workshop.authoring': 'workshop.native_authoring', 'workshop.files': 'workshop.native_files',
  'workshop.empty': 'workshop.native_empty', 'workshop.dirty': 'workshop.native_dirty',
  'workshop.saved': 'workshop.native_clean', 'workshop.changed': 'workshop.native_changed',
  'workshop.start': 'workshop.native_start',
  'workshop.recovery_loading': 'workshop.native_recovery_loading',
  'workshop.recovery_saving': 'workshop.native_recovery_saving',
  'workshop.recovery_saved': 'workshop.native_recovery_saved',
  'workshop.recovery_failed': 'workshop.native_recovery_failed',
  'workshop.recovery_restored': 'workshop.native_recovery_restored',
  'workshop.recovery_discarded': 'workshop.native_recovery_discarded',
  'workshop.recovery_empty': 'workshop.native_recovery_empty',
  'workshop.recovery_available': 'workshop.native_recovery_available',
  'workshop.recovery_invalid': 'workshop.native_recovery_invalid',
};

export function mountWorkshopAuthoring({ root, win = window, download = downloadZip, provider = null,
  runtime = provider?.runtime || createWorkshopRuntime(), recovery = provider?.recovery || createWorkshopRecovery({ indexedDB: win.indexedDB }) } = {}) {
  const doc = root.ownerDocument;
  const translate = (id, params) => t(provider?.save ? (NATIVE_COPY[id] || id) : id, params);
  function el(tag, textId, attrs = {}) {
    const node = doc.createElement(tag);
    if (textId) node.textContent = translate(textId);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    return node;
  }
  function button(textId, id, callback) {
    const node = el('button', textId, { id, type: 'button' });
    node.addEventListener('click', callback);
    return node;
  }
  root.replaceChildren();
  const sourceScope = el('p', provider?.save ? 'workshop.native_scope' : 'workshop.scope');
  root.append(el('h1', 'workshop.title'), el('p', 'workshop.authoring', { class: 'workshop-mode' }), sourceScope);
  const toolbar = el('div', null, { class: 'workshop-toolbar' });
  const fileInput = el('input', null, { type: 'file', accept: '.zip,application/zip', hidden: '' });
  const activate = id => actions.activate(id, { context: MOD_ACTION_CONTEXT, source: 'control' });
  const importButton = button('editor.mod.import.button', 'workshop-import', () => activate(MOD_IMPORT_ACTION_ID));
  const newButton = button('workshop.new', 'workshop-new', () => createPack());
  const saveButton = button('workshop.save', 'workshop-save', () => saveNative());
  saveButton.hidden = !provider?.save;
  newButton.hidden = provider?.canCreate === false;
  importButton.hidden = provider?.canImport === false;
  const undoButton = button('workshop.undo', 'workshop-undo', () => travel(false));
  const redoButton = button('workshop.redo', 'workshop-redo', () => travel(true));
  const checkButton = button('workshop.check', 'workshop-check', () => activate(MOD_VALIDATE_ACTION_ID));
  const exportButton = button('editor.mod.export.button', 'workshop-export', () => activate(MOD_EXPORT_ACTION_ID));
  const dirty = el('span', null, { role: 'status', id: 'workshop-dirty' });
  exportButton.hidden = Boolean(provider?.save);
  toolbar.append(newButton, importButton, undoButton, redoButton, checkButton, saveButton, exportButton, dirty, fileInput);
  const layout = el('div', null, { class: 'workshop-layout' });
  const filesPanel = el('div', null, { class: 'workshop-files' });
  const filesLabel = el('label', 'workshop.files', { for: 'workshop-files' });
  const files = el('select', null, { id: 'workshop-files' });
  filesPanel.append(filesLabel, files);
  const addPath = el('input', null, { id: 'workshop-add-path', type: 'text' });
  const assetInput = el('input', null, { type: 'file', hidden: '' });
  const addSource = button('workshop.add_source', 'workshop-add-source', () => addDocument());
  const addAsset = button('workshop.add_asset', 'workshop-add-asset', () => assetInput.click());
  filesPanel.append(el('label', 'workshop.add_path', { for: 'workshop-add-path' }), addPath, addSource, addAsset, assetInput);
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
  const dependencies = el('details');
  dependencies.append(el('summary', 'workshop.dependencies'));
  const dependencySelect = el('select', null, { id: 'workshop-dependency' });
  const dependencySource = el('textarea', null, { id: 'workshop-dependency-source', readonly: '', rows: '8' });
  const dependencyButton = button('workshop.dependencies_load', 'workshop-dependencies-load', () => loadDependencies());
  dependencies.append(dependencyButton, el('label', 'workshop.files', { for: 'workshop-dependency' }), dependencySelect,
    el('label', 'workshop.dependency_source', { for: 'workshop-dependency-source' }), dependencySource);
  dependencies.hidden = !runtime.dependencies;
  root.append(dependencies);
  let dependencyFiles = [];
  let draft = null;
  let selected = null;
  let pendingImport = null;
  let pendingValidation = false;
  let inspected = null;
  let disposed = false;
  let pendingRecovery = true;
  let recoveredDraft = null;
  let persistenceGeneration = 0;
  let testPanel = null;
  let soundAudition = null;
  let modelPanel = null;
  const feedbackRows = new Map();
  const lifecycle = new ActionFeedbackLifecycle({ onTransition(value) {
    emitActionFeedbackTransition(win, value);
    if (!value.isCurrent) return;
    if (value.cancelled || !value.state) feedbackRows.delete(value.actionId);
    else feedbackRows.set(value.actionId, value);
    feedback.replaceChildren(...[...feedbackRows.values()].map(entry => {
      const row = el('span', null, { 'data-action-id': entry.actionId, 'data-state': entry.state });
      row.textContent = translate('action_feedback.summary', {
        action: translate(actions.action(entry.actionId).labelId), status: translate(entry.statusId),
      });
      return row;
    }));
  } });
  const actions = createModActionRegistry({
    actionFeedback: lifecycle,
    openImport(activation) {
      if (provider?.canImport === false || pendingImport || pendingValidation || pendingRecovery || testPanel?.held()) return false;
      if (draft?.isDirty() && !win.confirm(translate('workshop.replace_confirm'))) return false;
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
  try { storage = win.PhoenixOperatorStorage || win.localStorage; } catch { /* private mode */ }
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
  function nativeStorageStatus() {
    if (win.PhoenixOperatorStorageStatus?.status === 'error') show('editor.mod.settings.storage_refused', [], true);
  }
  win.addEventListener('phoenix-operator-storage-status', nativeStorageStatus);

  function show(id, details = [], refused = false) {
    findings.textContent = [translate(id), ...details].join('\n');
    findings.setAttribute('role', refused ? 'alert' : 'status');
    findings.dataset.outcome = refused ? 'refused' : 'applied';
    if (refused) findings.focus();
  }
  function refresh({ selection = false } = {}) {
    soundAudition?.refresh({held:!!(pendingImport || pendingValidation || pendingRecovery || testPanel?.held())});
    if (selection) {
      files.replaceChildren(...(draft?.paths() || []).map(path => {
        const option = el('option', null, { value: path }); option.textContent = path; return option;
      }));
      files.value = selected || '';
      source.value = draft?.isBinary(selected) ? translate('workshop.binary_source', { bytes: draft.byteLength(selected) }) : draft?.read(selected) || '';
      sourceLabel.textContent = selected ? translate('workshop.source_path', { path: selected }) : translate('workshop.source');
    }
    const busy = Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held());
    const testing = testPanel?.testing() || false;
    modelPanel?.refresh({ hidden: testing });
    toolbar.hidden = recoveryPanel.hidden = layout.hidden = feedback.hidden = findings.hidden = testing;
    sourceScope.hidden = testing;
    dependencies.hidden = testing || !runtime.dependencies;
    root.querySelector('.workshop-mode').textContent = translate(testing ? 'workshop.test_mode' : 'workshop.authoring');
    files.disabled = !draft || busy;
    source.disabled = !draft || busy || draft.isBinary(selected);
    checkButton.disabled = !draft || busy;
    exportButton.disabled = !draft || busy;
    importButton.disabled = busy;
    newButton.disabled = busy;
    saveButton.disabled = !draft || busy;
    addPath.disabled = addSource.disabled = addAsset.disabled = !draft || busy;
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
      fieldInfo.textContent = translate('workshop.inspector_stale');
    }
    dirty.textContent = translate(!draft ? 'workshop.empty' : draft.isDirty() ? 'workshop.dirty' : 'workshop.saved');
    testPanel?.refresh();
  }
  function travel(redo) {
    if (pendingImport || pendingValidation || pendingRecovery || testPanel?.held()) return;
    const path = redo ? draft?.redo() : draft?.undo();
    if (!path) return;
    selected = draft.paths().includes(path) ? path : draft.paths()[0];
    refresh({ selection: true });
    show(redo ? 'workshop.redone' : 'workshop.undone', [path]);
    persistDraft();
  }
  async function inspect() {
    if (!draft || pendingImport || pendingValidation || pendingRecovery || testPanel?.held()) return;
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
    fieldInfo.textContent = field ? translate(field.runtime_owned ? 'workshop.field_runtime' : 'workshop.field_fallback', {
      type: field.kind, line: String(field.line),
    }) : translate('workshop.inspector_empty');
    if (field?.default_source != null) fieldInfo.textContent += ` ${translate('workshop.field_default', { value: field.default_source })}`;
  }
  fieldSelect.addEventListener('change', renderField);
  async function patchField() {
    if (!inspected || pendingValidation || pendingImport || testPanel?.held()) return;
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
      if (draft.read(snapshot.path) !== snapshot.source) throw new Error(translate('workshop.inspector_stale'));
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
    if (exporting && provider?.save) return false;
    if (!draft || pendingImport || pendingValidation || pendingRecovery || testPanel?.held()) return false;
    pendingValidation = true;
    const candidate = draft;
    show('workshop.runtime_checking');
    refresh();
    void (async () => {
      try {
        const zip = provider?.save ? null : candidate.archive();
        const result = await runtime.validate(zip, candidate);
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
      row.append(doc.createTextNode(` — ${translate(`workshop.severity.${record.severity}`)}: ${record.message}`));
      findings.append(row);
    }
  }
  async function createPack() {
    if (pendingImport || pendingValidation || pendingRecovery || testPanel?.held() || provider?.canCreate === false) return;
    if (draft?.isDirty() && !win.confirm(translate('workshop.replace_confirm'))) return;
    pendingValidation = true; refresh();
    try {
      const replacement = newWorkshopPack(await runtime.dependencies());
      if (disposed) return;
      draft = replacement; selected = draft.paths()[0];
      show('workshop.created'); persistDraft();
    } catch (error) { if (!disposed) show('workshop.runtime_unavailable', [errorText(error)], true); }
    finally { pendingValidation = false; if (!disposed) refresh({ selection: true }); }
  }
  async function saveNative() {
    if (!provider?.save || !draft || pendingValidation || pendingRecovery || pendingImport || testPanel?.held()) return;
    pendingValidation = true; refresh();
    try {
      await provider.save(draft);
      if (disposed) return;
      draft.markExported(); persistDraft(); show('workshop.native_saved');
    } catch (error) {
      if (disposed) return;
      if (error.report) showRuntimeFindings('workshop.check_refused', error.report.findings, true);
      else show('workshop.native_save_refused', [errorText(error)], true);
    } finally { pendingValidation = false; if (!disposed) refresh(); }
  }
  function addDocument(value = '') {
    if (!draft || pendingValidation || pendingRecovery || pendingImport || testPanel?.held()) return;
    const path = addPath.value.trim();
    if (!path.startsWith('assets/') || (!/\.(toml|rhai)$/.test(path) && !isWorkshopBinary(path))) {
      show('workshop.add_refused', [], true); return;
    }
    if (draft.paths().includes(path) && !win.confirm(translate('workshop.replace_file_confirm', { path }))) return;
    try {
      if (draft.put(path, value)) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); }
    } catch (error) { show('workshop.add_refused', [errorText(error)], true); }
  }
  assetInput.addEventListener('change', async () => {
    const file = assetInput.files?.[0];
    if (!file || !draft || pendingValidation || pendingRecovery || pendingImport || testPanel?.held()) return;
    const target = draft;
    const path = addPath.value.trim();
    pendingValidation = true; refresh();
    try {
      const nativeAsset = isWorkshopBinary(path) && provider?.importAsset;
      const bytes = nativeAsset ? await provider.importAsset(file) : new Uint8Array(await file.arrayBuffer());
      if (disposed || draft !== target) return;
      pendingValidation = false;
      addPath.value = path;
      addDocument(isWorkshopBinary(path) ? bytes : new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes));
    } catch (error) { if (!disposed) show('workshop.add_refused', [errorText(error)], true); }
    finally { pendingValidation = false; assetInput.value = ''; if (!disposed) refresh(); }
  });
  async function loadDependencies() {
    dependencyButton.disabled = true;
    try {
      const snapshot = await runtime.dependencies();
      if (disposed) return;
      dependencyFiles = [['base', snapshot.base_files], ...snapshot.packs.map(pack => [pack.id, pack.files])]
        .flatMap(([label, files]) => Object.entries(files).map(([path, text]) => ({ label: `${label}: ${path}`, text })));
      dependencySelect.replaceChildren(...dependencyFiles.map((entry, index) => {
        const option = el('option', null, { value: index }); option.textContent = entry.label; return option;
      }));
      dependencySource.value = dependencyFiles[0]?.text || '';
    } catch (error) { if (!disposed) show('workshop.runtime_unavailable', [errorText(error)], true); }
    finally { dependencyButton.disabled = false; }
  }
  dependencySelect.addEventListener('change', () => { dependencySource.value = dependencyFiles[Number(dependencySelect.value)]?.text || ''; });
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
        error?.code === 'workshop-missing-manifest' ? translate('workshop.missing_manifest') : errorText(error),
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
  files.addEventListener('change', () => {
    if (testPanel?.held()) { refresh({ selection: true }); return; }
    selected = files.value; refresh({ selection: true });
  });
  source.addEventListener('input', () => {
    if (testPanel?.held()) { refresh({ selection: true }); return; }
    if (pendingImport || pendingValidation || pendingRecovery || testPanel?.held()) return;
    if (draft?.edit(selected, source.value)) {
      findings.textContent = translate('workshop.changed');
      findings.setAttribute('role', 'status');
      delete findings.dataset.outcome;
      refresh();
      persistDraft();
    }
  });
  function persistDraft() {
    if (!draft || pendingRecovery) return;
    const generation = ++persistenceGeneration;
    recoveryStatus.textContent = translate('workshop.recovery_saving');
    void recovery.save({ version: 1, selected, draft: draft.snapshot() }).then(() => {
      if (!disposed && generation === persistenceGeneration) recoveryStatus.textContent = translate('workshop.recovery_saved');
    }, () => {
      if (!disposed && generation === persistenceGeneration) recoveryStatus.textContent = translate('workshop.recovery_failed');
    });
  }
  function restoreDraft() {
    if (!recoveredDraft) return;
    try { provider?.restore?.(recoveredDraft.record); }
    catch (error) { show('workshop.recovery_invalid', [errorText(error)], true); return; }
    draft = recoveredDraft.draft;
    selected = recoveredDraft.selected;
    recoveredDraft = null;
    pendingRecovery = false;
    restoreButton.hidden = discardButton.hidden = true;
    recoveryStatus.textContent = translate('workshop.recovery_restored');
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
      recoveryStatus.textContent = translate('workshop.recovery_discarded');
      refresh();
      importButton.focus();
    } catch {
      if (!disposed) recoveryStatus.textContent = translate('workshop.recovery_failed');
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
  soundAudition = mountWorkshopSoundCues({root,win,draft:()=>draft,native:!!provider?.save,
    resolveAsset:path=>runtime.readAsset?.(path)??null,
    readAudio:()=>profile.audio,saveAudio:audio=>{profile={...profile,audio};return saveOperatorProfile(win.PhoenixOperatorStorage||win.localStorage,profile);}});
  modelPanel = mountWorkshopModels({ root, provider, runtime, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); } });
  win.addEventListener('beforeunload', beforeUnload);
  testPanel = mountWorkshopTestPanel({ root, provider, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery), changed: () => refresh(), win });
  refresh({ selection: true });
  show('workshop.start');
  nativeStorageStatus();
  const ready = (async () => {
    let record;
    if (provider?.load) {
      try {
        const loaded = await provider.load();
        if (disposed) return;
        if (loaded) { draft = loaded; selected = draft.paths()[0]; refresh({ selection: true }); }
      } catch (error) {
        if (!disposed) { pendingRecovery = false; show('workshop.native_load_refused', [errorText(error)], true); refresh(); }
        return;
      }
    }
    try { record = await recovery.load(); }
    catch {
      if (!disposed) {
        pendingRecovery = false;
        recoveryStatus.textContent = translate('workshop.recovery_failed');
        refresh();
      }
      return;
    }
    if (disposed) return;
    if (!record) {
      pendingRecovery = false;
      recoveryStatus.textContent = translate('workshop.recovery_empty');
      refresh();
      return;
    }
    discardButton.hidden = false;
    try {
      if (record.version !== 1) throw new Error('Unsupported recovery version');
      const restored = provider?.restoreDocument ? provider.restoreDocument(record.draft) : WorkshopDocument.restore(record.draft);
      recoveredDraft = { record, draft: restored, selected: restored.paths().includes(record.selected) ? record.selected : restored.paths()[0] };
      restoreButton.hidden = false;
      recoveryStatus.textContent = translate('workshop.recovery_available');
    } catch { recoveryStatus.textContent = translate('workshop.recovery_invalid'); }
  })();
  return { ready, dispose() {
    disposed = true;
    testPanel.dispose();
    soundAudition?.dispose();
    modelPanel?.dispose();
    controls.destroy();
    doc.removeEventListener('keydown', keydown);
    win.removeEventListener('beforeunload', beforeUnload);
    win.removeEventListener('phoenix-operator-storage-status', nativeStorageStatus);
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
