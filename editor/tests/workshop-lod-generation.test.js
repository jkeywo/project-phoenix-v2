import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createWorkshopLodGeneration } from '../workshop-lod-generation.js';

const sidecar = 'assets/models/ship.model.toml';
const generated = 'assets/models/ship_lod1.glb';
const manifest = 'scripts/lod-manifest.toml';
const sidecarText = `[[lod]]\nmodel='assets/models/ship.glb'\nmax_distance=20\n[[lod]]\nmodel='${generated}'\n[lod.generate]\nsource='assets/models/ship.glb'\nratio=0.25\n`;
const glb = Uint8Array.of(0x67, 0x6c, 0x54, 0x46, 1);
const manifestText = 'version = 1\n[[output]]\npath="assets/models/ship_lod1.glb"\n';
const response = state => ({ status: 'lod-generation', run: 'run-id', state, progress: ['simplify 0.25'],
  sidecar, source_revision: 0, ...(state === 'ready' ? {
    base_url: 'http://127.0.0.1:7/workshop-lod-generation/run-id/', paths: [sidecar, generated, manifest],
  } : { paths: [] }), required_paths: [generated] });

function fixture() {
  const draft = WorkshopDocument.fromNativeFiles({
    [sidecar]: sidecarText,
    'assets/models/ship.glb': { asset: '0000000000000001-4', length: 4 },
    [manifest]: 'version = 1\n',
  }, { kind: 'project' });
  const call = vi.fn(async request => request.op === 'lod-generate-start' ? response('running')
    : request.op === 'lod-generate-status' ? response('ready') : { status: 'done' });
  const members = [new TextEncoder().encode(sidecarText), glb, new TextEncoder().encode(manifestText)];
  const runtime = { validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const generation = createWorkshopLodGeneration({ call, runtime,
    fetcher: vi.fn(async url => ({ ok: true, arrayBuffer: async () => members[Number(url.pathname.split('/').at(-1))].buffer })),
    upload: vi.fn(async bytes => ({ asset: `0000000000000002-${bytes.length}`, length: bytes.length })),
    restoreDocument: snapshot => WorkshopDocument.restore(snapshot, { native: true }) });
  return { draft, call, runtime, generation };
}

describe('native Workshop selected-model LOD generation', () => {
  it('reviews a validated candidate and adopts GLB plus manifest as one undo entry', async () => {
    const { draft, generation, runtime } = fixture();
    await generation.start(draft, sidecar, { remesh: true });
    const ready = await generation.status(draft, sidecar);
    expect(ready).toMatchObject({ state: 'ready', reviewReady: true });
    expect(draft.toNativeSources()[generated]).toBeUndefined();
    expect(generation.reviewDraft(draft, sidecar).toNativeSources()[generated]).toEqual({ asset: '0000000000000002-5', length: 5 });
    expect(runtime.validate).toHaveBeenCalledTimes(1);
    await generation.adopt(draft, sidecar);
    expect(draft.toNativeSources()[generated]).toEqual({ asset: '0000000000000002-5', length: 5 });
    expect(draft.read(manifest)).toBe(manifestText);
    draft.undo(); expect(draft.toNativeSources()[generated]).toBeUndefined(); expect(draft.read(manifest)).toBe('version = 1\n');
    expect(draft.canUndo()).toBe(false);
  });

  it('refuses a changed local selection and cancels the staged run without touching the draft', async () => {
    const { draft, generation, call } = fixture();
    await generation.start(draft, sidecar);
    await expect(generation.status(draft, 'assets/models/other.model.toml')).rejects.toThrow('workshop.lod_generation.stale');
    expect(call).toHaveBeenCalledWith({ op: 'lod-generate-cancel' });
    expect(draft.toNativeSources()[generated]).toBeUndefined(); expect(draft.canUndo()).toBe(false);
  });

  it('cleans the pending review when candidate validation refuses', async () => {
    const { draft, generation, runtime, call } = fixture(); runtime.validate.mockResolvedValue({ accepted: false, findings: [] });
    await generation.start(draft, sidecar);
    await expect(generation.status(draft, sidecar)).rejects.toThrow('runtime-validation-refused');
    expect(call).toHaveBeenCalledWith({ op: 'lod-generate-cancel' });
    expect(draft.toNativeSources()[generated]).toBeUndefined();
  });

  it('refuses a ready result whose remesh cannot mask a missing required generated level', async () => {
    const { draft, generation, call } = fixture();
    call.mockImplementation(async request => request.op === 'lod-generate-start' ? response('running')
      : request.op === 'lod-generate-status' ? { ...response('ready'),
        paths: [sidecar, 'assets/models/ship.remesh.glb', manifest] } : { status: 'done' });
    await generation.start(draft, sidecar, { remesh: true });
    await expect(generation.status(draft, sidecar)).rejects.toThrow('Generated LOD review is incomplete');
    expect(call).toHaveBeenCalledWith({ op: 'lod-generate-cancel' });
    expect(draft.toNativeSources()[generated]).toBeUndefined();
  });
});
