// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createBrowserWorkshopPreview } from '../../editor/workshop-preview.js';
import { createNativeWorkshopProvider } from '../../editor/workshop-provider.js';

const DRAFT = {
  paths: () => ['assets/models/courier.glb', 'assets/models/courier.toml'],
  isBinary: path => path.endsWith('.glb'),
  bytes: () => new Uint8Array([1, 2, 3]),
  read: () => '[rig]\n',
};

function harness({ started = { running: true, stats: null } } = {}) {
  const run = {
    start: vi.fn(async () => started),
    control: vi.fn(async () => ({ running: true })),
    status: vi.fn(async () => ({ running: true })),
    destroy: vi.fn(),
  };
  const prepare = vi.fn(async (files, selection) => ({ files, selection, revision: 'abc' }));
  const preview = createBrowserWorkshopPreview({ prepare, frame: () => run });
  return { preview, prepare, run };
}

describe('createBrowserWorkshopPreview', () => {
  beforeEach(() => { document.body.innerHTML = ''; });

  it('captures the draft exactly as the disposable Test does', () => {
    const { preview } = harness();
    const captured = preview.capture(DRAFT);
    // Same paths, text as text and bytes as bytes. A preview that captured
    // differently from Test would be a second definition of "captured", which
    // is how an uncaptured project file gets read.
    expect(Object.keys(captured)).toEqual(DRAFT.paths());
    expect(captured['assets/models/courier.glb']).toBeInstanceOf(Uint8Array);
    expect(typeof captured['assets/models/courier.toml']).toBe('string');
    const test = readFileSync('editor/workshop-test-frame.js', 'utf8');
    expect(test).toContain("draft.isBinary(path) ? draft.bytes(path) : draft.read(path)");
  });

  it('runs the captured draft through the shared preparation before rendering', async () => {
    const { preview, prepare, run } = harness();
    const captured = preview.capture(DRAFT);
    await preview.start(captured, { model: 'assets/models/courier.glb' });
    // The dependency merger is the Test's, so base content and other packs
    // resolve the same way and nothing is fetched.
    expect(prepare).toHaveBeenCalledWith(captured, { model: 'assets/models/courier.glb' });
    expect(run.start).toHaveBeenCalledWith({
      files: captured, selection: { model: 'assets/models/courier.glb' }, revision: 'abc',
    });
  });

  it('refuses a candidate without destroying the picture already on screen', async () => {
    const { preview, run } = harness();
    await preview.start({}, { model: 'a.glb' });
    const failing = createBrowserWorkshopPreview({
      prepare: async () => { throw new Error('refused'); }, frame: () => run,
    });
    await expect(failing.start({}, { model: 'b.glb' })).rejects.toThrow('refused');
    // The first run is still the current one: a refused replacement is not a
    // reason to take away what the operator was looking at.
    expect(run.destroy).not.toHaveBeenCalled();
  });

  it('retires the run when a control or status says it stopped', async () => {
    const { preview, run } = harness();
    await preview.start({}, { model: 'a.glb' });
    run.control.mockResolvedValueOnce({ running: false });
    await preview.control({ command: 'gizmos', enabled: false });
    expect(run.destroy).toHaveBeenCalled();
    // And a retired run answers as stopped rather than reaching a dead frame.
    expect(await preview.status()).toEqual({ running: false });
  });

  it('stops by destroying the frame, which retires the whole WASM instance', async () => {
    const { preview, run } = harness();
    await preview.start({}, { model: 'a.glb' });
    await preview.stop();
    expect(run.destroy).toHaveBeenCalled();
    expect(await preview.status()).toEqual({ running: false });
  });

  it('refuses to start when the page reports it could not run', async () => {
    const { preview } = harness({ started: { running: false, error: 'boom' } });
    await expect(preview.start({}, { model: 'a.glb' })).rejects.toThrow('boom');
  });
});

describe('native Workshop preview producer', () => {
  it('fetches immutable capture members instead of carrying merged bytes over JSON and retires them', async () => {
    const requests = [];
    const request = vi.fn(async value => {
      requests.push(value);
      if (value.op === 'preview-start') return {
        status: 'preview', capture: 'capture-1',
        base_url: 'http://127.0.0.1:8123/workshop-preview-capture/capture-1/',
        paths: ['assets/models/draft-only.glb', 'assets/models/draft-only.model.toml'],
        revision: 'native-revision', selection: value.selection,
      };
      return { status: 'done' };
    });
    const binary = new Uint8Array([1, 2, 3, 4]);
    const sidecar = new TextEncoder().encode('[rig]\n');
    const fetcher = vi.fn(async url => ({ ok: true,
      arrayBuffer: async () => (String(url).endsWith('/0') ? binary : sidecar).buffer }));
    const run = { start: vi.fn(async snapshot => ({ running: true, revision: snapshot.revision })),
      control: vi.fn(), status: vi.fn(async () => ({ running: true })), destroy: vi.fn() };
    const provider = createNativeWorkshopProvider({ request, fetcher,
      previewFrame: () => run });
    provider.modelPreview.mount(document.body, { title: 'Preview' });
    const compact = { 'assets/models/draft-only.glb': { version: 'abc', length: 4 } };
    expect(provider.modelPreview.capture({ toNativeSources: () => compact })).toBe(compact);
    await provider.modelPreview.start(compact, { model: 'assets/models/draft-only.glb', gizmos: true });
    expect(requests[0]).toEqual({ op: 'preview-start', files: compact,
      selection: { model: 'assets/models/draft-only.glb', gizmos: true } });
    expect(fetcher.mock.calls.map(([url]) => String(url))).toEqual([
      'http://127.0.0.1:8123/workshop-preview-capture/capture-1/0',
      'http://127.0.0.1:8123/workshop-preview-capture/capture-1/1',
    ]);
    expect(run.start.mock.calls[0][0]).toMatchObject({ revision: 'native-revision' });
    expect(run.start.mock.calls[0][0].files['assets/models/draft-only.glb']).toEqual(binary);
    expect(run.start.mock.calls[0][0].files['assets/models/draft-only.model.toml']).toBe('[rig]\n');
    await provider.modelPreview.stop();
    expect(requests.slice(1)).toEqual([
      { op: 'preview-release', capture: 'capture-1' }, { op: 'preview-stop' },
    ]);
  });
});

describe('the preview subject', () => {
  it('prepares a preview WITHOUT the Test launch check', async () => {
    // The Test's `prepare` puts its selection through checkTestSelection, whose
    // decoder takes a world/ship/seed Launch and refuses unknown fields — so a
    // {model, variant} preview selection is rejected before anything renders.
    // The preview shares the validation, the dependency merge and the
    // fingerprint, and skips exactly that one step.
    const snapshot = readFileSync('editor/workshop-test-snapshot.js', 'utf8');
    expect(snapshot).toContain('prepareSubject');
    const body = snapshot.slice(snapshot.indexOf('async prepareSubject'));
    expect(body.slice(0, body.indexOf('}'))).not.toContain('checkTestSelection');
    const provider = readFileSync('editor/workshop-provider.js', 'utf8');
    expect(provider).toContain('prepare: preparation.prepareSubject');
  });

  it('offers entity templates, which is how stars and planets are reachable', async () => {
    const { entityDocuments } = await import('../../editor/workshop-models.js');
    const paths = [
      'assets/entities/alliance_courier.toml',
      'assets/entities/sol_star.toml',
      'assets/entities/terra_planet.toml',
      'assets/models/courier.glb',
      // A rig sidecar is not a template: it is reached through its model.
      'assets/models/courier.model.toml',
    ];
    expect(entityDocuments(paths)).toEqual([
      'assets/entities/alliance_courier.toml',
      'assets/entities/sol_star.toml',
      'assets/entities/terra_planet.toml',
    ]);
    // The viewer dispatches [star], [planet] and [mesh] from the template
    // itself, so there is no separate star or planet list to drift.
    const subject = readFileSync('src/viewer/subject.rs', 'utf8');
    expect(subject).toContain('insert_star_visual');
    expect(subject).toContain('insert_planet_visual');
  });

  it('sends an entity subject as an entity, and a model as a model', () => {
    const panel = readFileSync('gui/workshop-models-panel.js', 'utf8');
    expect(panel).toContain("if (subject.value.startsWith('assets/entities/')) return { entity: subject.value };");
    // And the preview panel lets either kind be refreshed, rather than gating
    // on a model that an entity subject does not have.
    const preview = readFileSync('gui/workshop-model-preview-panel.js', 'utf8');
    expect(preview).toContain('!(chosen.model || chosen.entity)');
  });

  it('boots the renderer with the settings its own controls claim', () => {
    // A reading that claimed gizmos were on while the App booted with them off
    // would make the first frame contradict its own checkbox.
    const protocol = readFileSync('src/workshop/test_protocol.rs', 'utf8');
    expect(protocol).toContain('#[serde(default = "gizmos_on")]');
    const runtime = readFileSync('editor/workshop-preview-runtime.js', 'utf8');
    expect(runtime).toContain("lighting: 'ambient'");
    // Which is the viewer's own default, not a guess.
    const lighting = readFileSync('src/viewer/lighting.rs', 'utf8');
    expect(lighting.replace(/\s+/g, ' ')).toContain('#[default] Ambient,');
  });
});

describe('the preview page and its target', () => {
  it('builds with the viewer feature and reads nothing from disk', () => {
    const page = readFileSync('workshop-preview.html', 'utf8');
    // The shared viewer, and only it: no server bridge, no lobby, no renderer.
    expect(page).toContain('data-cargo-no-default-features');
    expect(page).toContain('data-cargo-features="viewer"');
    // No asset copy and no proxy: this page renders the bytes it was handed.
    // (The comment in the page explains the absence, so this looks for the
    // actual link rather than the word.)
    expect(page).not.toMatch(/<link[^>]*rel="copy-dir"/);
    const config = readFileSync('workshop-preview-trunk.toml', 'utf8');
    expect(config).toContain('target = "workshop-preview.html"');
    expect(config).not.toContain('[[proxy]]');
  });

  it('accepts only local renderer commands from its parent', async () => {
    const boot = readFileSync('gui/workshop-preview-boot.js', 'utf8');
    // The parent holds a whole captured draft; the only thing it may ask for is
    // where to point the camera and how to light what is already there.
    for (const command of ['lod', 'lighting', 'gizmos', 'distance', 'camera']) {
      expect(boot).toContain(`'${command}'`);
    }
    // Its own connect type, so a Test port cannot drive a preview or vice versa.
    expect(boot).toContain('phoenix-workshop-preview-connect');
    const frame = readFileSync('editor/workshop-preview.js', 'utf8');
    expect(frame).toContain('phoenix-workshop-preview-connect');
  });

  it('is published beside the pages that open it', () => {
    const build = readFileSync('scripts/build-workshop.mjs', 'utf8');
    expect(build).toContain("'workshop-preview'");
    expect(build).toContain("'workshop-preview-runtime'");
    const ci = readFileSync('.github/workflows/ci.yml', 'utf8');
    expect(ci).toContain('workshop-preview-trunk.toml');
    expect(ci).toContain('copy-workshop-preview.mjs');
  });
});
