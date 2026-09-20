const validResult = value => value?.status === 'billboard-capture'
  && typeof value.capture === 'string'
  && ['running', 'ready'].includes(value.state)
  && typeof value.output === 'string' && value.output.startsWith('assets/models/') && value.output.endsWith('.png')
  && typeof value.sidecar === 'string' && Number.isInteger(value.lod)
  && Number.isInteger(value.source_revision) && typeof value.source === 'string'
  && Number.isInteger(value.yaw_views) && value.yaw_views > 0
  && Number.isInteger(value.resolution) && value.resolution > 0 && Number.isFinite(value.pitch)
  && Array.isArray(value.paths) && value.paths.every(path => typeof path === 'string')
  && (value.state !== 'ready' || (typeof value.image_url === 'string' && typeof value.base_url === 'string'
    && value.paths.includes(value.output) && value.paths.includes(value.sidecar)
    && value.paths.includes('scripts/lod-capture-manifest.toml')));

function loopbackImage(value) {
  const url = new URL(value.image_url);
  if (url.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)
      || !url.pathname.startsWith('/workshop-billboard-capture/') || !/^\d+$/.test(url.pathname.split('/').at(-1))) {
    throw new Error('Invalid native billboard capture route');
  }
  return url;
}

function loopbackMembers(value) {
  const url = new URL(value.base_url);
  if (url.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)
      || !url.pathname.startsWith('/workshop-billboard-capture/') || !url.pathname.endsWith('/')) {
    throw new Error('Invalid native billboard capture route');
  }
  return url;
}

const same = (left, right) => JSON.stringify(left ?? null) === JSON.stringify(right ?? null);

/** Native-only reviewed bake. The returned PNG remains outside the draft until
 * adopt validates an exact candidate and applies one grouped history entry. */
export function createWorkshopBillboardCapture({ call, fetcher, upload, runtime, restoreDocument }) {
  let active = null;
  const current = (draft, sidecar, lod) => active && active.draft === draft && active.sidecar === sidecar
    && active.lod === lod && draft.sourceRevision === active.revision;
  const accept = (value, draft, sidecar, lod) => {
    if (!validResult(value)) throw new Error('Invalid native billboard capture result');
    if (!current(draft, sidecar, lod) || value.source_revision !== active.revision
      || value.sidecar !== sidecar || value.lod !== lod
      || (active.capture && value.capture !== active.capture)) throw new Error('workshop.billboard.stale');
    active = { capture: value.capture, draft, revision: value.source_revision, sidecar: value.sidecar,
      lod: value.lod, value, changes: active.changes };
    return structuredClone(value);
  };
  return {
    async start(draft, sidecar, lod) {
      await call({ op: 'billboard-capture-cancel' }).catch(() => {});
      const revision = draft.sourceRevision;
      active = { draft, revision, sidecar, lod, capture: null, value: null };
      try {
        return accept(await call({ op: 'billboard-capture-start', files: draft.toNativeSources(),
          sidecar, lod, source_revision: revision }), draft, sidecar, lod);
      } catch (error) { active = null; throw error; }
    },
    async status(draft, sidecar, lod) {
      if (!active) return null;
      const held = active;
      try {
        if (!current(draft, sidecar, lod)) throw new Error('workshop.billboard.stale');
        const value = accept(await call({ op: 'billboard-capture-status' }), draft, sidecar, lod);
        if (!current(draft, sidecar, lod)) throw new Error('workshop.billboard.stale');
        return value;
      } catch (error) {
        await call({ op: 'billboard-capture-cancel' }).catch(() => {}); active = null; throw error;
      }
    },
    async adopt(draft, sidecar, lod) {
      const held = active;
      if (!held?.value || held.value.state !== 'ready' || !current(draft, sidecar, lod)) {
        throw new Error('workshop.billboard.stale');
      }
      try {
        loopbackImage(held.value);
        const base = loopbackMembers(held.value);
        const entries = await Promise.all(held.value.paths.map(async (path, index) => {
          const response = await fetcher(new URL(String(index), base), { cache: 'no-store', credentials: 'omit' });
          if (!response.ok) throw new Error('workshop.billboard.image_unavailable');
          const bytes = new Uint8Array(await response.arrayBuffer());
          if (path.endsWith('.png')) {
            if (bytes.length < 8 || bytes[0] !== 0x89 || String.fromCharCode(...bytes.slice(1, 4)) !== 'PNG') {
              throw new Error('workshop.billboard.invalid_image');
            }
            return [path, await upload(bytes), bytes];
          }
          return [path, new TextDecoder('utf-8', { fatal: true }).decode(bytes), bytes];
        }));
        const image = entries.find(([path]) => path === held.value.output);
        const bytes = image?.[2] ?? new Uint8Array();
        if (bytes.length < 8 || bytes[0] !== 0x89 || String.fromCharCode(...bytes.slice(1, 4)) !== 'PNG') {
          throw new Error('workshop.billboard.invalid_image');
        }
        const changes = entries.map(([path, after]) => ({ path, before: held.draft.members().get(path) ?? null, after }))
          .filter(change => !same(change.before, change.after));
        const candidate = restoreDocument(held.draft.snapshot()); candidate.apply(changes);
        const report = await runtime.validate(null, candidate);
        if (!report?.accepted) { const error = new Error('runtime-validation-refused'); error.report = report; throw error; }
        if (active !== held || !current(draft, sidecar, lod)
            || changes.some(change => !same(held.draft.members().get(change.path) ?? null, change.before))) {
          throw new Error('workshop.billboard.stale');
        }
        held.draft.apply(changes);
        active = null; await call({ op: 'billboard-capture-cancel' }).catch(() => {});
        return { path: held.value.output, paths: changes.map(change => change.path), report };
      } catch (error) {
        await call({ op: 'billboard-capture-cancel' }).catch(() => {}); active = null; throw error;
      }
    },
    async cancel() { active = null; await call({ op: 'billboard-capture-cancel' }); },
    get active() { return active ? structuredClone(active.value) : null; },
  };
}
