import { describe, it, expect, vi } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { SaveFlow } from '../save-flow.js';
import { InvalidationBus } from '../invalidation-bus.js';

const noopWriter = async () => {};
const noopBus = { fireEntitySaved: () => {}, fireWorldSaved: () => {} };

describe('SaveFlow', () => {
  describe('saveActive', () => {
    it('returns ok when file is serializable', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const stringifyFns = {
        world: (obj) => `serialized: ${obj.name}`,
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {
        name: 'Test World',
        global: {},
        anchors: {},
      });

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(true);
      expect(result.errors).toEqual([]);
    });

    it('returns error when file cannot be serialized', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const stringifyFns = {
        world: () => {
          throw new Error('TOML serialization failed');
        },
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {
        name: 'Test World',
      });

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(false);
      expect(result.errors).toHaveLength(1);
      expect(result.errors[0]).toContain('failed');
    });

    it('returns warnings for validation errors but does not block', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/invalid.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/invalid.toml');
      modeShell.markDirty('World', 'assets/worlds/invalid.toml', true);

      const stringifyFns = {
        world: (obj) => 'serialized content',
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/invalid.toml', {
        name: 'Bad World',
      });

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(true);
      expect(result.errors).toEqual([]);
      expect(result.warnings.length).toBeGreaterThan(0);
    });

    it('marks file as clean after successful save', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const stringifyFns = {
        world: (obj) => 'serialized',
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {
        global: {},
        anchors: {},
      });

      expect(modeShell.isDirty('World', 'assets/worlds/test.toml')).toBe(true);
      await saveFlow.saveActive();
      expect(modeShell.isDirty('World', 'assets/worlds/test.toml')).toBe(false);
    });

    it('with no active file returns error', async () => {
      const modeShell = new ModeShell();
      const stringifyFns = { world: () => '', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(false);
      expect(result.errors).toHaveLength(1);
      expect(result.errors[0]).toContain('No active file');
    });
  });

  describe('saveAll', () => {
    it('saves all dirty files', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/a.toml']);
      modeShell.setOpenFiles('Entity', ['assets/entities/b.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/a.toml');
      modeShell.setActiveFile('Entity', 'assets/entities/b.toml');
      modeShell.markDirty('World', 'assets/worlds/a.toml', true);
      modeShell.markDirty('Entity', 'assets/entities/b.toml', true);

      const stringifyFns = {
        world: () => 'world content',
        entity: () => 'entity content',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/a.toml', {
        global: {},
        anchors: {},
      });
      saveFlow.setContent('Entity', 'assets/entities/b.toml', { tags: ['test'] });

      const results = await saveFlow.saveAll();
      expect(results).toHaveLength(2);
      expect(results.every((r) => r.ok)).toBe(true);
    });

    it('returns per-file results', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/a.toml']);
      modeShell.setOpenFiles('Entity', ['assets/entities/b.toml']);
      modeShell.markDirty('World', 'assets/worlds/a.toml', true);
      modeShell.markDirty('Entity', 'assets/entities/b.toml', true);

      const stringifyFns = {
        world: () => {
          throw new Error('World serialization error');
        },
        entity: () => 'entity content',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/a.toml', { name: 'test' });
      saveFlow.setContent('Entity', 'assets/entities/b.toml', { tags: ['test'] });

      const results = await saveFlow.saveAll();
      expect(results).toHaveLength(2);
      expect(results[0].path).toBe('assets/worlds/a.toml');
      expect(results[0].ok).toBe(false);
      expect(results[1].path).toBe('assets/entities/b.toml');
      expect(results[1].ok).toBe(true);
    });

    it('marks each file clean on success', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/a.toml']);
      modeShell.setOpenFiles('Entity', ['assets/entities/b.toml']);
      modeShell.markDirty('World', 'assets/worlds/a.toml', true);
      modeShell.markDirty('Entity', 'assets/entities/b.toml', true);

      const stringifyFns = {
        world: () => 'content',
        entity: () => 'content',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/a.toml', {
        global: {},
        anchors: {},
      });
      saveFlow.setContent('Entity', 'assets/entities/b.toml', { tags: ['test'] });

      await saveFlow.saveAll();
      expect(modeShell.isDirty('World', 'assets/worlds/a.toml')).toBe(false);
      expect(modeShell.isDirty('Entity', 'assets/entities/b.toml')).toBe(false);
    });
  });

  describe('getDirtyFiles', () => {
    it('returns list of dirty files', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['a.toml', 'b.toml']);
      modeShell.setOpenFiles('Entity', ['c.toml']);
      modeShell.markDirty('World', 'a.toml', true);
      modeShell.markDirty('Entity', 'c.toml', true);

      const saveFlow = new SaveFlow(modeShell, {
        world: () => '',
        entity: () => '',
      }, noopWriter, noopBus);

      const dirty = saveFlow.getDirtyFiles();
      expect(dirty).toHaveLength(2);
      expect(dirty).toContainEqual({ mode: 'World', path: 'a.toml' });
      expect(dirty).toContainEqual({ mode: 'Entity', path: 'c.toml' });
    });
  });

  describe('cross-reference warnings', () => {
    it('do not block save (never-refuse-valid-TOML)', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const stringifyFns = { world: () => 'content', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {});

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(true);
      expect(result.warnings).toBeDefined();
    });
  });

  describe('writeFile integration', () => {
    it('calls writeFile with (path, stringified content)', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const writeFile = vi.fn(async () => {});
      const stringifyFns = {
        world: () => 'WORLD_CONTENT',
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, writeFile, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {});

      await saveFlow.saveActive();
      expect(writeFile).toHaveBeenCalledTimes(1);
      expect(writeFile).toHaveBeenCalledWith('assets/worlds/test.toml', 'WORLD_CONTENT');
    });

    it('returns ok: false when writeFile rejects', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const writeFile = async () => { throw new Error('disk full'); };
      const stringifyFns = { world: () => 'X', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, writeFile, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {});

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(false);
      expect(result.errors[0]).toMatch(/disk full/);
      // Still dirty — write failed.
      expect(modeShell.isDirty('World', 'assets/worlds/test.toml')).toBe(true);
    });

    it('does not write or mark clean if serialization throws', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const writeFile = vi.fn(async () => {});
      const stringifyFns = {
        world: () => { throw new Error('bad'); },
        entity: () => '',
      };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, writeFile, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {});

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(false);
      expect(writeFile).not.toHaveBeenCalled();
      expect(modeShell.isDirty('World', 'assets/worlds/test.toml')).toBe(true);
    });
  });

  describe('undo + invalidation on save', () => {
    it('clears undo history on successful save', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);
      modeShell.pushUndoEntry('World', 'assets/worlds/test.toml', { v: 1 });
      modeShell.pushUndoEntry('World', 'assets/worlds/test.toml', { v: 2 });
      expect(modeShell.getUndoHistory('World', 'assets/worlds/test.toml')).toHaveLength(2);

      const stringifyFns = { world: () => 'X', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {});

      await saveFlow.saveActive();
      expect(modeShell.getUndoHistory('World', 'assets/worlds/test.toml')).toEqual([]);
    });

    it('fires fireWorldSaved on World mode save', async () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('World', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('World', 'assets/worlds/test.toml');
      modeShell.markDirty('World', 'assets/worlds/test.toml', true);

      const bus = new InvalidationBus();
      const fired = [];
      bus.onWorldSaved((p) => fired.push(p));

      const stringifyFns = { world: () => 'X', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, bus);
      saveFlow.setContent('World', 'assets/worlds/test.toml', {});

      await saveFlow.saveActive();
      expect(fired).toEqual(['assets/worlds/test.toml']);
    });

    it('fires fireEntitySaved on Entity mode save', async () => {
      const modeShell = new ModeShell();
      modeShell.switchMode('Entity');
      modeShell.setOpenFiles('Entity', ['assets/entities/a.toml']);
      modeShell.setActiveFile('Entity', 'assets/entities/a.toml');
      modeShell.markDirty('Entity', 'assets/entities/a.toml', true);

      const bus = new InvalidationBus();
      const fired = [];
      bus.onEntitySaved((p) => fired.push(p));

      const stringifyFns = { world: () => '', entity: () => 'E' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, bus);
      saveFlow.setContent('Entity', 'assets/entities/a.toml', {});

      await saveFlow.saveActive();
      expect(fired).toEqual(['assets/entities/a.toml']);
    });
  });

  describe('Definitions mode', () => {
    it('dispatches to the `definitions` stringifier when provided', async () => {
      const modeShell = new ModeShell();
      modeShell.switchMode('Definitions');
      modeShell.setOpenFiles('Definitions', ['assets/factions/federation.toml']);
      modeShell.setActiveFile('Definitions', 'assets/factions/federation.toml');
      modeShell.markDirty('Definitions', 'assets/factions/federation.toml', true);

      const writeFile = vi.fn(async () => {});
      const defStringify = vi.fn(() => 'DEFINITIONS_CONTENT');
      const stringifyFns = {
        world: () => 'W',
        entity: () => 'E',
        definitions: defStringify,
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns, writeFile, noopBus);
      saveFlow.setContent('Definitions', 'assets/factions/federation.toml', {
        kind: 'faction',
        data: { uuid: 'x', name: 'X', enemies: [] },
      });

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(true);
      expect(defStringify).toHaveBeenCalledTimes(1);
      expect(defStringify.mock.calls[0][0]).toEqual({
        kind: 'faction',
        data: { uuid: 'x', name: 'X', enemies: [] },
      });
      expect(writeFile).toHaveBeenCalledWith('assets/factions/federation.toml', 'DEFINITIONS_CONTENT');
    });

    it('fires fireFactionSaved when saving a faction path in Definitions mode', async () => {
      const modeShell = new ModeShell();
      modeShell.switchMode('Definitions');
      modeShell.setOpenFiles('Definitions', ['assets/factions/pirate.toml']);
      modeShell.setActiveFile('Definitions', 'assets/factions/pirate.toml');
      modeShell.markDirty('Definitions', 'assets/factions/pirate.toml', true);

      const bus = new InvalidationBus();
      const fired = [];
      bus.onFactionSaved((p) => fired.push(p));

      const stringifyFns = {
        world: () => '',
        entity: () => '',
        definitions: () => 'X',
      };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, bus);
      saveFlow.setContent('Definitions', 'assets/factions/pirate.toml', {
        kind: 'faction',
        data: {},
      });

      await saveFlow.saveActive();
      expect(fired).toEqual(['assets/factions/pirate.toml']);
    });

    it('does NOT fire fireFactionSaved for complexity paths in Definitions mode', async () => {
      const modeShell = new ModeShell();
      modeShell.switchMode('Definitions');
      modeShell.setOpenFiles('Definitions', ['assets/complexity/tactical.toml']);
      modeShell.setActiveFile('Definitions', 'assets/complexity/tactical.toml');
      modeShell.markDirty('Definitions', 'assets/complexity/tactical.toml', true);

      const bus = new InvalidationBus();
      const fired = [];
      bus.onFactionSaved((p) => fired.push(p));

      const stringifyFns = {
        world: () => '',
        entity: () => '',
        definitions: () => 'X',
      };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, bus);
      saveFlow.setContent('Definitions', 'assets/complexity/tactical.toml', {
        kind: 'complexity',
        data: [],
      });

      await saveFlow.saveActive();
      expect(fired).toEqual([]);
    });

    it('backward-compat: falls back to `entity` stringifier when `definitions` is missing', async () => {
      const modeShell = new ModeShell();
      modeShell.switchMode('Definitions');
      modeShell.setOpenFiles('Definitions', ['assets/factions/federation.toml']);
      modeShell.setActiveFile('Definitions', 'assets/factions/federation.toml');
      modeShell.markDirty('Definitions', 'assets/factions/federation.toml', true);

      const entityStringify = vi.fn(() => 'FALLBACK');
      const stringifyFns = { world: () => '', entity: entityStringify };
      const saveFlow = new SaveFlow(modeShell, stringifyFns, noopWriter, noopBus);
      saveFlow.setContent('Definitions', 'assets/factions/federation.toml', { foo: 'bar' });

      const result = await saveFlow.saveActive();
      expect(result.ok).toBe(true);
      expect(entityStringify).toHaveBeenCalledTimes(1);
    });
  });
});
