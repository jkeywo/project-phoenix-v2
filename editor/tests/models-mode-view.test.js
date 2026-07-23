// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest';
import { mountModelsMode } from '../models-mode-view.js';
import { ModeShell } from '../mode-shell.js';
import { SaveFlow } from '../save-flow.js';
import { parseSidecarName, wireRigIndexToSaves } from '../models-rig.js';
import { InvalidationBus } from '../invalidation-bus.js';
import { RigIndex } from '../marker-validate.js';

/**
 * A stub rig-view controller (no Three/WebGL). Records calls and returns
 * canned extents so the mount's data flow can be exercised in jsdom.
 */
function makeSceneStub() {
  const calls = {
    addMarker: [], removeMarker: [], select: [],
    setBase: 0, clearMarkers: 0, frame: 0, resize: 0, dispose: 0,
    order: [],
  };
  let changeCb = null;
  const controller = {
    loadModel: async () => controller.getExtents(),
    setBase: () => { calls.setBase += 1; calls.order.push('setBase'); return controller.getExtents(); },
    getExtents: () => ({ min: [-2, -1, -3], max: [2, 1, 3], size: [4, 2, 6] }),
    addMarker: (name, m) => { calls.addMarker.push([name, m]); calls.order.push(`addMarker:${name}`); },
    removeMarker: (name) => calls.removeMarker.push(name),
    clearMarkers: () => { calls.clearMarkers += 1; calls.order.push('clearMarkers'); },
    select: (name) => calls.select.push(name),
    setGizmoMode: () => {},
    onChange: (cb) => { changeCb = cb; },
    frame: () => { calls.frame += 1; calls.order.push('frame'); },
    resize: () => { calls.resize += 1; },
    dispose: () => { calls.dispose += 1; },
    _fireChange: (name, m) => changeCb && changeCb(name, m),
    _calls: calls,
  };
  return controller;
}

function makeIo({ files = {} } = {}) {
  const writes = {};
  return {
    writes,
    io: {
      readFile: async (p) => {
        if (p in files) return files[p];
        throw new Error(`ENOENT ${p}`);
      },
      writeFile: async (p, content) => { writes[p] = content; },
      readBinaryFile: async () => new ArrayBuffer(8),
      listDirectory: async (dir) => {
        if (dir !== 'assets/models') return [];
        return [
          { name: 'ship_a.glb', kind: 'file' },
          { name: 'ship_a.model.toml', kind: 'file' },
          { name: 'ship_b.glb', kind: 'file' },
        ];
      },
    },
  };
}

async function flush() {
  // Let the mount's async bootstrap IIFE settle.
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
}

describe('mountModelsMode (jsdom)', () => {
  let host;
  let modeShell;

  beforeEach(() => {
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    modeShell = new ModeShell();
  });

  it('renders the three-pane shell and discovers models', async () => {
    const { io } = makeIo();
    mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => makeSceneStub() },
    });
    await flush();

    expect(host.querySelector('.models-three-pane')).toBeTruthy();
    const rows = [...host.querySelectorAll('.models-file-row')].map((r) => r.textContent);
    expect(rows).toEqual(['ship_a', 'ship_b']);
  });

  it('registers sidecar paths as open files for the guard', async () => {
    const { io } = makeIo();
    mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => makeSceneStub() },
    });
    await flush();

    const open = modeShell.getOpenFiles('Models');
    // ship_a has a real model.toml; ship_b has none -> default 'model'.
    expect(open).toContain('assets/models/ship_a.model.toml');
    expect(open).toContain('assets/models/ship_b.model.toml');
  });

  it('selecting a model loads its rig and creates marker visuals', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo({
      files: {
        'assets/models/ship_a.model.toml':
          '[base]\noffset=[0,0,0]\n[markers]\nfore = { position=[0,0,-3], direction=[0,0,-1] }\n',
      },
    });
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();

    await view._internal.selectModel('ship_a');
    expect(stub._calls.addMarker.map((c) => c[0])).toContain('fore');
    expect(view._internal.getRig().markers.fore.position).toEqual([0, 0, -3]);
  });

  it('Save writes a TOML sidecar and clears dirty', async () => {
    const stub = makeSceneStub();
    const { io, writes } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    view._internal.handleAddMarker('aft');
    const path = 'assets/models/ship_b.model.toml';
    expect(modeShell.isDirty('Models', path)).toBe(true);

    await view._internal.saveCurrent();
    expect(writes[path]).toMatch(/\[markers\.aft\]/);
    // Cached extents from the stub scene are written.
    expect(writes[path]).toMatch(/size = \[ 4, 2, 6 \]/);
    expect(modeShell.isDirty('Models', path)).toBe(false);
  });

  it('Save as new variant writes <stem>.<name>.toml and switches to it', async () => {
    const stub = makeSceneStub();
    const { io, writes } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    await view._internal.saveAsVariant('weathered');
    const path = 'assets/models/ship_b.weathered.toml';
    expect(writes[path]).toBeTruthy();
    expect(parseSidecarName('ship_b.weathered.toml')).toEqual({ stem: 'ship_b', variant: 'weathered' });
    const model = view._internal.getModels().find((m) => m.stem === 'ship_b');
    expect(model.variants).toContain('weathered');
  });

  it('Save All over a dirty Models file does not throw and writes it', async () => {
    const stub = makeSceneStub();
    const { io, writes } = makeIo();
    // Real SaveFlow with the 'models' passthrough stringifier (matches app-v2).
    const saveFlow = new SaveFlow(
      modeShell,
      { models: (s) => s },
      io.writeFile,
    );
    const view = mountModelsMode({
      host, modeShell, saveFlow, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');
    view._internal.handleAddMarker('aft');

    const path = 'assets/models/ship_b.model.toml';
    expect(modeShell.isDirty('Models', path)).toBe(true);

    // Should not throw, and should write the cached TOML verbatim.
    const results = await saveFlow.saveAll();
    const result = results.find((r) => r.path === path);
    expect(result.ok).toBe(true);
    expect(writes[path]).toMatch(/aft/);
    expect(modeShell.isDirty('Models', path)).toBe(false);
  });

  it('gizmo move syncs back into the rig', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');
    view._internal.handleAddMarker('fore');

    stub._fireChange('fore', { position: [1, 2, 3], direction: [0, 0, -2] });
    const rig = view._internal.getRig();
    expect(rig.markers.fore.position).toEqual([1, 2, 3]);
    expect(rig.markers.fore.direction).toEqual([0, 0, -1]);
  });

  it('clears previous markers when switching variants (no ghosts)', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo({
      files: {
        'assets/models/ship_a.model.toml':
          '[markers]\nfore = { position=[0,0,-3], direction=[0,0,-1] }\n',
        'assets/models/ship_a.weathered.toml':
          '[markers]\naft = { position=[0,0,3], direction=[0,0,1] }\n',
      },
    });
    // Register the weathered variant in discovery.
    io.listDirectory = async (dir) => (dir !== 'assets/models' ? [] : [
      { name: 'ship_a.glb', kind: 'file' },
      { name: 'ship_a.model.toml', kind: 'file' },
      { name: 'ship_a.weathered.toml', kind: 'file' },
    ]);
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();

    await view._internal.selectModel('ship_a'); // model variant -> 'fore'
    stub._calls.clearMarkers = 0;
    await view._internal.selectVariant('weathered'); // -> 'aft'

    // clearMarkers must run before any addMarker for the new variant.
    expect(stub._calls.clearMarkers).toBeGreaterThan(0);
    const firstAdd = stub._calls.order.indexOf('addMarker:aft');
    const lastClear = stub._calls.order.lastIndexOf('clearMarkers');
    expect(lastClear).toBeGreaterThanOrEqual(0);
    expect(firstAdd).toBeGreaterThan(lastClear);
  });

  it('frames the camera AFTER the base transform is applied', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    const setBaseIdx = stub._calls.order.indexOf('setBase');
    const frameIdx = stub._calls.order.indexOf('frame');
    expect(setBaseIdx).toBeGreaterThanOrEqual(0);
    expect(frameIdx).toBeGreaterThan(setBaseIdx);
  });

  it('saveAsVariant rejects the reserved name "model"', async () => {
    const stub = makeSceneStub();
    const { io, writes } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    const prevAlert = globalThis.alert;
    globalThis.alert = () => {};
    await view._internal.saveAsVariant('model');
    if (prevAlert) globalThis.alert = prevAlert; else delete globalThis.alert;
    expect(writes['assets/models/ship_b.model.toml']).toBeUndefined();
  });

  it('saveAsVariant confirms before overwriting an existing variant', async () => {
    const stub = makeSceneStub();
    const { io, writes } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    // First save creates 'weathered'.
    await view._internal.saveAsVariant('weathered');
    const path = 'assets/models/ship_b.weathered.toml';
    expect(writes[path]).toBeTruthy();

    // Decline overwrite.
    globalThis.confirm = () => false;
    writes[path] = undefined;
    await view._internal.saveAsVariant('weathered');
    expect(writes[path]).toBeUndefined();

    // Accept overwrite.
    globalThis.confirm = () => true;
    await view._internal.saveAsVariant('weathered');
    expect(writes[path]).toBeTruthy();
    delete globalThis.confirm;
  });

  it('disposeScene disposes the scene and disconnects observers', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b'); // creates the scene

    expect(view._internal.getScene()).toBe(stub);
    view._internal.disposeScene();
    expect(stub._calls.dispose).toBe(1);
    expect(view._internal.getScene()).toBeNull();
  });

  it('lights the shared unsaved indicator on a Models edit and clears on save', async () => {
    const indicator = document.createElement('span');
    indicator.id = 'unsavedIndicator';
    document.body.appendChild(indicator);

    const stub = makeSceneStub();
    const { io } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    view._internal.handleAddMarker('aft');
    expect(indicator.textContent).toContain('Unsaved changes');

    await view._internal.saveCurrent();
    expect(indicator.textContent).toBe('');
  });
  // ── Model-marker contract (issue #758) ──────────────────────────────
  it('refuses to create a marker whose name cannot round-trip as a rig key', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    view._internal.handleAddMarker('fore emitter');
    // No attachment is created: not in the rig, not in the 3D view, not dirty.
    expect(view._internal.getRig().markers['fore emitter']).toBeUndefined();
    expect(stub._calls.addMarker.map((c) => c[0])).not.toContain('fore emitter');
    expect(modeShell.isDirty('Models', 'assets/models/ship_b.model.toml')).toBe(false);
    // The failure is presented, not swallowed.
    expect(host.textContent).toContain('is not a valid rig key');
  });

  it('refuses a rename to an invalid marker name and keeps the original', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    view._internal.handleAddMarker('aft');
    view._internal.handleRenameMarker('aft', 'aft.port');
    expect(view._internal.getRig().markers.aft).toBeDefined();
    expect(view._internal.getRig().markers['aft.port']).toBeUndefined();
    expect(host.textContent).toContain('is not a valid rig key');
  });

  // ── Blocked / failed writes must not be reported as success (issue #758) ──

  it('a gate-blocked "Save as new variant" does not register the variant', async () => {
    const stub = makeSceneStub();
    const { io, writes } = makeIo();
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');

    // Force a rig the sidecar gate refuses (a name the CRUD path would have
    // rejected up front, e.g. from a hand-edited file loaded into the view).
    view._internal.getRig().markers['engine port'] = {
      position: [0, 0, 0], direction: [0, 0, -1],
    };

    await view._internal.saveAsVariant('weathered');

    expect(writes['assets/models/ship_b.weathered.toml']).toBeUndefined();
    const model = view._internal.getModels().find((m) => m.stem === 'ship_b');
    expect(model.variants).not.toContain('weathered');
    expect(modeShell.getActiveFile('Models')).not.toBe('assets/models/ship_b.weathered.toml');
    expect(host.textContent).toContain('is not a valid rig key');
  });

  it('a failed write leaves the file dirty and the variant unregistered', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    io.writeFile = async () => { throw new Error('disk full'); };
    const view = mountModelsMode({
      host, modeShell, io,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');
    view._internal.handleAddMarker('aft');
    expect(modeShell.isDirty('Models', 'assets/models/ship_b.model.toml')).toBe(true);

    expect(await view._internal.saveCurrent()).toBe(false);
    expect(modeShell.isDirty('Models', 'assets/models/ship_b.model.toml')).toBe(true);

    await view._internal.saveAsVariant('weathered');
    const model = view._internal.getModels().find((m) => m.stem === 'ship_b');
    expect(model.variants).not.toContain('weathered');
  });

  it('a successful rig write re-seeds the cross-file rig index via the bus', async () => {
    const stub = makeSceneStub();
    const { io } = makeIo();
    const bus = new InvalidationBus();
    const rigIndex = new RigIndex();
    wireRigIndexToSaves(rigIndex, bus);
    const view = mountModelsMode({
      host, modeShell, io, invalidationBus: bus,
      deps: { createRigScene: () => stub },
    });
    await flush();
    await view._internal.selectModel('ship_b');
    view._internal.handleAddMarker('torpedo_dorsal');

    expect(await view._internal.saveCurrent()).toBe(true);
    // The very next entity save — no reload — can already see the marker.
    const rig = rigIndex.get('assets/models/ship_b.model.toml');
    expect(Object.keys(rig.markers)).toContain('torpedo_dorsal');
  });
});
