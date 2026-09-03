/**
 * mod-mode-view.js — the MOD-mode DOM view (issue #989).
 *
 * The fifth editor mode. It owns all DOM + IO around the pure
 * {@link ModPackWorkspace}: a `[pack]` identity form, a `[[scenario]]` list, a
 * member list with per-member `new`/`patch` provenance and bounded source edit,
 * plus Import / Validate / Export actions. Validate and Export run the existing
 * `exportModPack` admission gate (issue #759 / #986); Export downloads the
 * host-consumed ZIP, while Import reads one back into the workspace via the real
 * archive reader. Every edit marks MOD mode dirty so the shared `beforeunload`
 * guard fires (via `modeShell.hasAnyDirty()`).
 *
 * Discipline (matches #910 / M5): the logic module (`mod-pack-workspace.js`) is
 * DOM-free; THIS view owns the DOM. IO — reading base files to classify + detect
 * stale patches, resolving a composed hull's fragment closure into extra
 * members (#910), and the file download — is injectable so the view runs under
 * jsdom without a real filesystem or a browser download.
 */

import { ModPackWorkspace } from './mod-pack-workspace.js';
import {
  exportModPack,
  readStoreZipArchive,
  MANIFEST_PATH,
} from './mod-pack-export.js';
import { resolveEntityConfig as defaultResolveEntityConfig } from './entity-cache.js';
import { readFile as defaultReadFile } from './project-root.js';
import { canonicalTemplatePath } from './entity-includes.js';
import '../gui/strings-boot.js';
import { t } from '../gui/strings.js';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
  emitActionFeedbackTransition,
} from '../gui/action-feedback.js';
import {
  MOD_ACTION_CONTEXT,
  MOD_EXPORT_ACTION_ID,
  MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION_ID,
  createModActionRegistry,
} from './mod-actions.js';
import { createSemanticControlsRemapper } from '../gui/semantic-controls-remapper.js';
import {
  applyOperatorProfile,
  createOperatorProfileSnapshot,
  loadOperatorProfile,
  saveOperatorProfile,
} from '../gui/operator-profile.js';

/** MOD mode has no per-file save; a single sentinel key carries its dirty bit
 * so `modeShell.hasAnyDirty()` (and the beforeunload guard) sees pending edits. */
export const MOD_DIRTY_KEY = 'mod-pack';

const ENTITIES_PREFIX = 'assets/entities/';

function browserProfileStorage() {
  try {
    return typeof window !== 'undefined' ? window.localStorage : null;
  } catch {
    return null;
  }
}

function preserveSourceLineEndings(previousText, editedText) {
  const previous = String(previousText ?? '');
  const edited = String(editedText ?? '');
  const crlfCount = (previous.match(/\r\n/g) || []).length;
  const loneLfCount = (previous.replace(/\r\n/g, '').match(/\n/g) || []).length;
  return crlfCount > 0 && loneLfCount === 0
    ? edited.replace(/\r?\n/g, '\r\n')
    : edited;
}

/** Default browser download: a store-only ZIP Blob + a transient anchor click.
 * Guarded so a headless/jsdom run (no `URL.createObjectURL`) is a silent no-op. */
function defaultDownload(bytes, filename) {
  if (
    typeof document === 'undefined' ||
    typeof URL === 'undefined' ||
    typeof URL.createObjectURL !== 'function'
  ) {
    return;
  }
  const blob = new Blob([bytes], { type: 'application/zip' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') node.className = v;
    else if (k === 'text') node.textContent = v;
    else if (k === 'dataset') Object.assign(node.dataset, v);
    else if (k.startsWith('on') && typeof v === 'function') {
      node.addEventListener(k.slice(2).toLowerCase(), v);
    } else if (v != null) node.setAttribute(k, v);
  }
  for (const c of [].concat(children)) {
    if (c == null) continue;
    node.appendChild(typeof c === 'string' ? document.createTextNode(c) : c);
  }
  return node;
}

export function mountModMode({
  host,
  modeShell,
  io = {},
  rigIndex = null,
  resolveEntityConfig = defaultResolveEntityConfig,
  // Injectable seams so the view runs headless in tests.
  exportPack = exportModPack,
  readArchive = readStoreZipArchive,
  download = defaultDownload,
  feedbackRoot = typeof window !== 'undefined' ? window : globalThis,
  profileStorage = browserProfileStorage(),
} = {}) {
  if (!host) return null;

  const readFile = io.readFile || defaultReadFile;

  let workspace = new ModPackWorkspace();

  // ── Skeleton ──────────────────────────────────────────────────────────────
  host.innerHTML = '';
  const root = el('div', { class: 'mod-mode' });
  host.appendChild(root);

  root.appendChild(el('div', { class: 'mod-mode-header', text: 'MOD PACK' }));
  const body = el('div', { class: 'mod-mode-body' });
  root.appendChild(body);

  // Metadata form ------------------------------------------------------------
  const metaSection = el('section', { class: 'mod-section mod-meta' });
  metaSection.appendChild(el('h3', { text: 'Pack Identity — [pack]' }));
  const metaForm = el('div', { class: 'mod-meta-form' });
  metaSection.appendChild(metaForm);
  body.appendChild(metaSection);

  const fields = {};
  function metaField(key, label, { type = 'text', placeholder = '' } = {}) {
    const wrap = el('label', { class: 'mod-field' });
    wrap.appendChild(el('span', { class: 'mod-field-label', text: label }));
    const input = el('input', { type, placeholder, class: `mod-input mod-input-${key}` });
    input.dataset.field = key;
    input.addEventListener('input', () => onMetaInput(key, input));
    wrap.appendChild(input);
    metaForm.appendChild(wrap);
    fields[key] = input;
    return input;
  }
  metaField('id', 'id', { placeholder: 'my-pack' });
  metaField('version', 'version', { placeholder: '1.0.0' });
  metaField('name', 'name', { placeholder: 'My Pack' });
  metaField('author', 'author', { placeholder: '(optional)' });
  metaField('description', 'description', { placeholder: '(optional)' });
  metaField('content_id', 'requires.content_id', { placeholder: 'phoenix-base' });
  metaField('content_epoch', 'requires.content_epoch', { type: 'number', placeholder: '1' });

  // Scenarios ----------------------------------------------------------------
  const scenSection = el('section', { class: 'mod-section mod-scenarios' });
  scenSection.appendChild(el('h3', { text: 'Scenarios — [[scenario]]' }));
  const scenList = el('div', { class: 'mod-scenario-list' });
  scenSection.appendChild(scenList);
  const scenAdd = el('div', { class: 'mod-add-row' });
  const scenId = el('input', { type: 'text', placeholder: 'id', class: 'mod-input mod-scenario-id' });
  const scenWorld = el('input', { type: 'text', placeholder: 'assets/worlds/x.toml', class: 'mod-input mod-scenario-world' });
  const scenLabel = el('input', { type: 'text', placeholder: 'label (optional)', class: 'mod-input mod-scenario-label' });
  const scenAddBtn = el('button', { type: 'button', class: 'mod-scenario-add-btn', text: 'Add scenario' });
  scenAddBtn.addEventListener('click', () => {
    const id = scenId.value.trim();
    const world = scenWorld.value.trim();
    if (id === '' && world === '') return;
    workspace.addScenario({ id, world, label: scenLabel.value.trim() });
    scenId.value = '';
    scenWorld.value = '';
    scenLabel.value = '';
    markDirty();
    renderScenarios();
  });
  scenAdd.append(scenId, scenWorld, scenLabel, scenAddBtn);
  scenSection.appendChild(scenAdd);
  body.appendChild(scenSection);

  // Members ------------------------------------------------------------------
  const memSection = el('section', { class: 'mod-section mod-members' });
  memSection.appendChild(el('h3', { text: 'Members' }));
  const memList = el('div', { class: 'mod-member-list' });
  memSection.appendChild(memList);
  const memAdd = el('div', { class: 'mod-add-row' });
  const memPath = el('input', {
    type: 'text',
    placeholder: 'assets/worlds/my_world.toml',
    class: 'mod-input mod-member-path',
  });
  const memAddBtn = el('button', { type: 'button', class: 'mod-member-add-btn', text: 'Add member' });
  memAddBtn.addEventListener('click', async () => {
    const path = memPath.value.trim();
    if (path === '') return;
    await addMemberByPath(path);
    memPath.value = '';
  });
  memAdd.append(memPath, memAddBtn);
  memSection.appendChild(memAdd);
  const memberEditor = el('section', {
    class: 'mod-member-source-editor',
    dataset: { t2MemberSourceEditor: 'true' },
  });
  memberEditor.hidden = true;
  const memberEditorHeading = el('h4', { text: t('editor.mod.member.heading') });
  const memberEditorPath = el('code', { class: 'mod-member-source-path' });
  const memberEditorInput = el('textarea', {
    class: 'mod-member-source-input',
    rows: '14',
    spellcheck: 'false',
  });
  memberEditorInput.addEventListener('input', () => {
    if (!selectedMemberPath) return;
    const current = workspace.getMember(selectedMemberPath);
    const text = preserveSourceLineEndings(current?.text, memberEditorInput.value);
    if (workspace.setMemberText(selectedMemberPath, text)) markDirty();
  });
  memberEditor.append(
    memberEditorHeading,
    memberEditorPath,
    memberEditorInput,
    el('p', { class: 'mod-member-source-hint', text: t('editor.mod.member.hint') }),
  );
  memSection.appendChild(memberEditor);
  body.appendChild(memSection);

  // Actions ------------------------------------------------------------------
  const actionsSection = el('section', { class: 'mod-section mod-actions' });
  const importBtn = el('button', {
    type: 'button',
    class: 'mod-import-btn',
    text: t('editor.mod.import.button'),
    'aria-label': t('semantic_action.editor.mod.import.accessibility'),
  });
  const validateBtn = el('button', {
    type: 'button',
    class: 'mod-validate-btn',
    text: t('editor.mod.validate.button'),
    'aria-label': t('semantic_action.editor.mod.validate.accessibility'),
  });
  const exportBtn = el('button', {
    type: 'button',
    class: 'mod-export-btn',
    text: t('editor.mod.export.button'),
    'aria-label': t('semantic_action.editor.mod.export.accessibility'),
  });
  const importInput = el('input', {
    type: 'file',
    accept: '.zip,application/zip',
    class: 'mod-import-input mod-file-input',
    tabindex: '-1',
    'aria-hidden': 'true',
  });
  const feedbackStatus = el('div', {
    class: 'mod-action-feedback',
    role: 'status',
    'aria-live': 'polite',
    'aria-atomic': 'true',
  });
  const messages = el('div', {
    class: 'mod-messages',
  });
  const scopeBoundary = el('p', {
    class: 'mod-scope-boundary',
    text: t('editor.mod.import.m6_boundary'),
  });
  const privateSettings = el('details', { class: 'mod-private-settings' });
  privateSettings.appendChild(el('summary', { text: t('editor.mod.settings.heading') }));
  const privateSettingsBody = el('div', { class: 'mod-private-settings-body' });
  privateSettings.appendChild(privateSettingsBody);
  actionsSection.append(
    importBtn,
    validateBtn,
    exportBtn,
    importInput,
    feedbackStatus,
    messages,
    privateSettings,
    scopeBoundary,
  );
  body.appendChild(actionsSection);

  let selectedMemberPath = null;
  let pendingImport = null;
  let pendingOperation = null;
  const feedbackByAction = new Map();
  let feedbackSequence = 0;

  function renderActionFeedback() {
    feedbackStatus.innerHTML = '';
    const rows = [...feedbackByAction.values()].sort((left, right) => (
      left.sequence - right.sequence
    ));
    if (rows.length === 0) {
      delete feedbackStatus.dataset.state;
      return;
    }
    for (const entry of rows) {
      const action = semanticActions.action(entry.actionId);
      feedbackStatus.appendChild(el('div', {
        class: 'mod-action-feedback-row',
        dataset: { actionId: entry.actionId, state: entry.state },
        text: t('action_feedback.summary', {
          action: t(action.labelId),
          status: t(entry.statusId),
        }),
      }));
    }
    feedbackStatus.dataset.state = rows[rows.length - 1].state;
  }

  const actionFeedback = new ActionFeedbackLifecycle({
    onTransition(value) {
      emitActionFeedbackTransition(feedbackRoot, value);
      if (!value.isCurrent) return;
      if (value.cancelled || !value.state || !value.statusId) {
        const shown = feedbackByAction.get(value.actionId);
        if (!shown || shown.correlation === value.correlation) {
          feedbackByAction.delete(value.actionId);
        }
      } else {
        feedbackSequence += 1;
        feedbackByAction.set(value.actionId, {
          ...value,
          sequence: feedbackSequence,
        });
      }
      renderActionFeedback();
    },
  });
  const semanticActions = createModActionRegistry({
    actionFeedback,
    openImport: beginImport,
    validatePack: beginValidate,
    exportPack: beginExport,
  });
  let operatorProfile = null;
  let profileStatus = null;
  try {
    const loaded = loadOperatorProfile(profileStorage, { registry: semanticActions });
    const applied = applyOperatorProfile(loaded.profile, semanticActions);
    operatorProfile = loaded.profile;
    profileStatus = applied.status === 'applied'
      ? { status: loaded.status }
      : { status: 'rejected', code: applied.code };
  } catch {
    profileStatus = { status: 'rejected', code: 'profile-storage-read' };
  }

  const semanticControls = createSemanticControlsRemapper({
    doc: document,
    root: privateSettings,
    setBinding(actionId, slot, binding, options) {
      const result = semanticActions.setBinding(actionId, slot, binding, options);
      if (result.status === 'applied') persistPrivateProfile();
      return result;
    },
    resetAction(actionId) {
      const result = semanticActions.resetAction(actionId);
      if (result.status === 'applied') persistPrivateProfile();
      return result;
    },
    resetAll() {
      const result = semanticActions.resetAllBindings();
      if (result.status === 'applied') persistPrivateProfile();
      return result;
    },
    rebuild: renderPrivateSettings,
  });

  importBtn.addEventListener('click', () => {
    semanticActions.activate(MOD_IMPORT_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
      source: 'control',
    });
  });
  validateBtn.addEventListener('click', () => {
    semanticActions.activate(MOD_VALIDATE_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
      source: 'control',
    });
  });
  exportBtn.addEventListener('click', () => {
    semanticActions.activate(MOD_EXPORT_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
      source: 'control',
    });
  });
  importInput.addEventListener('change', completeSelectedImport);
  importInput.addEventListener('cancel', cancelPendingImport);

  function persistPrivateProfile() {
    operatorProfile = createOperatorProfileSnapshot({
      accessibility: operatorProfile?.accessibility,
      bindings: semanticActions.bindingProfile(),
      preferredGamepadSlot: operatorProfile?.gamepad?.preferredSlot,
      tuning: semanticActions.tuningProfile(),
      feedback: operatorProfile?.feedback,
      gmConfirmations: operatorProfile?.gmConfirmations,
    });
    const saved = saveOperatorProfile(profileStorage, operatorProfile);
    profileStatus = saved.status === 'saved'
      ? { status: 'saved' }
      : { status: 'rejected', code: saved.code };
    renderPrivateSettings();
    return saved;
  }

  function settingsSection(labelId) {
    const section = el('section', { class: 'mod-settings-section' });
    section.appendChild(el('h4', { text: t(labelId) }));
    return section;
  }

  function renderPrivateSettings() {
    privateSettingsBody.innerHTML = '';
    if (profileStatus?.status === 'saved' || profileStatus?.status === 'rejected') {
      privateSettingsBody.appendChild(el('div', {
        class: `mod-profile-status mod-profile-status-${profileStatus.status}`,
        role: profileStatus.status === 'rejected' ? 'alert' : 'status',
        'aria-live': profileStatus.status === 'rejected' ? 'assertive' : 'polite',
        text: t(profileStatus.status === 'rejected'
          ? 'editor.mod.settings.storage_refused'
          : 'editor.mod.settings.saved'),
      }));
    }
    semanticControls.render(privateSettingsBody, {
      actions: semanticActions.list(MOD_ACTION_CONTEXT),
      section: settingsSection,
      hint: (labelId) => el('p', { class: 'mod-settings-hint', text: t(labelId) }),
      row: (className) => el('div', { class: `mod-settings-row ${className}` }),
      action: (label, onClick) => el('button', {
        type: 'button',
        class: 'mod-settings-action',
        text: label,
        onclick: onClick,
      }),
    });
  }

  renderPrivateSettings();

  // ── Rendering ───────────────────────────────────────────────────────────

  function syncMetaForm() {
    const p = workspace.getPack();
    fields.id.value = p.id;
    fields.version.value = p.version;
    fields.name.value = p.name;
    fields.author.value = p.author;
    fields.description.value = p.description;
    fields.content_id.value = p.requires.content_id;
    fields.content_epoch.value =
      p.requires.content_epoch == null ? '' : String(p.requires.content_epoch);
  }

  function renderScenarios() {
    scenList.innerHTML = '';
    const scenarios = workspace.getScenarios();
    if (scenarios.length === 0) {
      scenList.appendChild(el('p', { class: 'mod-placeholder', text: 'No scenarios yet.' }));
      return;
    }
    for (const s of scenarios) {
      const row = el('div', { class: 'mod-scenario-row', dataset: { id: s.id } });
      row.appendChild(el('span', { class: 'mod-scenario-row-id', text: s.id || '(no id)' }));
      row.appendChild(el('span', { class: 'mod-scenario-row-world', text: s.world || '(no world)' }));
      if (s.label) row.appendChild(el('span', { class: 'mod-scenario-row-label', text: s.label }));
      if (Array.isArray(s.ships) && s.ships.length > 0) {
        row.appendChild(el('span', {
          class: 'mod-scenario-row-ships',
          text: s.ships.join(', '),
        }));
      }
      const rm = el('button', { type: 'button', class: 'mod-scenario-remove', text: '×' });
      rm.addEventListener('click', () => {
        workspace.removeScenario(s.id);
        markDirty();
        renderScenarios();
      });
      row.appendChild(rm);
      scenList.appendChild(row);
    }
  }

  function renderMembers() {
    memList.innerHTML = '';
    const members = workspace.getMembers();
    if (members.length === 0) {
      memList.appendChild(el('p', { class: 'mod-placeholder', text: 'No members yet.' }));
      selectedMemberPath = null;
      renderMemberEditor();
      return;
    }
    if (!members.some((member) => member.path === selectedMemberPath)) {
      selectedMemberPath = null;
    }
    for (const m of members) {
      const row = el('div', {
        class: `mod-member-row mod-member-${m.classification}`,
        dataset: { path: m.path, classification: m.classification },
      });
      row.appendChild(el('span', {
        class: `mod-member-badge mod-member-badge-${m.classification}`,
        text: m.classification,
      }));
      row.appendChild(el('span', { class: 'mod-member-path', text: m.path }));
      const edit = el('button', {
        type: 'button',
        class: 'mod-member-edit',
        text: t('editor.mod.member.edit'),
        'aria-label': t('editor.mod.member.edit_accessibility', { path: m.path }),
      });
      edit.addEventListener('click', () => {
        selectedMemberPath = m.path;
        renderMemberEditor();
        focusWithoutJump(memberEditorInput);
      });
      const rm = el('button', { type: 'button', class: 'mod-member-remove', text: '×' });
      rm.addEventListener('click', () => {
        workspace.removeMember(m.path);
        if (selectedMemberPath === m.path) selectedMemberPath = null;
        markDirty();
        renderMembers();
      });
      row.append(edit, rm);
      memList.appendChild(row);
    }
    renderMemberEditor();
  }

  function renderMemberEditor() {
    const member = selectedMemberPath ? workspace.getMember(selectedMemberPath) : null;
    memberEditor.hidden = !member;
    if (!member) {
      memberEditorPath.textContent = '';
      memberEditorInput.value = '';
      memberEditorInput.removeAttribute('aria-label');
      return;
    }
    memberEditorPath.textContent = member.path;
    memberEditorInput.value = member.text;
    memberEditorInput.setAttribute(
      'aria-label',
      t('editor.mod.member.source_accessibility', { path: member.path }),
    );
  }

  function renderAll() {
    syncMetaForm();
    renderScenarios();
    renderMembers();
  }

  function clearMessages() {
    messages.innerHTML = '';
  }

  function appendFindings(box, errors = [], warnings = []) {
    if (errors.length > 0) {
      const ul = el('ul');
      for (const error of errors) ul.appendChild(el('li', { text: error }));
      box.appendChild(ul);
    }
    if (warnings.length > 0) {
      const warningBox = el('div', { class: 'mod-messages-warnings' });
      warningBox.appendChild(el('strong', { text: t('editor.mod.import.warning_heading') }));
      const ul = el('ul');
      for (const warning of warnings) ul.appendChild(el('li', { text: warning }));
      warningBox.appendChild(ul);
      box.appendChild(warningBox);
    }
  }

  function renderOperationRefusal(headingId, errors = [], warnings = []) {
    messages.innerHTML = '';
    const box = el('div', {
      class: 'mod-operation-result mod-messages-errors',
      role: 'alert',
      tabindex: '-1',
      dataset: { outcome: 'refused' },
    });
    box.appendChild(el('strong', { text: t(headingId) }));
    appendFindings(box, errors, warnings);
    messages.appendChild(box);
    focusWithoutJump(box);
    return box;
  }

  function renderOperationSuccess(messageId, warnings = []) {
    messages.innerHTML = '';
    const box = el('div', {
      class: 'mod-operation-result mod-messages-ok',
      role: 'status',
      dataset: { outcome: 'applied' },
    });
    box.appendChild(el('p', { text: t(messageId) }));
    appendFindings(box, [], warnings);
    messages.appendChild(box);
    return box;
  }

  function focusWithoutJump(node) {
    if (!node || typeof node.focus !== 'function') return;
    try {
      node.focus({ preventScroll: true });
    } catch {
      node.focus();
    }
  }

  function renderImportFindings({ errors = [], warnings = [] }) {
    messages.innerHTML = '';
    const box = el('div', {
      class: 'mod-import-findings mod-messages-errors',
      role: 'alert',
      tabindex: '-1',
      dataset: { outcome: 'refused' },
    });
    box.appendChild(el('strong', { text: t('editor.mod.import.invalid_heading') }));
    box.appendChild(el('p', { text: t('editor.mod.import.source_retained') }));
    if (errors.length > 0) {
      const ul = el('ul');
      for (const error of errors) ul.appendChild(el('li', { text: error }));
      box.appendChild(ul);
    }
    if (warnings.length > 0) {
      const warningHeading = el('strong', { text: t('editor.mod.import.warning_heading') });
      const ul = el('ul');
      for (const warning of warnings) ul.appendChild(el('li', { text: warning }));
      box.append(warningHeading, ul);
    }
    messages.appendChild(box);
    focusWithoutJump(box);
    return box;
  }

  function renderUnreadableArchive(reason) {
    messages.innerHTML = '';
    const box = el('div', {
      class: 'mod-import-findings mod-messages-errors',
      role: 'alert',
      tabindex: '-1',
      dataset: { outcome: 'refused' },
    });
    box.appendChild(el('strong', { text: t('editor.mod.import.unreadable_heading') }));
    box.appendChild(el('p', {
      text: t('editor.mod.import.unreadable_detail', { reason: String(reason || '') }),
    }));
    box.appendChild(el('p', { text: t('editor.mod.import.previous_workspace_preserved') }));
    messages.appendChild(box);
    focusWithoutJump(box);
    return box;
  }

  function renderImportSuccess(warnings = []) {
    messages.innerHTML = '';
    const box = el('div', {
      class: 'mod-messages-ok mod-import-success',
      role: 'status',
      dataset: { outcome: 'applied' },
    });
    box.appendChild(el('p', { text: t('editor.mod.import.success') }));
    if (warnings.length > 0) {
      box.appendChild(el('strong', { text: t('editor.mod.import.warning_heading') }));
      const ul = el('ul');
      for (const warning of warnings) ul.appendChild(el('li', { text: warning }));
      box.appendChild(ul);
    }
    messages.appendChild(box);
  }

  function refuseBusyOperation(headingId, settleFeedback) {
    renderOperationRefusal(headingId, [t('editor.mod.operation_busy')]);
    settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
    return true;
  }

  function beginValidate({ settleFeedback } = {}) {
    if (pendingOperation === MOD_VALIDATE_ACTION_ID) return false;
    if (pendingOperation || pendingImport) {
      return refuseBusyOperation('editor.mod.validate.refused', settleFeedback);
    }
    pendingOperation = MOD_VALIDATE_ACTION_ID;
    void validatePackNow({ settleFeedback })
      .catch((error) => {
        renderOperationRefusal('editor.mod.validate.refused', [String(error?.message || error)]);
        settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      })
      .finally(() => {
        if (pendingOperation === MOD_VALIDATE_ACTION_ID) pendingOperation = null;
      });
    return true;
  }

  function beginExport({ settleFeedback } = {}) {
    if (pendingOperation === MOD_EXPORT_ACTION_ID) return false;
    if (pendingOperation || pendingImport) {
      return refuseBusyOperation('editor.mod.export.refused', settleFeedback);
    }
    pendingOperation = MOD_EXPORT_ACTION_ID;
    void exportPackNow({ settleFeedback })
      .catch((error) => {
        renderOperationRefusal('editor.mod.export.refused', [String(error?.message || error)]);
        settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      })
      .finally(() => {
        if (pendingOperation === MOD_EXPORT_ACTION_ID) pendingOperation = null;
      });
    return true;
  }

  function beginImport({ settleFeedback, cancelFeedback } = {}) {
    if (pendingImport) return false;
    if (pendingOperation) {
      return refuseBusyOperation('editor.mod.import.invalid_heading', settleFeedback);
    }
    pendingImport = { settleFeedback, cancelFeedback };
    importInput.value = '';
    try {
      importInput.click();
    } catch (error) {
      const pending = pendingImport;
      pendingImport = null;
      renderUnreadableArchive(error?.message);
      pending?.settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
    }
    return true;
  }

  function cancelPendingImport() {
    if (!pendingImport) return false;
    const pending = pendingImport;
    pendingImport = null;
    importInput.value = '';
    pending.cancelFeedback?.();
    focusWithoutJump(importBtn);
    return true;
  }

  async function completeSelectedImport() {
    const file = importInput.files && importInput.files[0];
    if (!file) {
      cancelPendingImport();
      return;
    }
    const pending = pendingImport;
    try {
      const buf = await file.arrayBuffer();
      await importArchiveBytes(new Uint8Array(buf), {
        settleFeedback: pending?.settleFeedback,
      });
    } catch (error) {
      renderUnreadableArchive(error?.message);
      pending?.settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
    } finally {
      pendingImport = null;
      importInput.value = '';
    }
  }

  // ── Edit plumbing ─────────────────────────────────────────────────────────

  function markDirty(dirty = true) {
    modeShell?.markDirty?.('MOD', MOD_DIRTY_KEY, dirty);
  }

  function onMetaInput(key, input) {
    if (key === 'content_epoch') {
      const raw = input.value.trim();
      let epoch = null;
      if (/^[+-]?\d+$/.test(raw)) {
        const exact = BigInt(raw);
        epoch = exact >= BigInt(Number.MIN_SAFE_INTEGER) && exact <= BigInt(Number.MAX_SAFE_INTEGER)
          ? Number(exact)
          : exact;
      }
      workspace.setPack({ requires: { content_epoch: epoch } });
    } else if (key === 'content_id') {
      workspace.setPack({ requires: { content_id: input.value } });
    } else {
      workspace.setPack({ [key]: input.value });
    }
    markDirty();
  }

  /** Read a file's on-disk text, or `undefined` if it is not under the root. */
  async function readMaybe(path) {
    try {
      return await readFile(path);
    } catch {
      return undefined;
    }
  }

  /**
   * Add a member by project-root-relative path. Reads the on-disk content so a
   * path that already exists classifies as `patch` (base digest recorded from
   * that content); an absent path is a `new` member seeded with empty text. When
   * the member is a composed entity hull, its `includes` fragment closure is
   * pulled in as extra members automatically (issue #910).
   */
  async function addMemberByPath(path) {
    const onDisk = await readMaybe(path);
    const baseFiles = onDisk === undefined ? {} : { [path]: onDisk };
    workspace.addMember({ path, text: onDisk ?? '' }, baseFiles);
    if (path.startsWith(ENTITIES_PREFIX)) {
      await addFragmentMembers(path);
    }
    markDirty();
    renderMembers();
  }

  /**
   * For a composed hull, resolve its include closure and add every fragment as
   * its own member (issue #910), so an exported pack never references a fragment
   * it lacks. Fragments already present are left untouched.
   */
  async function addFragmentMembers(hullPath) {
    let resolution;
    try {
      resolution = await resolveEntityConfig(hullPath);
    } catch {
      return;
    }
    if (!resolution || !resolution.ok || !Array.isArray(resolution.sources)) return;
    const rootCanonical = canonicalTemplatePath(hullPath);
    for (const source of resolution.sources) {
      if (source === rootCanonical || source === hullPath) continue;
      if (workspace.hasMember(source)) continue;
      const text = await readMaybe(source);
      if (text === undefined) continue; // resolver/exporter surfaces a missing fragment
      workspace.addMember({ path: source, text }, { [source]: text });
    }
  }

  // ── Export / Import ─────────────────────────────────────────────────────

  /** Re-read the on-disk base for every `patch` member, for the stale check. */
  async function currentBaseForPatches() {
    const out = {};
    for (const m of workspace.getMembers()) {
      if (m.classification !== 'patch') continue;
      const text = await readMaybe(m.path);
      if (text !== undefined) out[m.path] = text;
    }
    return out;
  }

  async function evaluateWorkspace() {
    const currentBase = await currentBaseForPatches();
    const staleWarnings = workspace.staleWarnings(currentBase);
    const staleMsgs = staleWarnings.map((w) => `${w.path}: ${w.message}`);
    const result = exportPack({ ...workspace.toExportInput(), rigIndex });
    return {
      result,
      staleWarnings,
      warnings: [...(result.warnings || []), ...staleMsgs],
    };
  }

  async function validatePackNow({ settleFeedback = null } = {}) {
    clearMessages();
    const evaluated = await evaluateWorkspace();
    const { result, staleWarnings, warnings } = evaluated;
    if (!result.ok) {
      renderOperationRefusal('editor.mod.validate.refused', result.errors || [], warnings);
      settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      return { ...result, staleWarnings };
    }
    renderOperationSuccess('editor.mod.validate.success', warnings);
    settleFeedback?.(ACTION_FEEDBACK_STATE.APPLIED);
    focusWithoutJump(exportBtn);
    return { ...result, staleWarnings };
  }

  async function exportPackNow({ settleFeedback = null } = {}) {
    clearMessages();
    const evaluated = await evaluateWorkspace();
    const { result, staleWarnings, warnings } = evaluated;
    if (!result.ok) {
      renderOperationRefusal('editor.mod.export.refused', result.errors || [], warnings);
      settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      return { ...result, staleWarnings };
    }
    const filename = `${workspace.getPack().id || 'mod-pack'}.zip`;
    try {
      download(result.zip, filename);
    } catch (error) {
      renderOperationRefusal(
        'editor.mod.export.download_refused',
        [String(error?.message || error)],
        warnings,
      );
      settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      // Preserve the successfully generated bytes for recovery/caller retry.
      return { ...result, staleWarnings, downloaded: false, downloadError: error };
    }
    markDirty(false);
    renderOperationSuccess('editor.mod.export.success', warnings);
    settleFeedback?.(ACTION_FEEDBACK_STATE.APPLIED);
    focusWithoutJump(exportBtn);
    return { ...result, staleWarnings, downloaded: true };
  }

  async function importArchiveBytes(bytes, { settleFeedback = null } = {}) {
    clearMessages();
    let files;
    let sourceArchive = null;
    try {
      const decoded = readArchive(
        bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes),
      );
      // Keep the injection seam compatible with its historical map-only shape,
      // while the real reader carries exact source provenance alongside it.
      if (decoded?.files && decoded?.source) {
        files = decoded.files;
        sourceArchive = decoded.source;
      } else {
        files = decoded;
      }
    } catch (e) {
      renderUnreadableArchive(e.message);
      settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      return null;
    }
    const memberPaths = Object.keys(files).filter((p) => p !== MANIFEST_PATH);
    const baseFiles = {};
    for (const p of memberPaths) {
      const text = await readMaybe(p);
      if (text !== undefined) baseFiles[p] = text;
    }
    let candidate;
    try {
      candidate = ModPackWorkspace.fromArchiveFiles(files, baseFiles, sourceArchive);
    } catch (error) {
      renderUnreadableArchive(error.message);
      settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      return null;
    }

    // A readable source bundle is the operator's recovery asset.  Install it
    // before surfacing semantic findings so the exact member text remains in
    // the editable workspace even when the existing export gate refuses it.
    workspace = candidate;
    selectedMemberPath = null;
    markDirty(false);
    renderAll();
    const validation = exportPack({ ...workspace.toExportInput(), rigIndex });
    if (!validation.ok) {
      renderImportFindings({
        errors: validation.errors || [],
        warnings: validation.warnings || [],
      });
      settleFeedback?.(ACTION_FEEDBACK_STATE.REFUSED);
      return workspace;
    }

    renderImportSuccess(validation.warnings || []);
    settleFeedback?.(ACTION_FEEDBACK_STATE.APPLIED);
    focusWithoutJump(fields.id);
    return workspace;
  }

  renderAll();

  return {
    getWorkspace: () => workspace,
    getOperatorProfile: () => operatorProfile,
    render: renderAll,
    semanticActions,
    dispatchKeyboardEvent: (event) => semanticActions.dispatchKeyboardEvent(
      event,
      MOD_ACTION_CONTEXT,
    ),
    _internal: {
      addMemberByPath,
      addFragmentMembers,
      validatePackNow,
      exportPackNow,
      importArchiveBytes,
      currentBaseForPatches,
      onMetaInput,
      fields,
      elements: {
        exportBtn,
        validateBtn,
        importBtn,
        importInput,
        feedbackStatus,
        scopeBoundary,
        memPath,
        memAddBtn,
        memberEditor,
        memberEditorInput,
        privateSettings,
        privateSettingsBody,
        scenAddBtn,
        messages,
        memList,
        scenList,
      },
    },
  };
}
