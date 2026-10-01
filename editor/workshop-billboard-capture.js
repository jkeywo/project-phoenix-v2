import { createWorkshopAssetJob } from './workshop-asset-job.js';
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

export function createWorkshopBillboardCapture(dependencies) {
  const job = createWorkshopAssetJob(dependencies, {
    operation: 'billboard-capture', stale: 'workshop.billboard.stale', emptyStatus: true,
    id: value => value.capture, matches: (value, selection) => value.sidecar === selection.sidecar && value.lod === selection.lod,
    validate(value) { if (!validResult(value)) throw new Error('Invalid native billboard capture result'); },
    route(value) { loopbackImage(value); return loopbackMembers(value); },
    binary: path => path.endsWith('.png'),
    validateMember(path, bytes) {
      if (path.endsWith('.png') && (bytes.length < 8 || bytes[0] !== 0x89 || String.fromCharCode(...bytes.slice(1, 4)) !== 'PNG')) {
        throw new Error('workshop.billboard.invalid_image');
      }
    },
    unavailable: () => 'workshop.billboard.image_unavailable',
    adopted: (value, result) => ({ path: value.output, paths: result.changes.map(change => change.path), report: result.report }),
  });
  return {
    start: (draft, sidecar, lod) => job.start(draft, { sidecar, lod }),
    status: (draft, sidecar, lod) => job.status(draft, { sidecar, lod }),
    adopt: (draft, sidecar, lod) => job.adopt(draft, { sidecar, lod }),
    cancel: () => job.cancel(),
    get active() { return job.active; },
  };
}
