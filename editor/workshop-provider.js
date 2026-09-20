/** Explicit Authoring capabilities. Native requests exist only on the private
 * embedded bridge; ordinary browser entry points have no filesystem methods. */
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument, isNativeAssetReference } from './workshop-document.js';
import { createWorkshopRuntime } from './workshop-runtime.js';
import { createStoreZip } from './mod-pack-export.js';
import { createBrowserWorkshopTest, createWorkshopTestFrame } from './workshop-test-frame.js';
import { createWorkshopTestPreparation } from './workshop-test-snapshot.js';
import { createBrowserWorkshopPreview, createWorkshopPreviewFrame } from './workshop-preview.js';

export function newWorkshopPack(dependencies) {
  const content = parse(dependencies.base_files['assets/scenarios.toml']).content;
  if (typeof content?.id !== 'string' || !Number.isInteger(content.epoch)) throw new Error('Missing base content identity');
  return new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: stringify({ pack: { format: 1, id: 'workshop-pack', name: 'workshop-pack', version: '1.0.0',
      requires: { content_id: content.id, content_epoch: content.epoch } },
    scenario: [{ id: 'workshop', world: 'assets/worlds/workshop.toml' }] }) },
    { path: 'assets/worlds/workshop.toml', text: '[global]\n[anchors]\n' },
  ]));
}

/** Caller supplies an immutable loaded pack only after leaving Live. No GM
 * state, credentials, profile, document object or installation API is accepted. */
export function createBrowserWorkshopProvider({ loadedPack = null, dependencies, load,
  testFrame = createWorkshopTestFrame, previewFrame = createWorkshopPreviewFrame } = {}) {
  const pack = loadedPack ? Uint8Array.from(loadedPack) : null;
  const editableId = pack ? parse(new WorkshopDocument(pack).read('scenarios.toml')).pack?.id : null;
  const source = dependencies ? structuredClone({ ...dependencies,
    packs: (dependencies.packs || []).filter(candidate => candidate.id !== editableId) }) : null;
  const runtime = createWorkshopRuntime({ ...(load ? { load } : {}),
    ...(source ? { dependencies: async () => structuredClone(source) } : {}) });
  const preparation = createWorkshopTestPreparation({ runtime, dependencies: () => runtime.testDependencies() });
  let viewport = null, title = '';
  const test = createBrowserWorkshopTest({ ...preparation, frame: () => {
    if (!viewport) throw new Error('Workshop Test viewport is unavailable');
    return testFrame({ mount: viewport, title });
  } });
  // The preview shares the Test's preparation — one merger, one set of
  // dependency rules, one definition of what a captured draft is.
  let previewViewport = null, previewTitle = '';
  const modelPreview = createBrowserWorkshopPreview({ prepare: preparation.prepareSubject, frame: () => {
    if (!previewViewport) throw new Error('Workshop preview viewport is unavailable');
    return previewFrame({ mount: previewViewport, title: previewTitle });
  } });
  return { runtime, canImport: true, canCreate: true,
    test: { ...test, mount(target, label) { viewport = target; title = label; } },
    modelPreview: { ...modelPreview,
      // The preview session passes `{ title }`; the Test panel passes a plain
      // string. Accept both rather than putting an object into a frame title.
      mount(target, label) {
        previewViewport = target;
        previewTitle = typeof label === 'string' ? label : label?.title || '';
      } },
    async load() { return pack ? new WorkshopDocument(pack) : null; },
  };
}

export function createNativeWorkshopProvider({ request, previewFrame = createWorkshopPreviewFrame,
  fetcher = (...args) => fetch(...args) }) {
  if (typeof request !== 'function') throw new Error('Native Workshop bridge is unavailable');
  let revision = null;
  let kind = null;
  let operation = Promise.resolve();
  const call = value => {
    const next = operation.then(async () => {
      const response = await request(value);
      if (response?.status === 'refused') { const error = new Error(response.message); error.report = response.report; throw error; }
      return response;
    });
    operation = next.catch(() => {});
    return next;
  };
  let previewViewport = null, previewTitle = '';
  const textDecoder = new TextDecoder('utf-8', { fatal: true });
  const preview = createBrowserWorkshopPreview({
    async prepare(files, selection) {
      const value = await call({ op: 'preview-start', files, selection });
      if (value?.status !== 'preview' || typeof value.capture !== 'string'
          || typeof value.base_url !== 'string' || typeof value.revision !== 'string'
          || !Array.isArray(value.paths) || value.paths.some(path => typeof path !== 'string')) {
        throw new Error('Invalid native Workshop preview capture');
      }
      const base = new URL(value.base_url);
      if (base.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(base.hostname)
          || !base.pathname.startsWith('/workshop-preview-capture/') || !base.pathname.endsWith('/')) {
        throw new Error('Invalid native Workshop preview route');
      }
      try {
        const entries = await Promise.all(value.paths.map(async (path, index) => {
          const response = await fetcher(new URL(String(index), base), { cache: 'no-store', credentials: 'omit' });
          if (!response.ok) throw new Error(`Captured Workshop preview member is unavailable: ${path}`);
          const bytes = new Uint8Array(await response.arrayBuffer());
          return [path, /\.(toml|rhai)$/.test(path) ? textDecoder.decode(bytes) : bytes];
        }));
        return { files: Object.fromEntries(entries), selection: value.selection,
          revision: value.revision, nativeCapture: value.capture };
      } catch (error) {
        await call({ op: 'preview-release', capture: value.capture }).catch(() => {});
        throw error;
      }
    },
    frame: () => {
      if (!previewViewport) throw new Error('Workshop preview viewport is unavailable');
      return previewFrame({ mount: previewViewport, title: previewTitle });
    },
    release(prepared) {
      return prepared?.nativeCapture
        ? call({ op: 'preview-release', capture: prepared.nativeCapture }).then(() => {})
        : Promise.resolve();
    },
  });
  return {
    canImport: false, canCreate: false,
    async load() {
      const value = await call({ op: 'load-sources' });
      if (value?.status !== 'sources' || !['project', 'mod'].includes(value.kind) || typeof value.revision !== 'string') throw new Error('Invalid native Workshop load');
      revision = value.revision; kind = value.kind;
      return WorkshopDocument.fromNativeFiles(value.files, { kind });
    },
    runtime: {
      async dependencies() {
        const value = await call({ op: 'load-dependencies' });
        const textFiles = files => files && typeof files === 'object' && !Array.isArray(files)
          && Object.values(files).every(text => typeof text === 'string');
        if (value?.status !== 'dependencies' || !textFiles(value.base_files) || !Array.isArray(value.packs)
            || value.packs.some(pack => typeof pack?.id !== 'string' || typeof pack.manifest_toml !== 'string'
              || !textFiles(pack.files))) {
          const error = new Error();
          error.code = 'native-workshop-dependencies-invalid';
          throw error;
        }
        return { base_files: value.base_files, packs: value.packs };
      },
      async validate(_archive, draft) { return (await call({ op: 'validate-sources', files: draft.toNativeSources() })).report; },
      async inspect(source, document_path) { return (await call({ op: 'inspect', source, document_path })).fields; },
      async patch(source, patch) { return (await call({ op: 'patch', source, patch })).source; },
      // The native provider builds the dependency bundle itself, the way the
      // validate route does, so only the draft's text members cross the bridge.
      async definitions(files) {
        const response = await call({ op: 'definitions', files });
        if (response?.status !== 'definitions' || !response.catalog || typeof response.catalog !== 'object') throw new Error('Invalid native definition catalog');
        return response.catalog;
      },
      async edit(source, edit) { return (await call({ op: 'edit', source, edit })).source; },
      async newFaction(name, uuid) { return (await call({ op: 'new-faction', name, uuid })).source; },
      // Composition (issue #1475) crosses the bridge the same way: the draft's
      // text members only, the host resolving the dependencies beneath them.
      async composition(files) {
        const response = await call({ op: 'composition', files });
        if (response?.status !== 'composition' || !response.catalog || typeof response.catalog !== 'object') throw new Error('Invalid native composition catalog');
        return response.catalog;
      },
      async compose(files, request) { return (await call({ op: 'compose', files, request })).source; },
      async newWorld(title) { return (await call({ op: 'new-world', title })).source; },
      // Entity composition (issue #1476) crosses the bridge the same way: the
      // draft's text members only, the host resolving the dependencies beneath
      // them and the template named by path.
      async entity(files, path) {
        const response = await call({ op: 'entity', files, path });
        if (response?.status !== 'entity' || !response.composition || typeof response.composition !== 'object') throw new Error('Invalid native entity composition');
        return response.composition;
      },
      async editEntity(files, request) { return (await call({ op: 'entity-edit', files, request })).source; },
      async materialiseEntity(files, path, address) {
        return (await call({ op: 'entity-materialise', files, path, address })).source;
      },
      // GM role presets (issue #1477) cross the bridge the same way: the draft's
      // text members only, the host resolving the dependencies beneath them and
      // the world named by path.
      async presets(files, path) {
        const response = await call({ op: 'presets', files, path });
        if (response?.status !== 'presets' || !response.presets || typeof response.presets !== 'object') throw new Error('Invalid native preset catalog');
        return response.presets;
      },
      async editPresets(files, request) { return (await call({ op: 'presets-edit', files, request })).source; },
      // `preset_id`, not `id`: the bridge envelope owns the key `id` (it is the
      // request's correlation number, and the host strips it before the
      // operation is read), so a preset's own id cannot travel under that name.
      async newPreset(id, label) { return (await call({ op: 'new-preset', preset_id: id, label })).source; },
      async scriptHostFunctions() {
        const value = await call({ op: 'script-host-functions' });
        if (value?.status !== 'script-host-functions' || !Array.isArray(value.functions)) throw new Error('Invalid Workshop Rhai registry');
        return value.functions;
      },
      async scriptDiagnostics(source, line_offset = 0) {
        const value = await call({ op: 'script-diagnostics', source, line_offset });
        if (value?.status !== 'script-diagnostics' || !Array.isArray(value.diagnostics)) throw new Error('Invalid Workshop Rhai diagnostics');
        return value.diagnostics;
      },
    },
    async save(draft) {
      const result = await call({ op: 'save-sources', files: draft.toNativeSources(), expected_revision: revision });
      if (result?.status !== 'saved' || typeof result.revision !== 'string') throw new Error('Invalid native Workshop save');
      revision = result.revision;
    },
    restore(record) {
      if (typeof record.nativeRevision !== 'string' || record.draft.kind !== kind) throw new Error('Invalid native Workshop recovery');
      revision = record.nativeRevision;
    },
    restoreDocument(snapshot) { return WorkshopDocument.restore(snapshot, { native: true }); },
    async importAsset(file) {
      const begin = await call({ op: 'asset-begin', length: file.size });
      if (begin?.status !== 'asset-upload' || typeof begin.token !== 'string') throw new Error('Invalid native asset upload');
      let finished = false;
      try {
        for (let offset = 0; offset < file.size; offset += 65536) {
          const bytes = new Uint8Array(await file.slice(offset, offset + 65536).arrayBuffer());
          await call({ op: 'asset-chunk', token: begin.token, offset, bytes: Array.from(bytes) });
        }
        const result = await call({ op: 'asset-finish', token: begin.token });
        if (result?.status !== 'asset-stored' || !isNativeAssetReference(result.reference) || result.reference.length !== file.size) throw new Error('Invalid native asset version');
        finished = true;
        return result.reference;
      } finally {
        if (!finished) await call({ op: 'asset-cancel', token: begin.token }).catch(() => {});
      }
    },
    async readAsset(reference) {
      if (!isNativeAssetReference(reference)) throw new Error('Invalid native asset reference');
      const bytes = new Uint8Array(reference.length);
      if (!bytes.length) {
        const result = await call({ op: 'asset-read', reference, offset: 0 });
        if (result?.status !== 'asset-chunk' || !Array.isArray(result.bytes) || result.bytes.length) throw new Error('Invalid native asset chunk');
      }
      for (let offset = 0; offset < bytes.length;) {
        const result = await call({ op: 'asset-read', reference, offset });
        if (result?.status !== 'asset-chunk' || !Array.isArray(result.bytes) || !result.bytes.length
          || result.bytes.length > Math.min(65536, bytes.length - offset)
          || result.bytes.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)) throw new Error('Invalid native asset chunk');
        bytes.set(result.bytes, offset); offset += result.bytes.length;
      }
      return bytes;
    },
    modelPreview: {
      capture(draft) { return draft.toNativeSources(); },
      start: preview.start,
      control: preview.control,
      status: preview.status,
      async stop() {
        await preview.stop();
        await call({ op: 'preview-stop' });
      },
      mount(target, label) {
        previewViewport = target;
        previewTitle = typeof label === 'string' ? label : label?.title || '';
      },
    },
    test: {
      async catalog(files) {
        const response = await call({ op: 'test-catalog', files });
        const value = response?.catalog;
        if (response?.status !== 'test-catalog' || !value || !['worlds', 'ships'].every(key =>
          Array.isArray(value[key]) && value[key].every(path => typeof path === 'string'))) throw new Error('Invalid native Test catalogue');
        return value;
      },
      async start(files, selection) {
        const response = await call({ op: 'test-start', files, selection });
        if (response?.status !== 'test' || !response.run) throw new Error('Invalid native Test start');
        return response.run;
      },
      async control(control) {
        const response = await call({ op: 'test-control', control });
        if (response?.status !== 'test') throw new Error('Invalid native Test control');
        return response.run || { running: false };
      },
      async status() {
        const response = await call({ op: 'test-status' });
        if (response?.status !== 'test') throw new Error('Invalid native Test status');
        return response.run || { running: false };
      },
      async stop() {
        const response = await call({ op: 'test-stop' });
        if (response?.status !== 'test' || response.run) throw new Error('Invalid native Test stop');
      },
    },
    recovery: {
      async load() {
        const value = (await call({ op: 'recovery-load' })).recovery;
        if (!value) return null;
        const record = JSON.parse(value.record);
        // A native draft carries its members as `sourceFiles`; every other
        // version is a browser record whose archive has to come back as bytes.
        // Listed rather than compared, so a new native version is a deliberate
        // addition instead of silently taking the browser branch (issue #1471
        // added version 5 alongside 3).
        if (![3, 5].includes(record?.draft?.version)) {
          if (!Array.isArray(record?.draft?.source) || !record.draft.source.every(byte => Number.isInteger(byte) && byte >= 0 && byte <= 255)) throw new Error('Invalid native recovery archive');
          record.draft.source = Uint8Array.from(record.draft.source);
        }
        return { ...record, nativeRevision: value.revision };
      },
      async save(record) {
        const text = JSON.stringify(record, (_key, value) => value instanceof Uint8Array ? Array.from(value) : value);
        // Capture the revision in the same serialized operation as the draft;
        // callers serialize a successful save before filing its new baseline.
        return call({ op: 'recovery-save', record: text, expected_revision: revision });
      },
      async clear() { return call({ op: 'recovery-clear' }); },
    },
  };
}

/** One private request/response queue. The host drains only this embedded view
 * and invokes receive(json) on that same view; no HTTP or game transport. */
export function createWorkshopBridge({ send }) {
  let sequence = 0;
  const pending = new Map();
  return {
    request(operation) {
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        try { send(JSON.stringify({ id, ...operation })); }
        catch (error) { pending.delete(id); reject(error); }
      });
    },
    receive(json) {
      const response = typeof json === 'string' ? JSON.parse(json) : json;
      const request = pending.get(response?.id);
      if (!request) return false;
      pending.delete(response.id); request.resolve(response); return true;
    },
    dispose() { for (const request of pending.values()) request.reject(new Error('Workshop surface closed')); pending.clear(); },
  };
}
