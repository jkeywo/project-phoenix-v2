/**
 * mod-mode-view.js — the MOD-mode DOM view (issue #989).
 *
 * The fifth editor mode. It owns all DOM + IO around the pure
 * {@link ModPackWorkspace}: a `[pack]` identity form, a `[[scenario]]` list, a
 * member list with per-member `new`/`patch` provenance, and Export / Import
 * actions. Export runs the existing `exportModPack` admission gate (issue #759 /
 * #986) and downloads the host-consumed ZIP; Import reads a ZIP back into the
 * workspace via `readStoreZip`. Every edit marks MOD mode dirty so the shared
 * `beforeunload` guard fires (via `modeShell.hasAnyDirty()`).
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
  MOD_IMPORT_ACTION_ID,
  createModActionRegistry,
} from './mod-actions.js';

/** MOD mode has no per-file save; a single sentinel key carries its dirty bit
 * so `modeShell.hasAnyDirty()` (and the beforeunload guard) sees pending edits. */
export const MOD_DIRTY_KEY = 'mod-pack';

const ENTITIES_PREFIX = 'assets/entities/';

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
  body.appendChild(memSection);

  // Actions ------------------------------------------------------------------
  const actionsSection = el('section', { class: 'mod-section mod-actions' });
  const exportBtn = el('button', { type: 'button', class: 'mod-export-btn', text: 'Export pack (.zip)' });
  exportBtn.addEventListener('click', () => { exportPackNow(); });
  const importBtn = el('button', {
    type: 'button',
    class: 'mod-import-btn',
    text: t('editor.mod.import.button'),
    'aria-label': t('semantic_action.editor.mod.import.accessibility'),
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
  actionsSection.append(exportBtn, importBtn, importInput, feedbackStatus, messages, scopeBoundary);
  body.appendChild(actionsSection);

  let pendingImport = null;
  const actionFeedback = new ActionFeedbackLifecycle({
    onTransition(value) {
      emitActionFeedbackTransition(feedbackRoot, value);
      if (!value.isCurrent) return;
      if (value.cancelled || !value.state || !value.statusId) {
        feedbackStatus.textContent = '';
        delete feedbackStatus.dataset.state;
        return;
      }
      feedbackStatus.textContent = t(value.statusId);
      feedbackStatus.dataset.state = value.state;
    },
  });
  const semanticActions = createModActionRegistry({
    actionFeedback,
    openImport: beginImport,
  });
  importBtn.addEventListener('click', () => {
    semanticActions.activate(MOD_IMPORT_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
      source: 'control',
    });
  });
  importInput.addEventListener('change', completeSelectedImport);
  importInput.addEventListener('cancel', cancelPendingImport);

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
      return;
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
      const rm = el('button', { type: 'button', class: 'mod-member-remove', text: '×' });
      rm.addEventListener('click', () => {
        workspace.removeMember(m.path);
        markDirty();
        renderMembers();
      });
      row.appendChild(rm);
      memList.appendChild(row);
    }
  }

  function renderAll() {
    syncMetaForm();
    renderScenarios();
    renderMembers();
  }

  function clearMessages() {
    messages.innerHTML = '';
  }

  function renderMessages({ errors = [], warnings = [] }) {
    messages.innerHTML = '';
    if (errors.length > 0) {
      const box = el('div', { class: 'mod-messages-errors' });
      box.appendChild(el('strong', { text: 'Export refused — resolve these first:' }));
      const ul = el('ul');
      for (const e of errors) ul.appendChild(el('li', { text: e }));
      box.appendChild(ul);
      messages.appendChild(box);
    }
    if (warnings.length > 0) {
      const box = el('div', { class: 'mod-messages-warnings' });
      box.appendChild(el('strong', { text: 'Warnings (non-blocking):' }));
      const ul = el('ul');
      for (const w of warnings) ul.appendChild(el('li', { text: w }));
      box.appendChild(ul);
      messages.appendChild(box);
    }
    if (errors.length === 0 && warnings.length === 0) {
      messages.appendChild(el('p', { class: 'mod-messages-ok', text: 'Pack exported.' }));
    }
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

  function beginImport({ settleFeedback, cancelFeedback } = {}) {
    if (pendingImport) return false;
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

  async function exportPackNow() {
    clearMessages();
    const currentBase = await currentBaseForPatches();
    const staleWarnings = workspace.staleWarnings(currentBase);
    const staleMsgs = staleWarnings.map((w) => `${w.path}: ${w.message}`);

    const result = exportPack({ ...workspace.toExportInput(), rigIndex });
    if (!result.ok) {
      renderMessages({
        errors: result.errors || [],
        warnings: [...(result.warnings || []), ...staleMsgs],
      });
      return { ...result, staleWarnings };
    }
    renderMessages({ errors: [], warnings: [...(result.warnings || []), ...staleMsgs] });
    const filename = `${workspace.getPack().id || 'mod-pack'}.zip`;
    try {
      download(result.zip, filename);
    } catch {
      // A download failure must not lose the exported bytes; they are returned.
    }
    markDirty(false);
    return { ...result, staleWarnings };
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
    render: renderAll,
    semanticActions,
    dispatchKeyboardEvent: (event) => semanticActions.dispatchKeyboardEvent(
      event,
      MOD_ACTION_CONTEXT,
    ),
    _internal: {
      addMemberByPath,
      addFragmentMembers,
      exportPackNow,
      importArchiveBytes,
      currentBaseForPatches,
      onMetaInput,
      fields,
      elements: {
        exportBtn,
        importBtn,
        importInput,
        feedbackStatus,
        scopeBoundary,
        memPath,
        memAddBtn,
        scenAddBtn,
        messages,
        memList,
        scenList,
      },
    },
  };
}
