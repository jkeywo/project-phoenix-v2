// @vitest-environment jsdom
/**
 * scenario-script-panel.test.js — integration of the Rhai script editor into
 * Scenario Mode (#983): the SCRIPTS list, opening a unit, and inline write-back
 * through the injected host-fn registry + diagnostics pass.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { extractScriptUnits } from '../script-editor.js';

const HOST_FNS = [
  { name: 'on_timer', receiver: '', category: 'trigger', signature: 'on_timer(after_secs, handler)', summary: 'x' },
  { name: 'complete_objective', receiver: 'effects', category: 'effect', signature: 'effects.complete_objective(id)', summary: 'x' },
];

class StubCanvasManager {
  constructor(lm) { this.layerManager = lm; this.selectedSpawn = null; }
  init() {}
  renderAll() {}
  buildV2Layers() { return []; }
  selectByEntityName() {}
  deselectSpawn() {}
}
class StubPropertiesPanel { render() {} updatePositionFields() {} }
class StubEntityEditor { init() {} loadEntitiesPalette() {} openModal() {} }

class FakeModeShell {
  constructor() { this._m = 'World'; }
  setActiveFile() {} setActiveLayer() {} setOpenFiles() {}
  getCurrentMode() { return this._m; }
  hasAnyDirty() { return false; }
}
class FakeSaveFlow {
  setSessionOnlyChecker() {} setContent() {}
  async saveAll() { return []; } async saveActive() { return { ok: true }; }
}

function el(tag, id) {
  const e = document.createElement(tag);
  if (id) e.id = id;
  return e;
}

describe('Scenario Mode script panel', () => {
  let mod, host, fakeWasm, diagCalls;

  beforeEach(async () => {
    document.body.innerHTML = '';
    globalThis.window = window;
    window.tomlParse = () => ({});

    host = el('div', 'world-mode-root');
    host.appendChild(el('button', 'newEntityBtn'));
    document.body.appendChild(host);
    for (const id of [
      'scriptList', 'scriptEditorHost', 'scriptEditorPanel',
      'propertiesPanel', 'propertiesPanelContent', 'worldContentList',
      'triggerableWorldsList', 'unsavedIndicator', 'saveAllBtn',
      'saveLayerBtn', 'layersList', 'entitiesList', 'canvas',
    ]) {
      const tag = id.endsWith('Btn') ? 'button' : 'div';
      document.body.appendChild(el(tag, id));
    }
    document.getElementById('scriptEditorPanel').classList.add('hidden');

    diagCalls = [];
    fakeWasm = {
      getHostFns: async () => HOST_FNS,
      getDiagnostics: async (src, offset) => {
        diagCalls.push({ src, offset });
        return src.includes('@@') // deliberately invalid → a diagnostic
          ? [{ message: 'syntax error', line: 1 + offset, column: 1, severity: 'error' }]
          : [];
      },
    };

    mod = await import('../scenario-mode.js');
  });

  function mount() {
    return mod.mountScenarioMode({
      host,
      modeShell: new FakeModeShell(),
      saveFlow: new FakeSaveFlow(),
      registerRestore: () => {},
      invalidationBus: { onEntitySaved: () => {} },
      io: { readFile: async () => 'fn sibling(ctx) { }', writeFile: async () => {}, listDirectory: async () => [] },
      deps: {
        CanvasManager: StubCanvasManager,
        PropertiesPanel: StubPropertiesPanel,
        EntityEditor: StubEntityEditor,
        preloadEntityCache: async () => {},
        renderTriggerableWorldsPanel: async () => {},
        renderWorldContentPanel: () => {},
        mountNewWorldButton: () => null,
        mountOpenWorldButton: () => null,
        scriptWasm: fakeWasm,
      },
    });
  }

  it('lists a world\'s inline script blocks', async () => {
    const handle = mount();
    await handle.ready;
    handle.layerManager.addInMemoryLayer('assets/worlds/w.toml', {
      global: { seed: 1 },
      script: { setup: 'fn setup(ctx) { }' },
    });
    handle.renderScriptPanel();

    const rows = document.getElementById('scriptList').querySelectorAll('.script-list-row');
    expect(rows.length).toBe(1);
    expect(rows[0].textContent).toContain('[script.setup]');
  });

  it('opens an inline block in the text editor and edits write back to the layer', async () => {
    const handle = mount();
    await handle.ready;
    const layer = handle.layerManager.addInMemoryLayer('assets/worlds/w.toml', {
      global: { seed: 1 },
      script: { setup: 'on_timer(5, "h");\nfn h(ctx) { }' },
    });
    const [unit] = extractScriptUnits(layer.toml, layer.filename);

    await handle.openScript(unit);
    const ctrl = handle.getScriptController();
    expect(ctrl).toBeTruthy();
    // The script panel is shown, properties hidden.
    expect(document.getElementById('scriptEditorPanel').classList.contains('hidden')).toBe(false);
    expect(document.getElementById('propertiesPanel').classList.contains('hidden')).toBe(true);
    // Seeded with the inline source.
    expect(ctrl.getSource()).toContain('on_timer(5');

    // Edit → the layer's inline block updates and the layer goes dirty.
    const ta = document.querySelector('.script-editor-input');
    ta.value = 'on_timer(9, "h");\nfn h(ctx) { }';
    ta.dispatchEvent(new window.Event('input'));
    expect(layer.toml.script.setup).toBe('on_timer(9, "h");\nfn h(ctx) { }');
    expect(layer.isDirty).toBe(true);
  });

  it('routes diagnostics through the injected WASM pass', async () => {
    const handle = mount();
    await handle.ready;
    const layer = handle.layerManager.addInMemoryLayer('assets/worlds/w.toml', {
      global: { seed: 1 },
      script: { setup: 'fn s(ctx) { @@ }' },
    });
    const [unit] = extractScriptUnits(layer.toml, layer.filename);
    await handle.openScript(unit);
    await handle.getScriptController().runDiagnostics();

    expect(diagCalls.length).toBeGreaterThan(0);
    const diag = document.querySelector('.script-diagnostic');
    expect(diag).toBeTruthy();
    expect(diag.textContent).toContain('syntax error');
  });

  it('reads a sibling .rhai unit through io.readFile', async () => {
    const handle = mount();
    await handle.ready;
    const layer = handle.layerManager.addInMemoryLayer('assets/worlds/w.toml', {
      global: { seed: 1 },
      script: 'combat.rhai',
    });
    const [unit] = extractScriptUnits(layer.toml, layer.filename);
    expect(unit.kind).toBe('sibling');
    await handle.openScript(unit);
    expect(handle.getScriptController().getSource()).toBe('fn sibling(ctx) { }');
  });
});
