/**
 * Integration test for `mountScenarioMode` (the extracted V1 Scenario shell).
 *
 * Pins the behaviour that the legacy `editor/app.js` was responsible for
 * before the V1/V2 dual-shell was collapsed:
 *
 *   1. Registers a 'World' undo restore callback.
 *   2. Subscribes to the entity-saved invalidation bus.
 *   3. Installs a session-only checker on SaveFlow.
 *   4. Wires Save All / Save Layer buttons to SaveFlow.
 *   5. Does NOT install the legacy file-picker fallback that creates
 *      unsavable layers (the "Saving through the new SaveFlow will fail"
 *      path is gone — opens must go through the FSA project root).
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { installDom, FakeElement, fireClick } from './slice-5-helpers.js';

// Stand-in stubs for the heavy V1 classes. The scenario shell only
// constructs them and forwards a handful of lifecycle calls; we don't
// need Konva/DOM-heavy behaviour for the wire test.
class StubCanvasManager {
  constructor(layerManager) {
    this.layerManager = layerManager;
    this.selectedSpawn = null;
    this.initialised = false;
    this.renderCount = 0;
  }
  init() { this.initialised = true; }
  renderAll() { this.renderCount += 1; }
  buildV2Layers() { return []; }
  selectByEntityName() {}
  deselectSpawn() { this.selectedSpawn = null; }
}

class StubPropertiesPanel {
  constructor() { this.renderCount = 0; }
  render() { this.renderCount += 1; }
  updatePositionFields() {}
}

class StubEntityEditor {
  constructor() { this.initialised = false; this.paletteLoads = 0; this.opened = 0; }
  init() { this.initialised = true; }
  loadEntitiesPalette() { this.paletteLoads += 1; }
  openModal() { this.opened += 1; }
}

class FakeSaveFlow {
  constructor() {
    this.sessionChecker = null;
    this.contentCalls = [];
    this.saveAllCalls = 0;
    this.saveActiveCalls = 0;
    this._contentCache = { World: {} };
  }
  setSessionOnlyChecker(fn) { this.sessionChecker = fn; }
  setContent(mode, path, payload) {
    this.contentCalls.push({ mode, path, payload });
    if (!this._contentCache[mode]) this._contentCache[mode] = {};
    this._contentCache[mode][path] = payload;
  }
  async saveAll() { this.saveAllCalls += 1; return []; }
  async saveActive() { this.saveActiveCalls += 1; return { ok: true }; }
}

class FakeModeShell {
  constructor() {
    this.activeFile = {};
    this.activeLayer = {};
    this.openFiles = {};
    this._mode = 'World';
  }
  setActiveFile(mode, path) { this.activeFile[mode] = path; }
  setActiveLayer(mode, path) { this.activeLayer[mode] = path; }
  setOpenFiles(mode, paths) { this.openFiles[mode] = paths; }
  getCurrentMode() { return this._mode; }
  swapUndoActive() { return null; }
  swapRedoActive() { return null; }
}

class FakeInvalidationBus {
  constructor() { this.entitySavedListeners = []; }
  onEntitySaved(fn) { this.entitySavedListeners.push(fn); return () => {}; }
}

function installDocumentWithIds(ids) {
  installDom();
  const elements = {};
  for (const id of ids) {
    const el = new FakeElement('div');
    el.id = id;
    elements[id] = el;
  }
  document.getElementById = (id) => elements[id] || null;
  document.querySelectorAll = () => [];
  document.querySelector = () => null;
  document.addEventListener = () => {};
  document.body = new FakeElement('body');
  return elements;
}

describe('mountScenarioMode', () => {
  let modeShell, saveFlow, invalidationBus, restoreRegistrations, elements, mod, host;

  beforeEach(async () => {
    elements = installDocumentWithIds([
      'saveAllBtn', 'saveLayerBtn',
      'newEntityBtn', 'newWorldBtn',
      'unsavedIndicator', 'worldContentList',
      'layersList', 'entitiesList', 'canvas', 'canvasContainer',
      'propertiesPanel', 'propertiesPanelContent', 'newEntityModal',
    ]);
    globalThis.window = { tomlParse: () => ({}) };

    // Build a #world-mode-root host that contains the elements that live
    // inside it in real editor.html (just `newEntityBtn` for now). The
    // toolbar IDs (`saveAllBtn`, `saveLayerBtn`, `unsavedIndicator`) stay
    // at document scope.
    host = new FakeElement('div');
    host.id = 'world-mode-root';
    host.appendChild(elements.newEntityBtn);

    modeShell = new FakeModeShell();
    saveFlow = new FakeSaveFlow();
    invalidationBus = new FakeInvalidationBus();
    restoreRegistrations = [];

    mod = await import('../scenario-mode.js');
  });

  function mount(overrides = {}) {
    return mod.mountScenarioMode({
      host,
      modeShell,
      saveFlow,
      registerRestore: (mode, fn) => restoreRegistrations.push({ mode, fn }),
      invalidationBus,
      io: {
        readFile: async () => '',
        writeFile: async () => {},
        listDirectory: async () => [],
        tomlParse: (s) => ({}),
      },
      deps: {
        CanvasManager: StubCanvasManager,
        PropertiesPanel: StubPropertiesPanel,
        EntityEditor: StubEntityEditor,
        preloadEntityCache: async () => {},
        renderWorldContentPanel: () => {},
        mountNewWorldButton: () => null,
      },
      ...overrides,
    });
  }

  it('registers a World undo restore callback', async () => {
    const handle = mount();
    await handle.ready;
    const reg = restoreRegistrations.find((r) => r.mode === 'World');
    expect(reg).toBeDefined();
    expect(typeof reg.fn).toBe('function');
  });

  it('installs a session-only checker on the SaveFlow', async () => {
    const handle = mount();
    await handle.ready;
    expect(typeof saveFlow.sessionChecker).toBe('function');
    // World mode + unknown layer → false (not session-only).
    expect(saveFlow.sessionChecker('World', 'nope')).toBe(false);
    // Other modes always false.
    expect(saveFlow.sessionChecker('Entity', 'whatever')).toBe(false);
  });

  it('subscribes to invalidationBus.onEntitySaved', async () => {
    const handle = mount();
    await handle.ready;
    expect(invalidationBus.entitySavedListeners.length).toBe(1);
  });

  it('initialises the canvas manager, properties panel and entity editor', async () => {
    const handle = mount();
    await handle.ready;
    expect(handle.canvasManager.initialised).toBe(true);
    expect(handle.entityEditor.initialised).toBe(true);
    // palette loaded after preload
    expect(handle.entityEditor.paletteLoads).toBeGreaterThanOrEqual(1);
  });

  it('Save All button routes through saveFlow.saveAll', async () => {
    const handle = mount();
    await handle.ready;
    fireClick(elements.saveAllBtn);
    // saveAll is async — flush microtasks.
    await Promise.resolve(); await Promise.resolve();
    expect(saveFlow.saveAllCalls).toBe(1);
  });

  it('Save Layer button (World mode, no active layer) does not crash and does not throw', async () => {
    const handle = mount();
    await handle.ready;
    fireClick(elements.saveLayerBtn);
    await Promise.resolve(); await Promise.resolve();
    // No active layer means saveActive is not called.
    expect(saveFlow.saveActiveCalls).toBe(0);
  });

  it('New Entity button opens the entity editor modal', async () => {
    const handle = mount();
    await handle.ready;
    fireClick(elements.newEntityBtn);
    expect(handle.entityEditor.opened).toBe(1);
  });

  it('does NOT create a hidden file input fallback for opening world TOMLs', async () => {
    const handle = mount();
    await handle.ready;
    // The legacy fallback appended a <input type="file"> to document.body.
    // The new code path goes through FSA project root only.
    const fileInputs = (document.body.children || []).filter(
      (c) => c.tagName === 'INPUT' && c.type === 'file',
    );
    expect(fileInputs.length).toBe(0);
  });
});
