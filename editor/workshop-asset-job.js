import { prepareWorkshopChanges } from './workshop-acceptance.js';

const same = (a, b) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** One native job and its private reviewed candidate. Adapters own output meaning. */
export function createWorkshopAssetJob({ call, fetcher, upload, runtime, restoreDocument }, adapter) {
  let active = null, transitions = Promise.resolve();
  const serial = operation => {
    const next = transitions.then(operation);
    transitions = next.catch(() => {});
    return next;
  };
  const cancelNative = () => call({ op: `${adapter.operation}-cancel` });
  const fresh = (held, draft = held?.draft, selection = held?.selection) => {
    if (!held || active !== held || held.draft !== draft || !same(held.selection, selection)
        || held.revision !== draft.sourceRevision) throw new Error(adapter.stale);
  };
  function retire(held) {
    if (active !== held) return Promise.resolve();
    active = null;
    return serial(cancelNative);
  }
  async function failure(held, error) {
    await retire(held).catch(() => {});
    throw error;
  }
  function accept(held, value) {
    fresh(held);
    adapter.validate(value);
    if (value.source_revision !== held.revision || !adapter.matches(value, held.selection)
        || (held.id && held.id !== adapter.id(value))) throw new Error(adapter.stale);
    held.id = adapter.id(value);
    held.value = value;
    return structuredClone(value);
  }
  async function prepare(held) {
    if (!held.preparing) held.preparing = prepareWorkshopChanges({
      draft: held.draft, provider: { restoreDocument, save: true }, runtime, dependencies: null,
      current: () => active === held, stale: adapter.stale, validateUnchanged: true,
      prepare: async candidate => {
        const value = held.value, base = adapter.route(value);
        const entries = await Promise.all(value.paths.map(async (path, index) => {
          const response = await fetcher(new URL(String(index), base), { cache: 'no-store', credentials: 'omit' });
          fresh(held);
          if (!response.ok) throw new Error(adapter.unavailable(path));
          const bytes = new Uint8Array(await response.arrayBuffer());
          fresh(held);
          adapter.validateMember(path, bytes);
          const after = adapter.binary(path) ? await upload(bytes) : new TextDecoder('utf-8', { fatal: true }).decode(bytes);
          fresh(held);
          return { path, before: candidate.members().get(path) ?? null, after };
        }));
        fresh(held);
        return entries.filter(change => !same(change.before, change.after));
      },
    });
    const review = await held.preparing;
    fresh(held);
    held.review = review;
    return review;
  }
  return {
    async start(draft, selection, options = {}) {
      const held = { draft, selection, revision: draft.sourceRevision, value: null, id: null };
      active = held;
      try {
        return await serial(async () => {
          await cancelNative().catch(() => {});
          fresh(held);
          const value = await call({ op: `${adapter.operation}-start`, files: draft.toNativeSources(),
            ...selection, source_revision: held.revision, ...options });
          return accept(held, value);
        });
      } catch (error) { return failure(held, error); }
    },
    async status(draft, selection) {
      const held = active;
      if (!held && adapter.emptyStatus) return null;
      try {
        fresh(held, draft, selection);
        const value = accept(held, await serial(() => {
          fresh(held, draft, selection);
          return call({ op: `${adapter.operation}-status` });
        }));
        if (value.state === 'ready' && adapter.reviewOnStatus) await prepare(held);
        fresh(held, draft, selection);
        return adapter.reviewOnStatus ? { ...value, reviewReady: Boolean(held.review) } : value;
      } catch (error) { return failure(held, error); }
    },
    reviewDraft(draft, selection) {
      fresh(active, draft, selection);
      if (!active.review) throw new Error(adapter.notReady);
      return active.review.candidate;
    },
    async adopt(draft, selection) {
      const held = active;
      try {
        fresh(held, draft, selection);
        if (held.value?.state !== 'ready') throw new Error(adapter.stale);
        if (adapter.reviewOnStatus && !held.review) throw new Error(adapter.notReady);
        const review = held.review || await prepare(held);
        fresh(held, draft, selection);
        const result = review.commit();
        await retire(held).catch(() => {});
        return adapter.adopted(held.value, result);
      } catch (error) { return failure(held, error); }
    },
    cancel() {
      active = null;
      return serial(cancelNative);
    },
    get active() { return active?.value ? structuredClone(active.value) : null; },
  };
}
