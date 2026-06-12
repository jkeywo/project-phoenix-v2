/**
 * Slice-6 helpers — DOM shim + Definitions Mode mount with real fixtures.
 */
import { installDom, FakeElement, fixture } from './slice-5-helpers.js';

const FACTION_FILES = ['federation.toml', 'pirate.toml', 'harrow.toml', 'requiem.toml'];
const COMPLEXITY_FILES = ['tactical.toml', 'power.toml', 'sensors.toml', 'shields.toml', 'navigation.toml'];

export async function setupDefinitionsMode(opts = {}) {
  installDom();
  globalThis.window = globalThis.window || {};
  const host = new FakeElement('div');

  const { mountDefinitionsMode } = await import('../definitions-mode-view.js');
  const { ModeShell } = await import('../mode-shell.js');
  const { SaveFlow } = await import('../save-flow.js');
  const { InvalidationBus } = await import('../invalidation-bus.js');
  const { stringifyFactionToml } = await import('../faction-editor.js');
  const { stringifyComplexityToml } = await import('../complexity-editor.js');

  function stringifyDefinitionsPayload(payload) {
    if (!payload || typeof payload !== 'object') throw new Error('bad payload');
    if (payload.kind === 'faction') return stringifyFactionToml(payload.data);
    if (payload.kind === 'complexity') return stringifyComplexityToml(payload.data);
    throw new Error(`unknown kind: ${payload.kind}`);
  }

  const writeFileCalls = [];
  const writeFileFn = async (path, content) => {
    writeFileCalls.push({ path, content });
  };

  const modeShell = new ModeShell();
  modeShell.switchMode('Definitions');
  const invalidationBus = new InvalidationBus();
  const saveFlow = new SaveFlow(
    modeShell,
    {
      world: () => '',
      entity: () => '',
      definitions: stringifyDefinitionsPayload,
    },
    writeFileFn,
    invalidationBus,
  );

  let restoreCb = null;
  const registerRestore = (mode, fn) => {
    if (mode === 'Definitions') restoreCb = fn;
  };

  const factionFiles = opts.factionFiles ?? FACTION_FILES;
  const complexityFiles = opts.complexityFiles ?? COMPLEXITY_FILES;

  const io = {
    readFile: async (path) => fixture(path),
    listDirectory: async (rel) => {
      if (rel === 'assets/factions') {
        return factionFiles.map((name) => ({ name, kind: 'file' }));
      }
      if (rel === 'assets/complexity') {
        return complexityFiles.map((name) => ({ name, kind: 'file' }));
      }
      return [];
    },
    getProjectRoot: async () => ({ stub: true }),
  };

  const view = mountDefinitionsMode({
    host,
    modeShell,
    saveFlow,
    registerRestore,
    invalidationBus,
    io,
  });

  // Wait for the fire-and-forget bootstrap to finish.
  await view._internal.bootstrap();

  return {
    view,
    host,
    modeShell,
    saveFlow,
    invalidationBus,
    writeFileCalls,
    getRestoreCb: () => restoreCb,
  };
}
