// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountWorkshopAuthoring } from '../../gui/workshop-authoring.js';
import { createNativeWorkshopProvider } from '../../editor/workshop-provider.js';
import { t } from '../../gui/strings.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';

const queue = readFileSync('src/native_host/workshop/queue.js', 'utf8');
const boot = readFileSync('src/native_host/workshop/boot.js', 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const runBoot = new AsyncFunction('window', 'document', 'mountNativeWorkshop', 'applyToDom', boot.replace(/^import .*;\r?\n/gm, ''));
let dispose;
const drain = () => window.__phoenixNativeWorkshopDrain().split('\n').filter(Boolean);
afterEach(() => {
  window.dispatchEvent(new Event('pagehide'));
  for (const key of ['__phoenixNativeWorkshopSend', '__phoenixNativeWorkshopDrain', '__phoenixNativeWorkshopReply', '__phoenixNativeWorkshopKey',
    '__phoenixOperatorReply', 'PhoenixOperatorStorage', 'PhoenixOperatorStorageStatus']) delete window[key];
  vi.restoreAllMocks();
});

describe('native Workshop shared boot', () => {
  it('loads durable preferences before mounting and reports live only after source startup settles', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    window.eval(queue);
    let completeSource;
    const ready = new Promise(resolve => { completeSource = resolve; });
    const receive = vi.fn();
    dispose = vi.fn();
    const mount = vi.fn(() => ({ ready, receive, dispose }));
    const apply = vi.fn();
    const pending = runBoot(window, document, mount, apply);
    expect(drain().map(JSON.parse)).toEqual([{ type: 'NativeOperator', operation: 'load' }]);
    expect(mount).not.toHaveBeenCalled();
    const profile = '{"kind":"project-phoenix/operator-profile","version":1}';
    window.__phoenixOperatorReply({ operation: 'load', status: 'ok', profile });
    await vi.waitFor(() => expect(mount).toHaveBeenCalledOnce());
    expect(window.PhoenixOperatorStorage.getItem('phoenix-operator-profile-v1')).toBe(profile);
    expect(apply).toHaveBeenCalledWith(document);
    window.__phoenixNativeWorkshopReply({ id: 1, status: 'sources' });
    expect(receive).toHaveBeenCalledWith({ id: 1, status: 'sources' });
    expect(drain()).toEqual([]);
    completeSource();
    await pending;
    expect(drain()).toEqual(['NativeWorkshopReady']);
    window.PhoenixOperatorStorage.setItem('phoenix-operator-profile-v1', profile);
    expect(drain().map(JSON.parse)).toEqual([{ type: 'NativeOperator', operation: 'save', profile }]);
    expect(() => window.PhoenixOperatorStorage.setItem('unrelated', 'value')).toThrow();
    window.dispatchEvent(new Event('pagehide'));
    expect(dispose).toHaveBeenCalledOnce();
  });

  it('mounts the same docked panel hierarchy after native profile startup', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    window.eval(queue);
    const provider = { load: async () => null, runtime: {}, recovery: { load: async () => null } };
    const pending = runBoot(window, document, ({ root }) => ({ ...mountWorkshopAuthoring({ root, provider }), receive: vi.fn() }), vi.fn());
    drain();
    window.__phoenixOperatorReply({ operation: 'load', status: 'ok', profile: null });
    await pending;
    expect([...document.querySelectorAll('.workshop-dock-panel')].map(node => node.dataset.panel))
      .toEqual(['files', 'dependencies', 'changes', 'composition', 'source', 'findings', 'feedback', 'model-preview',
        'inspector', 'add', 'recovery', 'settings', 'models', 'sound', 'definitions']);
  });

  it('mounts with visible storage status when preference loading fails and bounds the private queue', async () => {
    window.eval(queue);
    const mount = vi.fn(() => ({ ready: Promise.resolve(), receive: vi.fn(), dispose: vi.fn() }));
    const pending = runBoot(window, document, mount, vi.fn());
    window.__phoenixNativeWorkshopDrain();
    const failure = { operation: 'load', status: 'error', error: 'Unavailable' };
    window.__phoenixOperatorReply(failure);
    await pending;
    expect(window.PhoenixOperatorStorageStatus).toEqual(failure);
    expect(window.PhoenixOperatorStorage.getItem('phoenix-operator-profile-v1')).toBeNull();
    window.__phoenixNativeWorkshopDrain();
    for (let i = 0; i < 8; i++) window.__phoenixNativeWorkshopSend(String(i));
    expect(() => window.__phoenixNativeWorkshopSend('overflow')).toThrow();
    expect(drain()).toEqual(['0', '1', '2', '3', '4', '5', '6', '7']);
    expect(() => window.__phoenixNativeWorkshopSend({})).toThrow();
  });

  it('the shared authoring surface reads native profile storage and keeps its refusal visible', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    const getItem = vi.fn(() => null);
    window.PhoenixOperatorStorage = { getItem, setItem: vi.fn() };
    window.PhoenixOperatorStorageStatus = { status: 'error' };
    const workspace = mountWorkshopAuthoring({ root: document.getElementById('workshop'), recovery: { load: async () => null } });
    try {
      await workspace.ready;
      expect(getItem).toHaveBeenCalledWith('phoenix-operator-profile-v1');
      expect(document.querySelector('.workshop-findings').textContent).toBe(t('editor.mod.settings.storage_refused'));
    } finally { workspace.dispose(); }
  });

  it('native modifier keys execute the shared chronological undo without inserting shortcut text', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    window.eval(queue);
    const source = '# retained\n[global]\ntitle="Native"\n';
    const draft = WorkshopDocument.fromNativeFiles({ 'assets/worlds/test.toml': source }, { kind: 'project' });
    const provider = { save: vi.fn(), load: async () => draft, runtime: {},
      recovery: { load: async () => null, save: async () => {} } };
    const pending = runBoot(window, document, ({ root }) => {
      const authoring = mountWorkshopAuthoring({ root, provider });
      return { ...authoring, receive: vi.fn() };
    }, vi.fn());
    drain();
    window.__phoenixOperatorReply({ operation: 'load', status: 'ok', profile: null });
    await pending;
    const input = document.getElementById('workshop-source');
    input.focus(); input.value = `${source}# edit\n`; input.dispatchEvent(new Event('input'));
    const key = { code: 'KeyZ', key: 'z', pressed: true, ctrlKey: true, repeat: false };
    expect(window.__phoenixNativeWorkshopKey(key)).toBe(true);
    expect(input.value).toBe(source);
    expect(window.__phoenixNativeWorkshopKey({ ...key, shiftKey: true })).toBe(true);
    expect(input.value).toBe(`${source}# edit\n`);
    const released = vi.fn();
    input.addEventListener('keyup', released, { once: true });
    expect(window.__phoenixNativeWorkshopKey({ ...key, pressed: false })).toBe(false);
    expect(released.mock.calls[0][0].code).toBe('KeyZ');
    expect(released.mock.calls[0][0].ctrlKey).toBe(true);
  });

  it('runs the restricted native lifecycle through the real provider bridge and dock controls', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    const worldPath = 'assets/worlds/test.toml';
    const entityPath = 'assets/entities/test.toml';
    const recoveredPath = 'assets/scripts/recovered.rhai';
    const addedPath = 'assets/scripts/added.rhai';
    const assetPath = 'assets/models/imported.glb';
    const diskWorld = '[global]\ntitle="Disk"\n';
    const recoveredWorld = '[global]\ntitle="Recovered"\n';
    const entitySource = '[entity]\nname="Second document"\n';
    const recoveredSource = 'fn recovered() {\n  print("yes");\n}\n';
    const addedSource = 'fn added() {\n  print("new");\n}\n';
    const recovered = WorkshopDocument.fromNativeFiles({
      [worldPath]: diskWorld,
      [entityPath]: entitySource,
    }, { kind: 'project' });
    recovered.edit(worldPath, recoveredWorld);
    recovered.put(recoveredPath, recoveredSource);
    const recoveredSnapshot = recovered.snapshot();
    recoveredSnapshot.sourceFiles = recoveredSnapshot.sourceFiles.map(([path, text]) => [path, [...new TextEncoder().encode(text)]]);
    const recoveryRecord = JSON.stringify({ version: 1, selected: entityPath, draft: recoveredSnapshot });
    const requests = [];
    const request = async request => {
      requests.push(request);
      if (request.op === 'load-sources') return {
        status: 'sources', kind: 'project', revision: 'disk-r1', files: {
          [worldPath]: diskWorld,
          [entityPath]: entitySource,
        },
      };
      if (request.op === 'recovery-load') return {
        status: 'recovery', recovery: { revision: 'recovered-r1', record: recoveryRecord },
      };
      if (request.op === 'validate-sources') return {
        status: 'validated', report: { accepted: true, findings: [
          { file: addedPath, line: 1, severity: 'warning', message: 'accepted native source' },
        ] },
      };
      if (request.op === 'load-dependencies') return {
        status: 'dependencies', base_files: { 'assets/base.toml': 'immutable = true\n' }, packs: [],
      };
      if (request.op === 'asset-begin') return { status: 'asset-upload', token: 'asset-1' };
      if (request.op === 'asset-chunk') return { status: 'done' };
      if (request.op === 'asset-finish') return {
        status: 'asset-stored', reference: { asset: '0000000000000001-4', length: 4 },
      };
      if (request.op === 'save-sources') return { status: 'saved', revision: 'saved-r2' };
      if (request.op === 'test-catalog') return {
        status: 'test-catalog', catalog: { worlds: [], ships: [] },
      };
      return { status: 'done' };
    };
    const provider = createNativeWorkshopProvider({ request });
    const mounted = mountWorkshopAuthoring({ root: document.getElementById('workshop'), provider });
    try {
      await mounted.ready;
      expect(document.getElementById('workshop-new').hidden).toBe(true);
      expect(document.getElementById('workshop-import').hidden).toBe(true);
      expect(document.getElementById('workshop-export').hidden).toBe(true);

      document.querySelector('[data-layout-panel="recovery"][role="tab"]').click();
      expect(document.getElementById('workshop-recovery-status').textContent).toBe(t('workshop.native_recovery_available'));
      document.getElementById('workshop-restore').click();
      const files = document.getElementById('workshop-files');
      const source = document.getElementById('workshop-source');
      await vi.waitFor(() => expect(source.value).toBe(entitySource));
      expect(files.value).toBe(entityPath);
      expect([...files.options].map(option => option.value)).toEqual([worldPath, entityPath, recoveredPath]);

      document.querySelector('[data-layout-panel="dependencies"][role="tab"]').click();
      document.getElementById('workshop-dependencies-load').click();
      await vi.waitFor(() => expect(document.getElementById('workshop-dependency-source').value).toContain('immutable'));
      expect(document.getElementById('workshop-dependency-source').readOnly).toBe(true);

      document.getElementById('workshop-undo').click();
      expect(files.value).toBe(worldPath);
      expect(source.value).toBe(recoveredWorld);
      expect([...files.options].map(option => option.value)).toEqual([worldPath, entityPath]);
      document.getElementById('workshop-redo').click();
      expect(files.value).toBe(recoveredPath);
      expect(source.value).toBe(recoveredSource);
      document.getElementById('workshop-undo').click();
      document.getElementById('workshop-undo').click();
      expect(files.value).toBe(worldPath);
      expect(source.value).toBe(diskWorld);
      document.getElementById('workshop-redo').click();
      expect(source.value).toBe(recoveredWorld);
      document.getElementById('workshop-redo').click();
      expect(files.value).toBe(recoveredPath);
      expect(source.value).toBe(recoveredSource);

      files.value = entityPath;
      files.dispatchEvent(new Event('change'));
      expect(source.value).toBe(entitySource);

      document.querySelector('[data-layout-panel="add"][role="tab"]').click();
      document.getElementById('workshop-add-path').value = addedPath;
      document.getElementById('workshop-add-source').click();
      expect(files.value).toBe(addedPath);
      expect(source.value).toBe('');
      source.value = addedSource;
      source.dispatchEvent(new Event('input'));

      document.getElementById('workshop-check').click();
      await vi.waitFor(() => expect(requests.some(request => request.op === 'validate-sources')).toBe(true));
      expect(requests.find(request => request.op === 'validate-sources').files).toEqual({
        [worldPath]: recoveredWorld,
        [entityPath]: entitySource,
        [recoveredPath]: recoveredSource,
        [addedPath]: addedSource,
      });
      await vi.waitFor(() => expect(document.activeElement).toBe(document.querySelector('.workshop-findings')));
      const acceptedFindings = document.querySelector('.workshop-findings');
      const acceptedFeedback = document.querySelector('[data-action-id="editor.mod.validate"]');
      expect(acceptedFindings.textContent).toContain('accepted native source');
      expect(acceptedFindings.dataset.outcome).toBe('applied');
      expect(acceptedFeedback.dataset.state).toBe('Applied');
      expect(acceptedFindings.dataset.producingAction).toBe('editor.mod.validate');
      expect(acceptedFindings.dataset.producingCorrelation).toBe(acceptedFeedback.dataset.correlation);

      document.getElementById('workshop-add-path').value = assetPath;
      const assetInput = document.querySelector('.workshop-add input[type="file"]');
      const bytes = Uint8Array.of(0, 255, 13, 10);
      Object.defineProperty(assetInput, 'files', { configurable: true, value: [{
        size: bytes.length,
        slice(start, end) { return { arrayBuffer: async () => bytes.slice(start, end).buffer }; },
      }] });
      assetInput.dispatchEvent(new Event('change'));
      await vi.waitFor(() => expect(requests.some(request => request.op === 'asset-finish')).toBe(true));
      expect(requests.find(request => request.op === 'asset-chunk').bytes).toEqual([...bytes]);

      document.getElementById('workshop-save').click();
      await vi.waitFor(() => expect(requests.some(request => request.op === 'save-sources')).toBe(true));
      const save = requests.find(request => request.op === 'save-sources');
      expect(save.expected_revision).toBe('recovered-r1');
      expect(save.files).toEqual({
        [worldPath]: recoveredWorld,
        [entityPath]: entitySource,
        [recoveredPath]: recoveredSource,
        [addedPath]: addedSource,
        [assetPath]: { asset: '0000000000000001-4', length: 4 },
      });
      expect(document.querySelectorAll('#workshop-save')).toHaveLength(1);
    } finally { mounted.dispose(); }
  });

  it('keeps refused native validation correlated, focused, and on the initiating draft', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    const path = 'assets/worlds/test.toml';
    const original = '[global]\ntitle="Original"\n';
    const edited = '[global\ntitle="Retained"\n';
    const request = vi.fn(async request => {
      if (request.op === 'load-sources') return {
        status: 'sources', kind: 'project', revision: 'r1', files: { [path]: original },
      };
      if (request.op === 'recovery-load') return { status: 'recovery', recovery: null };
      if (request.op === 'validate-sources') return {
        status: 'refused', message: 'validation refused', report: { accepted: false, findings: [
          { file: path, line: 1, severity: 'error', message: 'invalid table' },
        ] },
      };
      return { status: 'done' };
    });
    const mounted = mountWorkshopAuthoring({ root: document.getElementById('workshop'),
      provider: createNativeWorkshopProvider({ request }) });
    try {
      await mounted.ready;
      const source = document.getElementById('workshop-source');
      source.value = edited;
      source.dispatchEvent(new Event('input'));
      document.querySelector('[data-panel="findings"] [data-layout-control="close"]').click();
      document.getElementById('workshop-check').click();
      await vi.waitFor(() => expect(document.querySelector('.workshop-findings')?.dataset.outcome).toBe('refused'));
      const findings = document.querySelector('.workshop-findings');
      expect(document.activeElement).toBe(findings);
      expect(source.value).toBe(edited);
      expect(findings.textContent).toContain('invalid table');
      expect(findings.dataset.producingAction).toBe('editor.mod.validate');
      const feedback = document.querySelector('[data-action-id="editor.mod.validate"]');
      expect(feedback.dataset.state).toBe('Refused');
      expect(findings.dataset.producingCorrelation).toBe(feedback.dataset.correlation);
    } finally { mounted.dispose(); }
  });
});
