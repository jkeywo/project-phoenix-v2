import { createWorkshopAssetJob } from './workshop-asset-job.js';
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

export function createWorkshopLodGeneration(dependencies) {
  const job = createWorkshopAssetJob(dependencies, {
    operation: 'lod-generate', stale: 'workshop.lod_generation.stale', notReady: 'workshop.lod_generation.not_ready', reviewOnStatus: true,
    id: value => value.run, matches: (value, selection) => value.sidecar === selection.sidecar,
    validate(value) { if (!valid(value)) throw new Error('Invalid native LOD generation result'); },
    route(value) {
      const base = route(value);
      if (!value.paths.includes('scripts/lod-manifest.toml') || value.required_paths.some(path => !value.paths.includes(path))) {
        throw new Error('Generated LOD review is incomplete');
      }
      return base;
    },
    binary: isWorkshopBinary, validateMember() {},
    unavailable: path => 'Generated LOD review member is unavailable: ' + path,
    adopted: (_value, result) => ({ changed: result.applied, paths: result.changes.map(change => change.path), report: result.report }),
  });
  return {
    start: (draft, sidecar, { remesh = false } = {}) => job.start(draft, { sidecar }, { remesh: Boolean(remesh) }),
    status: (draft, sidecar) => job.status(draft, { sidecar }),
    reviewDraft: (draft, sidecar) => job.reviewDraft(draft, { sidecar }),
    adopt: (draft, sidecar) => job.adopt(draft, { sidecar }),
    cancel: () => job.cancel(),
    get active() { return job.active; },
  };
}
