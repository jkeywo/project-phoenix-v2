import { describe, it, expect } from 'vitest';
import { installDom, FakeElement, fixture, KonvaStub } from './slice-5-helpers.js';

/**
 * Slice 6 cross-mode coupling regression:
 *
 * When Definitions Mode saves a faction file, Entity Mode re-runs faction
 * discovery so its component-card faction dropdown reflects the rename.
 */
describe('Slice 6: faction-save invalidation refreshes Entity Mode', () => {
  it('Definitions save → invalidationBus.fireFactionSaved → Entity Mode reruns discover + setFactionMap', async () => {
    installDom();
    globalThis.window = { Konva: KonvaStub };

    const { mountEntityMode } = await import('../entity-mode-view.js');
    const { ModeShell } = await import('../mode-shell.js');
    const { SaveFlow } = await import('../save-flow.js');
    const { InvalidationBus } = await import('../invalidation-bus.js');
    const { stringifyEntityToml } = await import('../entity-toml.js');
    const { stringifyFactionToml } = await import('../faction-editor.js');
    const { stringifyComplexityToml } = await import('../complexity-editor.js');

    const modeShell = new ModeShell();
    const invalidationBus = new InvalidationBus();

    function stringifyDefinitionsPayload(p) {
      if (p.kind === 'faction') return stringifyFactionToml(p.data);
      if (p.kind === 'complexity') return stringifyComplexityToml(p.data);
      throw new Error('bad kind');
    }

    const saveFlow = new SaveFlow(
      modeShell,
      {
        world: () => '',
        entity: stringifyEntityToml,
        definitions: stringifyDefinitionsPayload,
      },
      async () => {},
      invalidationBus,
    );

    // Entity Mode discover() returns a controllable factionMap so we can
    // assert it gets re-called after a faction save.
    let discoverCallCount = 0;
    const factionMapV1 = new Map([['uuid-1', 'Federation']]);
    const factionMapV2 = new Map([['uuid-1', 'United Federation']]);
    const io = {
      readFile: async (path) => fixture(path),
      listDirectory: async (rel) => {
        if (rel === 'assets/entities') {
          return [{ name: 'pirate_raider.toml', kind: 'file' }];
        }
        return [];
      },
      preload: async () => {},
      onCacheInvalidate: () => {},
      getProjectRoot: async () => ({ stub: true }),
      discover: async () => {
        discoverCallCount += 1;
        return {
          factionMap: discoverCallCount === 1 ? factionMapV1 : factionMapV2,
          complexityPaths: [],
        };
      },
      Konva: KonvaStub,
    };

    const host = new FakeElement('div');
    const view = mountEntityMode({
      host,
      modeShell,
      saveFlow,
      registerRestore: () => {},
      invalidationBus,
      io,
    });
    for (let i = 0; i < 10; i += 1) await Promise.resolve();
    await view._internal.refreshFileList();

    expect(discoverCallCount).toBe(1);
    // Sanity: initial setFactionMap landed.
    const initial = view.shell.getFactionDropdownOptions();
    expect(initial.some((o) => o.name === 'Federation')).toBe(true);

    // Stage a fake faction save through SaveFlow → fires fireFactionSaved
    // → Entity Mode's subscriber re-runs discover().
    modeShell.switchMode('Definitions');
    modeShell.setOpenFiles('Definitions', ['assets/factions/federation.toml']);
    modeShell.setActiveFile('Definitions', 'assets/factions/federation.toml');
    modeShell.markDirty('Definitions', 'assets/factions/federation.toml', true);
    saveFlow.setContent('Definitions', 'assets/factions/federation.toml', {
      kind: 'faction',
      data: { uuid: 'uuid-1', name: 'United Federation', enemies: [] },
    });

    await saveFlow.saveActive();

    // Subscriber's discover() is async; flush microtasks.
    for (let i = 0; i < 10; i += 1) await Promise.resolve();

    expect(discoverCallCount).toBe(2);
    const after = view.shell.getFactionDropdownOptions();
    expect(after.some((o) => o.name === 'United Federation')).toBe(true);
    expect(after.some((o) => o.name === 'Federation')).toBe(false);
  });
});
