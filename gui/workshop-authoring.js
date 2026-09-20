/** Standalone browser Authoring adapter. Only user-selected pack bytes enter;
 * no filesystem provider, GM connection, simulation or live-state capture.
 */
import { WorkshopDocument, isWorkshopBinary } from '../editor/workshop-document.js';
import { workspaceDiff, workspaceDiffIsEmpty } from '../editor/workshop-diff.js';
import { proposeWorkshopMigration } from '../editor/workshop-migration.js';
import { newWorkshopPack } from '../editor/workshop-provider.js';
import { parse as parseToml } from 'smol-toml';
import { createWorkshopRuntime } from '../editor/workshop-runtime.js';
import { createWorkshopRecovery } from '../editor/workshop-recovery.js';
import { mountWorkshopTestPanel } from './workshop-test-panel.js';
import { mountWorkshopSoundCues } from '../editor/workshop-sound-cues.js';
import { mountWorkshopModels } from './workshop-models-panel.js';
import { mountWorkshopDefinitions } from './workshop-definitions-panel.js';
import { mountWorkshopComposition } from './workshop-composition-panel.js';
import { mountWorkshopEntity } from './workshop-entity-panel.js';
import { mountWorkshopPresets } from './workshop-presets-panel.js';
import { mountWorkshopScripts } from './workshop-scripts-panel.js';
import { createModActionRegistry, MOD_ACTION_CONTEXT, MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION_ID, MOD_EXPORT_ACTION_ID } from '../editor/mod-actions.js';
import { ACTION_FEEDBACK_STATE, ActionFeedbackLifecycle, emitActionFeedbackTransition } from './action-feedback.js';
import { applyAccessibilityProfile, EXPLICIT_OFF, EXPLICIT_ON, FOLLOW_OS,
  profileWithPresentation, TEXT_SCALE_MAX, TEXT_SCALE_MIN, TEXT_SCALE_STEP } from './accessibility-profile.js';
import { loadOperatorProfile, applyOperatorProfile, saveOperatorProfile } from './operator-profile.js';
import { createSemanticControlsRemapper } from './semantic-controls-remapper.js';
import { t } from './strings.js';
import { renderInspectorMetadata, validInspectorDescriptor } from './inspector-field.js';
import { mountWorkshopLayout } from './workshop-layout-renderer.js';
import { workshopTestLayoutModel } from './workshop-test-layout-model.js';

// wasm-bindgen may reject with a string JsValue rather than an Error object.
const ERROR_STRING_IDS = Object.freeze({
  'native-workshop-dependencies-invalid': 'workshop.native_dependencies_invalid',
});
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
  const errorText = error => ERROR_STRING_IDS[error?.code]
    ? translate(ERROR_STRING_IDS[error.code]) : String(error?.message ?? error);
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
  let testWorkspace = false;
  let testHadRun = false;
  const openTestButton = button('workshop.test_heading', 'workshop-open-test', () => {
    testWorkspace = true;
    refresh();
    void testPanel?.enter().catch(() => {}).finally(refresh);
  });
  openTestButton.hidden = !provider?.test;
  saveButton.hidden = !provider?.save;
  newButton.hidden = provider?.canCreate === false;
  importButton.hidden = provider?.canImport === false;
  const undoButton = button('workshop.undo', 'workshop-undo', () => travel(false));
  const redoButton = button('workshop.redo', 'workshop-redo', () => travel(true));
  const checkButton = button('workshop.check', 'workshop-check', () => activate(MOD_VALIDATE_ACTION_ID));
  const exportButton = button('editor.mod.export.button', 'workshop-export', () => activate(MOD_EXPORT_ACTION_ID));
  const dirty = el('span', null, { role: 'status', id: 'workshop-dirty' });
  exportButton.hidden = Boolean(provider?.save);
  toolbar.append(newButton, importButton, undoButton, redoButton, checkButton, saveButton, exportButton,
    openTestButton, dirty, fileInput);
  const layout = el('div', null, { class: 'workshop-layout' });
  const testLayout = el('div', null, { class: 'workshop-test-layout' });
  testLayout.hidden = true;
  const filesPanel = el('div', null, { class: 'workshop-files' });
  const filesLabel = el('label', 'workshop.files', { for: 'workshop-files' });
  const files = el('select', null, { id: 'workshop-files' });
  filesPanel.append(filesLabel, files);
  const addPanel = el('div', null, { class: 'workshop-add' });
  const addPath = el('input', null, { id: 'workshop-add-path', type: 'text' });
  const assetInput = el('input', null, { type: 'file', hidden: '' });
  const addSource = button('workshop.add_source', 'workshop-add-source', () => addDocument());
  const addAsset = button('workshop.add_asset', 'workshop-add-asset', () => assetInput.click());
  // Rename and delete act on the SELECTED member and read the same path box the
  // add controls do: one place to say where a member should be, rather than a
  // second field that can disagree with the first.
  const renameButton = button('workshop.rename_file', 'workshop-rename', () => renameDocument());
  const deleteButton = button('workshop.delete_file', 'workshop-delete', () => deleteDocument());
  addPanel.append(el('label', 'workshop.add_path', { for: 'workshop-add-path' }), addPath, addSource, addAsset,
    renameButton, deleteButton, assetInput);
  // What this draft has done to the source it was imported as.
  const changesPanel = el('div', null, { class: 'workshop-changes' });
  const changesSummary = el('p', null, { id: 'workshop-changes-summary', role: 'status' });
  const changesList = el('ul', null, { id: 'workshop-changes-list' });
  // A proposed migration of older supported content: shown as the exact before
  // and after of the member it would rewrite, and applied only by a deliberate
  // press. Nothing here changes source on its own.
  const migrationPanel = el('div', null, { class: 'workshop-migration', id: 'workshop-migration' });
  const migrationSummary = el('p', null, { id: 'workshop-migration-summary', role: 'status' });
  const migrationBefore = el('textarea', null, { id: 'workshop-migration-before', rows: '4', readonly: '', spellcheck: 'false' });
  const migrationAfter = el('textarea', null, { id: 'workshop-migration-after', rows: '4', readonly: '', spellcheck: 'false' });
  const migrationAccept = button('workshop.migration.accept', 'workshop-migration-accept', () => acceptMigration());
  migrationPanel.append(migrationSummary,
    el('label', 'workshop.migration.before', { for: 'workshop-migration-before' }), migrationBefore,
    el('label', 'workshop.migration.after', { for: 'workshop-migration-after' }), migrationAfter,
    migrationAccept);
  migrationPanel.hidden = true;
  changesPanel.append(changesSummary, changesList, migrationPanel);
  const inspector = el('details', null, { class: 'workshop-inspector' });
  inspector.append(el('summary', 'workshop.inspector'));
  const inspectButton = button('workshop.inspect', 'workshop-inspect', () => inspect());
  const fieldSelect = el('select', null, { id: 'workshop-field' });
  const fieldValue = el('textarea', null, { id: 'workshop-field-value', rows: '3', spellcheck: 'false' });
  const fieldInfo = el('p', null, { id: 'workshop-field-info' });
  const applyField = button('workshop.apply_field', 'workshop-apply-field', () => patchField());
  inspector.append(inspectButton, el('label', 'workshop.field', { for: 'workshop-field' }), fieldSelect,
    fieldInfo, el('label', 'workshop.field_value', { for: 'workshop-field-value' }), fieldValue, applyField);
  const sourcePanel = el('div', null, { class: 'workshop-source' });
  const sourceLabel = el('label', 'workshop.source', { for: 'workshop-source' });
  const source = el('textarea', null, { id: 'workshop-source', spellcheck: 'false', 'aria-describedby': 'workshop-source-hint' });
  sourcePanel.append(sourceLabel, source, el('p', 'workshop.source_hint', { id: 'workshop-source-hint' }));
  layout.append(filesPanel, sourcePanel, inspector);
  const feedback = el('div', null, { class: 'workshop-feedback', 'aria-live': 'polite', tabindex: '-1' });
  const findings = el('div', null, { class: 'workshop-findings', role: 'status', tabindex: '-1' });
  const settings = el('div', null, { class: 'workshop-settings' });
  const settingsBody = el('div');
  settings.append(settingsBody);
  const recoveryPanel = el('section', null, { class: 'workshop-recovery', 'aria-live': 'polite' });
  const recoveryStatus = el('p', 'workshop.recovery_loading', { id: 'workshop-recovery-status' });
  const restoreButton = button('workshop.recovery_restore', 'workshop-restore', () => restoreDraft());
  const discardButton = button('workshop.recovery_discard', 'workshop-discard', () => discardRecovery());
  restoreButton.hidden = discardButton.hidden = true;
  recoveryPanel.append(recoveryStatus, restoreButton, discardButton);
  const dependencies = el('div', null, { class: 'workshop-dependencies' });
  const dependencySelect = el('select', null, { id: 'workshop-dependency' });
  const dependencySource = el('textarea', null, { id: 'workshop-dependency-source', readonly: '', rows: '8' });
  const dependencyButton = button('workshop.dependencies_load', 'workshop-dependencies-load', () => loadDependencies());
  dependencies.append(dependencyButton, el('label', 'workshop.files', { for: 'workshop-dependency' }), dependencySelect,
    el('label', 'workshop.dependency_source', { for: 'workshop-dependency-source' }), dependencySource);
  dependencyButton.disabled = !runtime.dependencies;
  root.append(toolbar, layout, testLayout);
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
  let definitionsPanel = null;
  let compositionPanel = null;
  let entityPanel = null;
  let presetsPanel = null;
  let scriptsPanel = null;
  let layoutMount = null;
  let testLayoutMount = null;
  const feedbackRows = new Map();
  const lifecycle = new ActionFeedbackLifecycle({ onTransition(value) {
    emitActionFeedbackTransition(win, value);
    if (!value.isCurrent) return;
    if (value.cancelled || !value.state) feedbackRows.delete(value.actionId);
    else feedbackRows.set(value.actionId, value);
    feedback.replaceChildren(...[...feedbackRows.values()].map(entry => {
      const row = el('span', null, { 'data-action-id': entry.actionId, 'data-correlation': entry.correlation,
        'data-state': entry.state });
      row.textContent = translate('action_feedback.summary', {
        action: translate(actions.action(entry.actionId).labelId), status: translate(entry.statusId),
      });
      return row;
    }));
    if (value.state === ACTION_FEEDBACK_STATE.PRESSED) layoutMount?.reveal('feedback');
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
        show('editor.mod.import.previous_workspace_preserved', [errorText(error)], true,
          MOD_IMPORT_ACTION_ID, activation.correlation);
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
  soundAudition = mountWorkshopSoundCues({root,win,attach:false,draft:()=>draft,native:!!provider?.save,
    resolveAsset:path=>runtime.readAsset?.(path)??null,
    readAudio:()=>profile.audio,saveAudio:audio=>{profile={...profile,audio};return saveOperatorProfile(win.PhoenixOperatorStorage||win.localStorage,profile);}});
  modelPanel = mountWorkshopModels({ root, attach: false, provider, runtime, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); } });
  definitionsPanel = mountWorkshopDefinitions({ root, attach: false, runtime, win, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); } });
  compositionPanel = mountWorkshopComposition({ root, attach: false, runtime, win, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); } });
  entityPanel = mountWorkshopEntity({ root, attach: false, runtime, win, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); },
    preview: previewTemplate, test: testTemplate });
  presetsPanel = mountWorkshopPresets({ root, attach: false, runtime, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); } });
  scriptsPanel = mountWorkshopScripts({ root, attach: false, provider, runtime, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held()),
    setBusy(value) { pendingValidation = value; refresh(); },
    changed(path) { selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed'); } });
  layoutMount = mountWorkshopLayout({
    root, surface: layout,
    panels: { files: filesPanel, source: sourcePanel, inspector, add: addPanel, recovery: recoveryPanel,
      findings, feedback, dependencies, settings,
      models: modelPanel.node, 'model-preview': modelPanel.previewNode, sound: soundAudition.node,
      changes: changesPanel, definitions: definitionsPanel.node, composition: compositionPanel.node,
      entity: entityPanel.node, presets: presetsPanel.node, scripts: scriptsPanel.node },
    labels: {
      switcher: translate('workshop.layout.switcher'), reset: translate('workshop.layout.reset'),
      float: translate('workshop.layout.float'), close: translate('workshop.layout.close'),
      dock: {
        left: translate('workshop.layout.dock_left'), right: translate('workshop.layout.dock_right'),
        top: translate('workshop.layout.dock_top'), bottom: translate('workshop.layout.dock_bottom'),
        tab: translate('workshop.layout.dock_tab'),
      },
      panels: {
        files: translate('workshop.files'), source: translate('workshop.source'), inspector: translate('workshop.inspector'),
        add: translate('workshop.add_files'), recovery: translate('workshop.recovery'),
        findings: translate('workshop.findings'), feedback: translate('workshop.feedback'),
        dependencies: translate('workshop.dependencies'), settings: translate('editor.mod.settings.heading'),
        models: translate('workshop.models.title'), 'model-preview': translate('workshop.models.preview.title'),
        sound: translate('sound_cues.title'), changes: translate('workshop.changes.title'),
        definitions: translate('workshop.definitions.title'), composition: translate('workshop.composition.title'),
        entity: translate('workshop.entity.title'), presets: translate('workshop.presets.title'),
        scripts: translate('workshop.scripts.title'),
      },
    },
    initial: profile.authoringLayout, doc, win,
    onVisible(visible) {
      modelPanel?.setPreviewVisible(visible.has('model-preview'));
      soundAudition?.setVisible(visible.has('sound'));
    },
    onChange(authoringLayout) {
      profile = { ...profile, authoringLayout };
      if (saveOperatorProfile(storage, profile).status !== 'saved') reportPersistenceFailure();
    },
  });

  /** Show a composed entity template in the shared model preview (issue #1476).
   *
   * Through the models panel's OWN subject control rather than a second preview:
   * there is one captured preview, one subject list and one set of dependency
   * rules, and a template the list does not offer is answered honestly instead
   * of appearing to work. Returns whether the surface took it. */
  function previewTemplate(path) {
    const subject = modelPanel?.node.querySelector('#workshop-preview-subject');
    if (!subject || ![...subject.options].some(option => option.value === path)) return false;
    subject.value = path;
    subject.dispatchEvent(new win.Event('change'));
    layoutMount.reveal('model-preview');
    return true;
  }

  /** Exercise the same unsaved source in the existing disposable Test, through
   * the Test panel's own ship control — one Test, one catalogue. A template that
   * catalogue does not offer as a hull is refused rather than silently ignored. */
  function testTemplate(path) {
    const ship = root.querySelector('#workshop-test-ship');
    if (!ship || ![...ship.options].some(option => option.value === path && !option.disabled)) return false;
    ship.value = path;
    ship.dispatchEvent(new win.Event('change'));
    ship.focus();
    return true;
  }

  function reportPersistenceFailure() {
    const message = translate('editor.mod.settings.storage_refused');
    if (findings.textContent === message && findings.dataset.outcome === 'refused'
        && !findings.dataset.producingAction && !findings.dataset.producingCorrelation) return;
    show('editor.mod.settings.storage_refused', [], true, null, null, { reveal: false });
    layoutMount.reveal('findings', { focus: '.workshop-findings', notify: false });
  }

  function persistBindings(result) {
    if (result.status !== 'applied') return result;
    profile = { ...profile, bindings: actions.bindingProfile() };
    const saved = saveOperatorProfile(storage, profile);
    if (saved.status !== 'saved') reportPersistenceFailure();
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
    const accessibility = el('section');
    accessibility.append(el('h2', 'settings.tab.accessibility'), el('p', 'settings.accessibility.local_hint'));
    const presentation = profile.accessibility.presentation;
    const textScale = el('input', null, { id: 'workshop-text-scale', type: 'range', min: String(TEXT_SCALE_MIN),
      max: String(TEXT_SCALE_MAX), step: String(TEXT_SCALE_STEP),
      value: String(presentation.textScale === FOLLOW_OS ? 1 : presentation.textScale) });
    const contrast = el('select', null, { id: 'workshop-contrast' });
    const motion = el('select', null, { id: 'workshop-motion' });
    const fillOptions = (node, values) => node.replaceChildren(...values.map(([value, id]) => el('option', id, { value })));
    fillOptions(contrast, [[FOLLOW_OS, 'settings.accessibility.follow_system'],
      [EXPLICIT_ON, 'settings.accessibility.contrast_more'], [EXPLICIT_OFF, 'settings.accessibility.contrast_standard']]);
    fillOptions(motion, [[FOLLOW_OS, 'settings.accessibility.follow_system'],
      [EXPLICIT_ON, 'settings.accessibility.motion_reduce'], [EXPLICIT_OFF, 'settings.accessibility.motion_allow']]);
    contrast.value = presentation.contrast;
    motion.value = presentation.reducedMotion;
    const updateAccessibility = (effect, value) => {
      const accessibilityProfile = profileWithPresentation(profile.accessibility, effect, value);
      profile = { ...profile, accessibility: accessibilityProfile };
      applyAccessibilityProfile(accessibilityProfile, { doc, win });
      if (saveOperatorProfile(storage, profile).status !== 'saved') reportPersistenceFailure();
    };
    textScale.addEventListener('input', () => updateAccessibility('textScale', Number(textScale.value)));
    contrast.addEventListener('change', () => updateAccessibility('contrast', contrast.value));
    motion.addEventListener('change', () => updateAccessibility('reducedMotion', motion.value));
    accessibility.append(el('label', 'settings.accessibility.text_scale', { for: textScale.id }), textScale,
      el('label', 'settings.accessibility.contrast', { for: contrast.id }), contrast,
      el('label', 'settings.accessibility.reduced_motion', { for: motion.id }), motion);
    settingsBody.append(accessibility);
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
    if (win.PhoenixOperatorStorageStatus?.status === 'error') reportPersistenceFailure();
  }
  win.addEventListener('phoenix-operator-storage-status', nativeStorageStatus);

  function show(id, details = [], refused = false, actionId = null, correlation = null, { reveal = refused } = {}) {
    findings.textContent = [translate(id), ...details].join('\n');
    findings.setAttribute('role', refused ? 'alert' : 'status');
    findings.dataset.outcome = refused ? 'refused' : 'applied';
    if (actionId) findings.dataset.producingAction = actionId;
    else delete findings.dataset.producingAction;
    if (correlation) findings.dataset.producingCorrelation = correlation;
    else delete findings.dataset.producingCorrelation;
    if (reveal) layoutMount.reveal('findings', { focus: '.workshop-findings' });
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
    toolbar.setAttribute('aria-busy', String(busy));
    const testing = testWorkspace;
    modelPanel?.refresh({ hidden: testing });
    definitionsPanel?.refresh({ hidden: testing });
    compositionPanel?.refresh({ hidden: testing });
    entityPanel?.refresh({ hidden: testing });
    presetsPanel?.refresh({ hidden: testing });
    scriptsPanel?.refresh({ hidden: testing });
    toolbar.hidden = layout.hidden = feedback.hidden = findings.hidden = testing;
    testLayout.hidden = !testing;
    sourceScope.hidden = testing;
    root.querySelector('.workshop-mode').textContent = translate(testing ? 'workshop.test_mode' : 'workshop.authoring');
    files.disabled = !draft || busy;
    source.disabled = !draft || busy || draft.isBinary(selected);
    checkButton.disabled = !draft || busy;
    exportButton.disabled = !draft || busy;
    importButton.disabled = busy;
    newButton.disabled = busy;
    saveButton.disabled = !draft || busy;
    addPath.disabled = addSource.disabled = addAsset.disabled = !draft || busy;
    // Both act on the selection, so neither is offered without one.
    renameButton.disabled = deleteButton.disabled = !draft || busy || !selected;
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
    paintChanges();
    paintMigration();
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
    if (validInspectorDescriptor(field)) renderInspectorMetadata(fieldInfo, field, { t: translate });
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
          showRuntimeFindings('workshop.check_refused', result.findings, true,
            exporting ? MOD_EXPORT_ACTION_ID : MOD_VALIDATE_ACTION_ID, activation.correlation);
          activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
          return;
        }
        if (exporting) {
          try { download(zip, 'mod-pack.zip', doc, win); }
          catch (error) {
            show('editor.mod.export.download_refused', [errorText(error)], true,
              MOD_EXPORT_ACTION_ID, activation.correlation);
            activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
            return;
          }
          candidate.markExported();
          persistDraft();
        }
        showRuntimeFindings(exporting ? 'workshop.exported' : 'workshop.runtime_checked', result.findings,
          false, exporting ? MOD_EXPORT_ACTION_ID : MOD_VALIDATE_ACTION_ID, activation.correlation);
        activation.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
      } catch (error) {
        if (!disposed) {
          if (error.report) showRuntimeFindings('workshop.check_refused', error.report.findings, true,
            exporting ? MOD_EXPORT_ACTION_ID : MOD_VALIDATE_ACTION_ID, activation.correlation);
          else show('workshop.runtime_unavailable', [errorText(error)], true,
            exporting ? MOD_EXPORT_ACTION_ID : MOD_VALIDATE_ACTION_ID, activation.correlation);
          activation.settleFeedback(ACTION_FEEDBACK_STATE.REFUSED);
        }
      } finally {
        pendingValidation = false;
        if (!disposed) refresh();
      }
    })();
    return true;
  }
  function showRuntimeFindings(title, records, refused = false, actionId = null, correlation = null) {
    show(title, [], refused, actionId, correlation);
    if (!refused) layoutMount.reveal('findings', { focus: '.workshop-findings' });
    for (const record of records) {
      const row = el('p');
      const location = `${record.file}${record.line ? `:${record.line}` : ''}`;
      if (draft.paths().includes(record.file)) {
        const target = button(null, '', () => {
          layoutMount.reveal('source', { focus: '#workshop-source' });
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
  /** Move the selected member to the path box's value, bytes untouched. */
  function renameDocument() {
    if (!draft || !selected || pendingValidation || pendingRecovery || pendingImport || testPanel?.held()) return;
    const path = addPath.value.trim();
    if (!path.startsWith('assets/') || (!/\.(toml|rhai)$/.test(path) && !isWorkshopBinary(path))) {
      show('workshop.add_refused', [], true); return;
    }
    if (draft.paths().includes(path) && !win.confirm(translate('workshop.replace_file_confirm', { path }))) return;
    try {
      if (draft.rename(selected, path)) {
        selected = path; refresh({ selection: true }); persistDraft(); show('workshop.changed');
      }
    } catch (error) { show('workshop.add_refused', [errorText(error)], true); }
  }

  /** Remove the selected member. Confirmed, because one press otherwise takes
   * away source whose only other copy may be the imported archive — and undone
   * by the ordinary history if it was not what the operator meant. */
  function deleteDocument() {
    if (!draft || !selected || pendingValidation || pendingRecovery || pendingImport || testPanel?.held()) return;
    const path = selected;
    if (!win.confirm(translate('workshop.delete_file_confirm', { path }))) return;
    if (!draft.remove(path)) return;
    selected = draft.paths()[0] || null;
    refresh({ selection: true }); persistDraft(); show('workshop.changed');
  }

  /** The content identity the base declares, from its own scenarios.toml.
   *
   * Parsed with the same narrow reader the proposal uses rather than a TOML
   * round-trip: this only needs two scalars, and anything it cannot read
   * confidently is simply no proposal. */
  function readBaseContent(snapshot) {
    const manifest = snapshot?.base_files?.['assets/scenarios.toml']
      ?? snapshot?.base_files?.['scenarios.toml'];
    if (typeof manifest !== 'string') return null;
    // Parsed, not pattern-matched. A regex for `id = "..."` takes the FIRST one
    // in the file, which is only the content id while `[content]` happens to
    // precede the first `[[scenario]]`; reorder the manifest and the Workshop
    // would quietly pin packs to a scenario id. `newWorkshopPack` already reads
    // it this way.
    try {
      const content = parseToml(manifest)?.content;
      if (typeof content?.epoch !== 'number') return null;
      return { contentId: typeof content.id === 'string' ? content.id : null,
        contentEpoch: content.epoch };
    } catch { return null; }
  }

  /** The migration on offer for this draft, or null. Recomputed rather than
   * cached: the draft it describes can change under it, and a stale before/after
   * is a review of source that is no longer there. */
  let baseContent = null;
  function currentMigration() {
    return draft ? proposeWorkshopMigration(draft, baseContent) : null;
  }

  /** Apply the exact text that was on screen, as ONE undoable entry.
   *
   * Re-proposed first: if the draft moved while the proposal was being read,
   * the bytes reviewed are not the bytes that would be written, and writing
   * them anyway would be applying a review of something else. */
  function acceptMigration() {
    if (!draft || pendingValidation || pendingRecovery || pendingImport || testPanel?.held()) return;
    const proposed = currentMigration();
    if (!proposed || proposed.before !== migrationBefore.value || proposed.after !== migrationAfter.value) {
      show('workshop.migration.stale', [], true); refresh(); return;
    }
    if (draft.apply([{ path: proposed.path, before: proposed.before, after: proposed.after }])) {
      selected = proposed.path;
      refresh({ selection: true }); persistDraft(); show('workshop.changed');
    }
  }

  function paintMigration() {
    const proposed = currentMigration();
    const busy = Boolean(pendingImport || pendingValidation || pendingRecovery || testPanel?.held());
    migrationPanel.hidden = !proposed;
    migrationAccept.disabled = !proposed || busy;
    if (!proposed) { migrationBefore.value = ''; migrationAfter.value = ''; return; }
    migrationSummary.textContent = translate('workshop.migration.content_epoch', {
      path: proposed.path, from: String(proposed.from), to: String(proposed.to),
      line: String(proposed.line),
    });
    migrationBefore.value = proposed.before;
    migrationAfter.value = proposed.after;
  }

  /** What this draft has done to the source it was imported as.
   *
   * Against the IMPORTED members rather than the last export: the question this
   * answers is "what have I changed about this pack", and an export in the
   * middle does not make earlier edits stop being changes. */
  function paintChanges() {
    if (!draft) {
      changesSummary.textContent = translate('workshop.changes.none');
      changesList.replaceChildren();
      return;
    }
    const diff = draft.changes(workspaceDiff);
    changesSummary.textContent = workspaceDiffIsEmpty(diff)
      ? translate('workshop.changes.none')
      : translate('workshop.changes.summary', {
        added: String(diff.added.length), removed: String(diff.removed.length),
        renamed: String(diff.renamed.length), modified: String(diff.modified.length),
      });
    const rows = [
      ...diff.added.map(path => ['added', path]),
      ...diff.removed.map(path => ['removed', path]),
      ...diff.renamed.map(entry => ['renamed', `${entry.from} → ${entry.to}`]),
      ...diff.modified.map(path => ['modified', path]),
    ];
    changesList.replaceChildren(...rows.map(([kind, text]) => {
      const row = el('li');
      row.dataset.change = kind;
      // The kind is a word, never a colour alone: a reader who cannot tell
      // green from red still has to be able to tell an addition from a removal.
      row.textContent = `${translate(`workshop.changes.${kind}`)} ${text}`;
      return row;
    }));
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
    if (!runtime.dependencies) return;
    dependencyButton.disabled = true;
    try {
      const snapshot = await runtime.dependencies();
      if (disposed) return;
      // The pin a migration would move this draft onto. Read from the same
      // dependency snapshot the Test and preview merge from, so the Workshop
      // never proposes an epoch the runtime would not itself accept.
      baseContent = readBaseContent(snapshot);
      dependencyFiles = [['base', snapshot.base_files], ...snapshot.packs.map(pack => [pack.id,
        { ...pack.files, 'scenarios.toml': pack.manifest_toml }])]
        .flatMap(([label, files]) => Object.entries(files).map(([path, text]) => ({ label: `${label}: ${path}`, text })));
      dependencySelect.replaceChildren(...dependencyFiles.map((entry, index) => {
        const option = el('option', null, { value: index }); option.textContent = entry.label; return option;
      }));
      dependencySource.value = dependencyFiles[0]?.text || '';
    } catch (error) { if (!disposed) show('workshop.runtime_unavailable', [errorText(error)], true); }
    finally { dependencyButton.disabled = false; if (!disposed) refresh(); }
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
      show('workshop.imported', [], false, MOD_IMPORT_ACTION_ID, pending.correlation, { reveal: true });
      persistDraft();
      pending.settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    } catch (error) {
      show('editor.mod.import.previous_workspace_preserved', [
        error?.code === 'workshop-missing-manifest' ? translate('workshop.missing_manifest') : errorText(error),
      ], true, MOD_IMPORT_ACTION_ID, pending.correlation);
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
      show('workshop.changed');
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
  win.addEventListener('beforeunload', beforeUnload);
  testPanel = mountWorkshopTestPanel({ root, provider, draft: () => draft,
    busy: () => Boolean(pendingImport || pendingValidation || pendingRecovery), changed(state) {
      if (state?.run) testHadRun = true;
      else if (testHadRun) { testHadRun = false; testWorkspace = false; }
      refresh();
    },
    leave() { testWorkspace = false; refresh(); }, win });
  if (testPanel.node && testPanel.viewNode) {
    testLayoutMount = mountWorkshopLayout({
      root, surface: testLayout, model: workshopTestLayoutModel,
      panels: { 'test-controls': testPanel.node, 'test-viewscreen': testPanel.viewNode },
      labels: {
        switcher: translate('workshop.layout.switcher'), reset: translate('workshop.layout.reset'),
        float: translate('workshop.layout.float'), close: translate('workshop.layout.close'),
        dock: {
          left: translate('workshop.layout.dock_left'), right: translate('workshop.layout.dock_right'),
          top: translate('workshop.layout.dock_top'), bottom: translate('workshop.layout.dock_bottom'),
          tab: translate('workshop.layout.dock_tab'),
        },
        panels: {
          'test-controls': translate('workshop.test_heading'),
          'test-viewscreen': translate('workshop.test_view'),
        },
      },
      initial: profile.testLayout, doc, win,
      onChange(testLayoutValue) {
        profile = { ...profile, testLayout: testLayoutValue };
        if (saveOperatorProfile(storage, profile).status !== 'saved') reportPersistenceFailure();
      },
    });
  }
  refresh({ selection: true });
  show('workshop.start');
  nativeStorageStatus();
  /** The base identity a migration is measured against.
   *
   * Read at boot rather than waiting for the Dependencies panel's Load button:
   * the criterion is that older content OPENS with a proposal, and a proposal
   * nobody can see until they press an unrelated control has not opened with
   * anything. Failure is silence — no base identity simply means no proposal.
   */
  async function loadBaseContent() {
    if (!runtime.dependencies) return;
    try {
      const snapshot = await runtime.dependencies();
      if (disposed) return;
      baseContent = readBaseContent(snapshot);
      refresh();
    } catch { /* No identity, no proposal. */ }
  }

  const ready = (async () => {
    let record;
    void loadBaseContent();
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
    testLayoutMount?.dispose();
    testPanel.dispose();
    soundAudition?.dispose();
    modelPanel?.dispose();
    definitionsPanel?.dispose();
    compositionPanel?.dispose();
    entityPanel?.dispose();
    presetsPanel?.dispose();
    scriptsPanel?.dispose();
    layoutMount.dispose();
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
