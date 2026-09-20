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
  const capture = createWorkshopBillboardCapture({ call, runtime,
    fetcher: vi.fn(async url => ({ ok: true, arrayBuffer: async () => members[Number(new URL(url).pathname.split('/').at(-1))].buffer })),
    upload: vi.fn(async () => ({ asset: `0000000000000002-${png.length}`, length: png.length })),
    restoreDocument: snapshot => WorkshopDocument.restore(snapshot, { native: true }) });
  return { draft, call, runtime, capture };
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
