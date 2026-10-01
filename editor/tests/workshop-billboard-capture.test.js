import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createWorkshopBillboardCapture } from '../workshop-billboard-capture.js';

const sidecar = 'assets/models/ship.model.toml';
const output = 'assets/models/ship_far.png';
const manifest = 'scripts/lod-capture-manifest.toml';
const png = Uint8Array.of(0x89, 0x50, 0x4e, 0x47, 13, 10, 26, 10, 1);
const capturedSidecar = '[[lod]]\nmodel="assets/models/ship.glb"\n[[lod]]\nbillboard="assets/models/ship_far.png"\nscale=[12.0, 5.0, 1.0]\n[lod.capture]\nsource="assets/models/ship.glb"\nyaw_views=8\nresolution=256\npitch=20\n';
const capturedManifest = 'version = 1\n';
const result = (state, revision = 0) => ({ status: 'billboard-capture', capture: 'capture-id', state,
  ...(state === 'ready' ? { image_url: 'http://127.0.0.1:7/workshop-billboard-capture/capture-id/0',
    base_url: 'http://127.0.0.1:7/workshop-billboard-capture/capture-id/', paths: [output, sidecar, manifest] } : { paths: [] }),
  output, sidecar, lod: 1, source_revision: revision, source: 'assets/models/ship.glb',
  yaw_views: 8, resolution: 256, pitch: 20 });

function fixture() {
  const draft = WorkshopDocument.fromNativeFiles({
    [sidecar]: '[[lod]]\nmodel="assets/models/ship.glb"\n[[lod]]\nbillboard="assets/models/ship_far.png"\n[lod.capture]\nsource="assets/models/ship.glb"\nyaw_views=8\nresolution=256\npitch=20\n',
    'assets/models/ship.glb': { asset: '0000000000000001-3', length: 3 },
  }, { kind: 'project' });
  const call = vi.fn(async request => request.op === 'billboard-capture-start' ? result('running')
    : request.op === 'billboard-capture-status' ? result('ready') : { status: 'done' });
  const runtime = { validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const members = [png, new TextEncoder().encode(capturedSidecar), new TextEncoder().encode(capturedManifest)];
  const dependencies = { call, runtime,
    fetcher: vi.fn(async url => ({ ok: true, arrayBuffer: async () => members[Number(new URL(url).pathname.split('/').at(-1))].buffer })),
    upload: vi.fn(async () => ({ asset: `0000000000000002-${png.length}`, length: png.length })),
    restoreDocument: snapshot => WorkshopDocument.restore(snapshot, { native: true }) };
  const capture = createWorkshopBillboardCapture(dependencies);
  return { draft, call, runtime, capture, dependencies };
}

describe('reviewed native Workshop billboard capture', () => {
  it('keeps the render outside the draft until one validated undoable adoption', async () => {
    const { draft, capture, runtime } = fixture();
    await capture.start(draft, sidecar, 1);
    expect(draft.read(output)).toBeUndefined();
    const ready = await capture.status(draft, sidecar, 1);
    expect(ready).toMatchObject({ state: 'ready', source_revision: 0, source: 'assets/models/ship.glb' });
    expect(draft.read(output)).toBeUndefined();
    await capture.adopt(draft, sidecar, 1);
    expect(runtime.validate).toHaveBeenCalledTimes(1);
    expect(draft.toNativeSources()[output]).toEqual({ asset: `0000000000000002-${png.length}`, length: png.length });
    expect(draft.read(sidecar)).toBe(capturedSidecar); expect(draft.read(manifest)).toBe(capturedManifest);
    expect(draft.undo()).toBe(output); expect(draft.toNativeSources()[output]).toBeUndefined();
    expect(draft.read(sidecar)).not.toBe(capturedSidecar); expect(draft.read(manifest)).toBeUndefined(); expect(draft.canUndo()).toBe(false);
  });

  it('rejects a late result after any draft edit and explicitly cancels the staged process', async () => {
    const { draft, capture, call } = fixture();
    await capture.start(draft, sidecar, 1);
    draft.edit(sidecar, `${draft.read(sidecar)}# changed\n`);
    await expect(capture.status(draft, sidecar, 1)).rejects.toThrow('workshop.billboard.stale');
    expect(draft.read(output)).toBeUndefined();
    expect(call).toHaveBeenCalledWith({ op: 'billboard-capture-cancel' });
  });

  it('leaves the draft untouched when runtime validation refuses adoption', async () => {
    const { draft, capture, runtime } = fixture(); runtime.validate.mockResolvedValue({ accepted: false, findings: [] });
    await capture.start(draft, sidecar, 1); await capture.status(draft, sidecar, 1);
    await expect(capture.adopt(draft, sidecar, 1)).rejects.toThrow('runtime-validation-refused');
    expect(draft.read(output)).toBeUndefined(); expect(draft.canUndo()).toBe(false);
  });

  it('rejects and cancels status when the selected sidecar or LOD changes', async () => {
    const { draft, capture, call } = fixture();
    await capture.start(draft, sidecar, 1);
    await expect(capture.status(draft, sidecar, 0)).rejects.toThrow('workshop.billboard.stale');
    expect(call).toHaveBeenCalledWith({ op: 'billboard-capture-cancel' });
  });
});

for (const stage of ['fetch', 'bytes', 'upload', 'validate']) for (const change of ['edit', 'cancel', 'replacement']) {
  it('rejects ' + change + ' while awaiting ' + stage + ' without affecting a newer job', async () => {
    const { draft, call, dependencies } = fixture();
    let release, entered;
    const waiting = new Promise(resolve => { release = resolve; });
    const started = new Promise(resolve => { entered = resolve; });
    let once = true;
    const pause = async () => { if (once) { once = false; entered(); await waiting; } };
    const fetcher = dependencies.fetcher, upload = dependencies.upload, validate = dependencies.runtime.validate;
    dependencies.fetcher = async (...args) => {
      if (stage === 'fetch') await pause();
      const response = await fetcher(...args);
      return { ...response, arrayBuffer: async () => { if (stage === 'bytes') await pause(); return response.arrayBuffer(); } };
    };
    dependencies.upload = async bytes => { if (stage === 'upload') await pause(); return upload(bytes); };
    dependencies.runtime.validate = async (...args) => { if (stage === 'validate') await pause(); return validate(...args); };
    const job = createWorkshopBillboardCapture(dependencies);
    await job.start(draft, sidecar, 1);
    await job.status(draft, sidecar, 1);
    const operation = job.adopt(draft, sidecar, 1);
    const rejected = expect(operation).rejects.toThrow('stale');
    await started;
    if (change === 'edit') draft.edit(sidecar, draft.read(sidecar) + '# later\n');
    if (change === 'cancel') await job.cancel();
    if (change === 'replacement') await job.start(draft, sidecar, 1);
    const cancels = call.mock.calls.filter(([row]) => row.op.endsWith('-cancel')).length;
    release(); await rejected;
    expect(draft.canUndo()).toBe(change === 'edit');
    if (change === 'replacement') {
      expect(job.active.state).toBe('running');
      expect(call.mock.calls.filter(([row]) => row.op.endsWith('-cancel'))).toHaveLength(cancels);
    }
  });
}

it('queues status after native start instead of reading a previous run', async () => {
  const { draft, capture: job, call } = fixture();
  const implementation = call.getMockImplementation();
  let release, entered;
  const waiting = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { entered = resolve; });
  call.mockImplementation(async request => {
    if (request.op.endsWith('-start')) { entered(); await waiting; }
    return implementation(request);
  });
  const starting = job.start(draft, sidecar, 1);
  const status = job.status(draft, sidecar, 1);
  await started;
  expect(call.mock.calls.some(([request]) => request.op.endsWith('-status'))).toBe(false);
  release(); await starting;
  expect((await status).state).toBe('ready');
});
