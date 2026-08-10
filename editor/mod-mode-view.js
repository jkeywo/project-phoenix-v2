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
  readStoreZip,
  MANIFEST_PATH,
} from './mod-pack-export.js';
import { resolveEntityConfig as defaultResolveEntityConfig } from './entity-cache.js';
import { readFile as defaultReadFile } from './project-root.js';
import { canonicalTemplatePath } from './entity-includes.js';

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
  readArchive = readStoreZip,
  download = defaultDownload,
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
  const importInput = el('input', { type: 'file', accept: '.zip', class: 'mod-import-input' });
  importInput.addEventListener('change', async () => {
    const file = importInput.files && importInput.files[0];
    if (!file) return;
    const buf = await file.arrayBuffer();
    await importArchiveBytes(new Uint8Array(buf));
    importInput.value = '';
  });
  const importLabel = el('label', { class: 'mod-import-label', text: 'Import pack: ' });
  importLabel.appendChild(importInput);
  actionsSection.append(exportBtn, importLabel);
  const messages = el('div', { class: 'mod-messages' });
  actionsSection.appendChild(messages);
  body.appendChild(actionsSection);

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

  // ── Edit plumbing ─────────────────────────────────────────────────────────

  function markDirty(dirty = true) {
    modeShell?.markDirty?.('MOD', MOD_DIRTY_KEY, dirty);
  }

  function onMetaInput(key, input) {
    if (key === 'content_epoch') {
      const raw = input.value.trim();
      const epoch = raw === '' ? null : Number.parseInt(raw, 10);
      workspace.setPack({ requires: { content_epoch: Number.isNaN(epoch) ? null : epoch } });
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

  async function importArchiveBytes(bytes) {
    clearMessages();
    let files;
    try {
      files = readArchive(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
    } catch (e) {
      renderMessages({ errors: [`could not read archive: ${e.message}`], warnings: [] });
      return null;
    }
    const memberPaths = Object.keys(files).filter((p) => p !== MANIFEST_PATH);
    const baseFiles = {};
    for (const p of memberPaths) {
      const text = await readMaybe(p);
      if (text !== undefined) baseFiles[p] = text;
    }
    workspace = ModPackWorkspace.fromArchiveFiles(files, baseFiles);
    markDirty(false);
    renderAll();
    return workspace;
  }

  renderAll();

  return {
    getWorkspace: () => workspace,
    render: renderAll,
    _internal: {
      addMemberByPath,
      addFragmentMembers,
      exportPackNow,
      importArchiveBytes,
      currentBaseForPatches,
      onMetaInput,
      fields,
      elements: { exportBtn, importInput, memPath, memAddBtn, scenAddBtn, messages, memList, scenList },
    },
  };
}
