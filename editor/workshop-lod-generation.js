import { isWorkshopBinary } from './workshop-document.js';

function valid(value) {
  return value?.status === 'lod-generation' && typeof value.run === 'string'
    && ['running', 'ready'].includes(value.state) && Array.isArray(value.progress)
    && value.progress.length <= 64 && value.progress.every(line => typeof line === 'string' && line.length <= 240)
    && typeof value.sidecar === 'string' && Number.isInteger(value.source_revision)
    && Array.isArray(value.paths) && value.paths.every(path => typeof path === 'string')
    && Array.isArray(value.required_paths) && value.required_paths.length > 0
    && value.required_paths.every(path => typeof path === 'string' && path.startsWith('assets/models/') && path.endsWith('.glb'))
    && (value.state !== 'ready' || typeof value.base_url === 'string');
}

function route(value) {
  const url = new URL(value.base_url);
  if (url.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)
      || !url.pathname.startsWith('/workshop-lod-generation/') || !url.pathname.endsWith('/')) {
    throw new Error('Invalid native LOD review route');
  }
  return url;
}

const same = (left, right) => JSON.stringify(left ?? null) === JSON.stringify(right ?? null);

export function createWorkshopLodGeneration({ call, fetcher, upload, runtime, restoreDocument }) {
  let active = null;
  const ensureCurrent = (draft, sidecar) => {
    if (!active || active.draft !== draft || active.sidecar !== sidecar || draft.sourceRevision !== active.revision) {
      throw new Error('workshop.lod_generation.stale');
    }
  };
  const accept = (value, draft, sidecar) => {
    if (!valid(value)) throw new Error('Invalid native LOD generation result');
    if (value.source_revision !== active.revision || value.sidecar !== sidecar || (active.run && active.run !== value.run)) {
      throw new Error('workshop.lod_generation.stale');
    }
    active = { ...active, run: value.run, value }; return structuredClone(value);
  };
  async function prepareReview(value) {
    if (active.candidate) return;
    const base = route(value), entries = await Promise.all(value.paths.map(async (path, index) => {
      const response = await fetcher(new URL(String(index), base), { cache: 'no-store', credentials: 'omit' });
      if (!response.ok) throw new Error(`Generated LOD review member is unavailable: ${path}`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      return [path, isWorkshopBinary(path) ? await upload(bytes) : new TextDecoder('utf-8', { fatal: true }).decode(bytes)];
    }));
    const changes = entries.map(([path, after]) => ({ path, before: active.draft.members().get(path) ?? null, after }))
      .filter(change => !same(change.before, change.after));
    if (!value.paths.includes('scripts/lod-manifest.toml')
        || value.required_paths.some(path => !value.paths.includes(path))) {
      throw new Error('Generated LOD review is incomplete');
    }
    const candidate = restoreDocument(active.draft.snapshot()); candidate.apply(changes);
    const report = await runtime.validate(null, candidate);
    if (!report?.accepted) { const error = new Error('runtime-validation-refused'); error.report = report; throw error; }
    active = { ...active, changes, candidate, report };
  }
  async function fail(error) {
    await call({ op: 'lod-generate-cancel' }).catch(() => {}); active = null; throw error;
  }
  return {
    async start(draft, sidecar, { remesh = false } = {}) {
      await call({ op: 'lod-generate-cancel' }).catch(() => {});
      active = { draft, sidecar, revision: draft.sourceRevision, run: null, value: null };
      try { return accept(await call({ op: 'lod-generate-start', files: draft.toNativeSources(), sidecar,
        source_revision: draft.sourceRevision, remesh: Boolean(remesh) }), draft, sidecar); }
      catch (error) { return fail(error); }
    },
    async status(draft, sidecar) {
      try {
        ensureCurrent(draft, sidecar);
        const value = accept(await call({ op: 'lod-generate-status' }), draft, sidecar);
        ensureCurrent(draft, sidecar);
        if (value.state === 'ready') await prepareReview(value);
        return { ...value, reviewReady: Boolean(active.candidate) };
      } catch (error) { return fail(error); }
    },
    reviewDraft(draft, sidecar) { ensureCurrent(draft, sidecar); if (!active.candidate) throw new Error('workshop.lod_generation.not_ready'); return active.candidate; },
    async adopt(draft, sidecar) {
      try {
        ensureCurrent(draft, sidecar); if (!active.candidate) throw new Error('workshop.lod_generation.not_ready');
        if (active.changes.some(change => !same(draft.members().get(change.path) ?? null, change.before))) {
          throw new Error('workshop.lod_generation.stale');
        }
        const changed = draft.apply(active.changes);
        const result = { changed, paths: active.changes.map(change => change.path), report: active.report };
        active = null; await call({ op: 'lod-generate-cancel' }).catch(() => {}); return result;
      } catch (error) { return fail(error); }
    },
    async cancel() { active = null; await call({ op: 'lod-generate-cancel' }); },
    get active() { return active?.value ? structuredClone(active.value) : null; },
  };
}
