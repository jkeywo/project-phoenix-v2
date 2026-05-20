import { describe, it, expect } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { SaveFlow } from '../save-flow.js';

describe('SaveFlow', () => {
  describe('saveActive', () => {
    it('returns ok when file is serializable', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('Scenario', 'assets/worlds/test.toml');
      modeShell.markDirty('Scenario', 'assets/worlds/test.toml', true);

      const stringifyFns = {
        world: (obj) => `serialized: ${obj.name}`,
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/test.toml', {
        name: 'Test World',
        global: {},
        anchors: {},
      });

      const result = saveFlow.saveActive(null);
      expect(result.ok).toBe(true);
      expect(result.errors).toEqual([]);
    });

    it('returns error when file cannot be serialized', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('Scenario', 'assets/worlds/test.toml');
      modeShell.markDirty('Scenario', 'assets/worlds/test.toml', true);

      const stringifyFns = {
        world: () => {
          throw new Error('TOML serialization failed');
        },
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/test.toml', {
        name: 'Test World',
      });

      const result = saveFlow.saveActive(null);
      expect(result.ok).toBe(false);
      expect(result.errors).toHaveLength(1);
      expect(result.errors[0]).toContain('failed');
    });

    it('returns warnings for validation errors but does not block', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/invalid.toml']);
      modeShell.setActiveFile('Scenario', 'assets/worlds/invalid.toml');
      modeShell.markDirty('Scenario', 'assets/worlds/invalid.toml', true);

      const stringifyFns = {
        world: (obj) => 'serialized content',
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/invalid.toml', {
        name: 'Bad World',
      });

      const result = saveFlow.saveActive(null);
      expect(result.ok).toBe(true);
      expect(result.errors).toEqual([]);
      expect(result.warnings.length).toBeGreaterThan(0);
    });

    it('marks file as clean after successful save', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('Scenario', 'assets/worlds/test.toml');
      modeShell.markDirty('Scenario', 'assets/worlds/test.toml', true);

      const stringifyFns = {
        world: (obj) => 'serialized',
        entity: () => '',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/test.toml', {
        global: {},
        anchors: {},
      });

      expect(modeShell.isDirty('Scenario', 'assets/worlds/test.toml')).toBe(true);
      saveFlow.saveActive(null);
      expect(modeShell.isDirty('Scenario', 'assets/worlds/test.toml')).toBe(false);
    });

    it('with no active file returns error', () => {
      const modeShell = new ModeShell();
      const stringifyFns = { world: () => '', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns);

      const result = saveFlow.saveActive(null);
      expect(result.ok).toBe(false);
      expect(result.errors).toHaveLength(1);
      expect(result.errors[0]).toContain('No active file');
    });
  });

  describe('saveAll', () => {
    it('saves all dirty files', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/a.toml']);
      modeShell.setOpenFiles('Entity', ['assets/entities/b.toml']);
      modeShell.setActiveFile('Scenario', 'assets/worlds/a.toml');
      modeShell.setActiveFile('Entity', 'assets/entities/b.toml');
      modeShell.markDirty('Scenario', 'assets/worlds/a.toml', true);
      modeShell.markDirty('Entity', 'assets/entities/b.toml', true);

      const stringifyFns = {
        world: () => 'world content',
        entity: () => 'entity content',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/a.toml', {
        global: {},
        anchors: {},
      });
      saveFlow.setContent('Entity', 'assets/entities/b.toml', { tags: ['test'] });

      const results = saveFlow.saveAll(null);
      expect(results).toHaveLength(2);
      expect(results.every((r) => r.ok)).toBe(true);
    });

    it('returns per-file results', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/a.toml']);
      modeShell.setOpenFiles('Entity', ['assets/entities/b.toml']);
      modeShell.markDirty('Scenario', 'assets/worlds/a.toml', true);
      modeShell.markDirty('Entity', 'assets/entities/b.toml', true);

      const stringifyFns = {
        world: () => {
          throw new Error('World serialization error');
        },
        entity: () => 'entity content',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/a.toml', { name: 'test' });
      saveFlow.setContent('Entity', 'assets/entities/b.toml', { tags: ['test'] });

      const results = saveFlow.saveAll(null);
      expect(results).toHaveLength(2);
      expect(results[0].path).toBe('assets/worlds/a.toml');
      expect(results[0].ok).toBe(false);
      expect(results[1].path).toBe('assets/entities/b.toml');
      expect(results[1].ok).toBe(true);
    });

    it('marks each file clean on success', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/a.toml']);
      modeShell.setOpenFiles('Entity', ['assets/entities/b.toml']);
      modeShell.markDirty('Scenario', 'assets/worlds/a.toml', true);
      modeShell.markDirty('Entity', 'assets/entities/b.toml', true);

      const stringifyFns = {
        world: () => 'content',
        entity: () => 'content',
      };

      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/a.toml', {
        global: {},
        anchors: {},
      });
      saveFlow.setContent('Entity', 'assets/entities/b.toml', { tags: ['test'] });

      saveFlow.saveAll(null);
      expect(modeShell.isDirty('Scenario', 'assets/worlds/a.toml')).toBe(false);
      expect(modeShell.isDirty('Entity', 'assets/entities/b.toml')).toBe(false);
    });
  });

  describe('getDirtyFiles', () => {
    it('returns list of dirty files', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['a.toml', 'b.toml']);
      modeShell.setOpenFiles('Entity', ['c.toml']);
      modeShell.markDirty('Scenario', 'a.toml', true);
      modeShell.markDirty('Entity', 'c.toml', true);

      const saveFlow = new SaveFlow(modeShell, {
        world: () => '',
        entity: () => '',
      });

      const dirty = saveFlow.getDirtyFiles();
      expect(dirty).toHaveLength(2);
      expect(dirty).toContainEqual({ mode: 'Scenario', path: 'a.toml' });
      expect(dirty).toContainEqual({ mode: 'Entity', path: 'c.toml' });
    });
  });

  describe('cross-reference warnings', () => {
    it('do not block save (never-refuse-valid-TOML)', () => {
      const modeShell = new ModeShell();
      modeShell.setOpenFiles('Scenario', ['assets/worlds/test.toml']);
      modeShell.setActiveFile('Scenario', 'assets/worlds/test.toml');
      modeShell.markDirty('Scenario', 'assets/worlds/test.toml', true);

      const stringifyFns = { world: () => 'content', entity: () => '' };
      const saveFlow = new SaveFlow(modeShell, stringifyFns);
      saveFlow.setContent('Scenario', 'assets/worlds/test.toml', {});

      const result = saveFlow.saveActive(null);
      expect(result.ok).toBe(true);
      expect(result.warnings).toBeDefined();
    });
  });
});
