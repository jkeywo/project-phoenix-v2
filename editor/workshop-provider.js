/** Explicit Authoring capabilities. Native requests exist only on the private
 * embedded bridge; ordinary browser entry points have no filesystem methods. */
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from './workshop-document.js';
import { createWorkshopRuntime } from './workshop-runtime.js';
import { createStoreZip } from './mod-pack-export.js';

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
export function createBrowserWorkshopProvider({ loadedPack = null, dependencies, load } = {}) {
  const pack = loadedPack ? Uint8Array.from(loadedPack) : null;
  const editableId = pack ? parse(new WorkshopDocument(pack).read('scenarios.toml')).pack?.id : null;
  const source = dependencies ? JSON.stringify({ ...dependencies,
    packs: (dependencies.packs || []).filter(candidate => candidate.id !== editableId) }) : null;
  const runtime = createWorkshopRuntime({ ...(load ? { load } : {}),
    ...(source ? { dependencies: async () => JSON.parse(source) } : {}) });
  return { runtime, canImport: true, canCreate: true,
    async load() { return pack ? new WorkshopDocument(pack) : null; },
  };
}

export function createNativeWorkshopProvider({ request }) {
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
  return {
    canImport: false, canCreate: false,
    async load() {
      const value = await call({ op: 'load' });
      if (value?.status !== 'loaded' || !['project', 'mod'].includes(value.kind) || typeof value.revision !== 'string') throw new Error('Invalid native Workshop load');
      revision = value.revision; kind = value.kind;
      return WorkshopDocument.fromFiles(value.files, { kind });
    },
    runtime: {
      async validate(_archive, draft) { return (await call({ op: 'validate', files: draft.toFiles() })).report; },
      async inspect(source, document_path) { return (await call({ op: 'inspect', source, document_path })).fields; },
      async patch(source, patch) { return (await call({ op: 'patch', source, patch })).source; },
    },
    async save(draft) {
      const result = await call({ op: 'save', files: draft.toFiles(), expected_revision: revision });
      if (result?.status !== 'saved' || typeof result.revision !== 'string') throw new Error('Invalid native Workshop save');
      revision = result.revision;
    },
    restore(record) {
      if (typeof record.nativeRevision !== 'string' || record.draft.kind !== kind) throw new Error('Invalid native Workshop recovery');
      revision = record.nativeRevision;
    },
    recovery: {
      async load() {
        const value = (await call({ op: 'recovery-load' })).recovery;
        if (!value) return null;
        const record = JSON.parse(value.record);
        if (!Array.isArray(record?.draft?.source) || !record.draft.source.every(byte => Number.isInteger(byte) && byte >= 0 && byte <= 255)) throw new Error('Invalid native recovery archive');
        record.draft.source = Uint8Array.from(record.draft.source);
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
