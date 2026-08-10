import { describe, it, expect } from 'vitest';
import { ModeShell } from '../mode-shell.js';

describe('mode-shell', () => {
  describe('constructor', () => {
    it('starts with World as default mode', () => {
      const shell = new ModeShell();
      expect(shell.getCurrentMode()).toBe('World');
    });

    it('accepts custom modes', () => {
      const shell = new ModeShell(['Red', 'Green', 'Blue']);
      expect(shell.getCurrentMode()).toBe('Red');
    });

    it('starts with empty open files for all modes', () => {
      const shell = new ModeShell();
      expect(shell.getOpenFiles('World')).toEqual([]);
      expect(shell.getOpenFiles('Entity')).toEqual([]);
      expect(shell.getOpenFiles('Definitions')).toEqual([]);
    });
  });

  describe('switchMode', () => {
    it('switches to a valid mode', () => {
      const shell = new ModeShell();
      expect(shell.getCurrentMode()).toBe('World');

      const result = shell.switchMode('Entity');
      expect(result).toBe(true);
      expect(shell.getCurrentMode()).toBe('Entity');
    });

    it('refuses to switch to an invalid mode', () => {
      const shell = new ModeShell();
      const result = shell.switchMode('InvalidMode');
      expect(result).toBe(false);
      expect(shell.getCurrentMode()).toBe('World');
    });

    it('switching to the current mode is a no-op but returns true', () => {
      const shell = new ModeShell();
      const result = shell.switchMode('World');
      expect(result).toBe(true);
      expect(shell.getCurrentMode()).toBe('World');
    });
  });

  describe('getModes', () => {
    it('returns the list of registered modes', () => {
      const shell = new ModeShell();
      expect(shell.getModes()).toEqual(['World', 'Entity', 'Definitions', 'Models', 'MOD']);
    });
  });

  describe('MOD mode (issue #989)', () => {
    it('MOD is a registered, switchable mode', () => {
      const shell = new ModeShell();
      expect(shell.getModes()).toContain('MOD');
      expect(shell.switchMode('MOD')).toBe(true);
      expect(shell.getCurrentMode()).toBe('MOD');
    });

    it('a dirty MOD-mode workspace participates in hasAnyDirty()', () => {
      const shell = new ModeShell();
      expect(shell.hasAnyDirty()).toBe(false);
      shell.markDirty('MOD', 'mod-pack', true);
      expect(shell.hasAnyDirty()).toBe(true);
      shell.markDirty('MOD', 'mod-pack', false);
      expect(shell.hasAnyDirty()).toBe(false);
    });
  });

  describe('open files per mode', () => {
    it('getOpenFiles returns empty array for a mode with no files', () => {
      const shell = new ModeShell();
      expect(shell.getOpenFiles('Entity')).toEqual([]);
    });

    it('setOpenFiles stores files for the current mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['worlds/default.toml', 'worlds/patrol.toml']);
      expect(shell.getOpenFiles('World')).toEqual(['worlds/default.toml', 'worlds/patrol.toml']);
    });

    it('setOpenFiles stores files for a non-current mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('Definitions', ['defs.toml']);
      expect(shell.getOpenFiles('Definitions')).toEqual(['defs.toml']);
    });

    it('mode switching preserves open files per mode', () => {
      const shell = new ModeShell();

      shell.setOpenFiles('World', ['scenario.toml']);
      shell.switchMode('Entity');
      shell.setOpenFiles('Entity', ['entity.toml']);

      expect(shell.getOpenFiles('World')).toEqual(['scenario.toml']);
      expect(shell.getOpenFiles('Entity')).toEqual(['entity.toml']);

      shell.switchMode('World');
      expect(shell.getOpenFiles('World')).toEqual(['scenario.toml']);
      expect(shell.getCurrentMode()).toBe('World');
    });

    it('setOpenFiles with invalid mode is silently ignored', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('NonExistent', ['file.toml']);
      expect(shell.getOpenFiles('NonExistent')).toBeUndefined();
    });
  });

  describe('dirty state tracking', () => {
    it('starts with no dirty files', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      expect(shell.isDirty('World', 'a.toml')).toBe(false);
    });

    it('markDirty sets a file as dirty', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      shell.markDirty('World', 'a.toml', true);
      expect(shell.isDirty('World', 'a.toml')).toBe(true);
    });

    it('markDirty(false) clears the dirty flag', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      shell.markDirty('World', 'a.toml', true);
      shell.markDirty('World', 'a.toml', false);
      expect(shell.isDirty('World', 'a.toml')).toBe(false);
    });

    it('hasAnyDirty returns true when any file in any mode is dirty', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      shell.setOpenFiles('Entity', ['b.toml']);
      expect(shell.hasAnyDirty()).toBe(false);
      shell.markDirty('Entity', 'b.toml', true);
      expect(shell.hasAnyDirty()).toBe(true);
    });

    it('dirty state persists across mode switches', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      shell.markDirty('World', 'a.toml', true);

      shell.switchMode('Entity');
      expect(shell.isDirty('World', 'a.toml')).toBe(true);

      shell.switchMode('World');
      expect(shell.isDirty('World', 'a.toml')).toBe(true);
    });
  });

  describe('active file per mode', () => {
    it('starts with no active file', () => {
      const shell = new ModeShell();
      expect(shell.getActiveFile('World')).toBeNull();
    });

    it('setActiveFile stores the active file for a mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml', 'b.toml']);
      shell.setActiveFile('World', 'a.toml');
      expect(shell.getActiveFile('World')).toBe('a.toml');
    });

    it('active file is independent per mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      shell.setOpenFiles('Entity', ['b.toml']);
      shell.setActiveFile('World', 'a.toml');
      shell.setActiveFile('Entity', 'b.toml');
      expect(shell.getActiveFile('World')).toBe('a.toml');
      expect(shell.getActiveFile('Entity')).toBe('b.toml');
    });

    it('active file persists when switching away and back', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['a.toml']);
      shell.setActiveFile('World', 'a.toml');

      shell.switchMode('Entity');
      expect(shell.getActiveFile('World')).toBe('a.toml');

      shell.switchMode('World');
      expect(shell.getActiveFile('World')).toBe('a.toml');
    });

    it('setActiveFile with invalid mode is silently ignored', () => {
      const shell = new ModeShell();
      shell.setActiveFile('NonExistent', 'file.toml');
      expect(shell.getActiveFile('NonExistent')).toBeNull();
    });
  });

  describe('active layer per mode (write-destination)', () => {
    it('starts with no active layer', () => {
      const shell = new ModeShell();
      expect(shell.getActiveLayer('World')).toBeNull();
    });

    it('setActiveLayer stores the write-destination layer for a mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['worlds/a.toml', 'worlds/b.toml']);
      shell.setActiveLayer('World', 'worlds/b.toml');
      expect(shell.getActiveLayer('World')).toBe('worlds/b.toml');
    });

    it('active layer is independent from active file', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['worlds/a.toml', 'worlds/b.toml']);
      shell.setActiveFile('World', 'worlds/a.toml');
      shell.setActiveLayer('World', 'worlds/b.toml');
      expect(shell.getActiveFile('World')).toBe('worlds/a.toml');
      expect(shell.getActiveLayer('World')).toBe('worlds/b.toml');
    });

    it('active layer and active file remain independent after mode switch and back', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('World', ['worlds/a.toml', 'worlds/b.toml']);
      shell.setActiveFile('World', 'worlds/a.toml');
      shell.setActiveLayer('World', 'worlds/b.toml');

      shell.switchMode('Entity');
      shell.setActiveFile('Entity', 'entities/c.toml');
      shell.setActiveLayer('Entity', 'entities/d.toml');

      shell.switchMode('World');
      expect(shell.getActiveFile('World')).toBe('worlds/a.toml');
      expect(shell.getActiveLayer('World')).toBe('worlds/b.toml');
    });

    it('active layer is independent per mode', () => {
      const shell = new ModeShell();
      shell.setActiveLayer('World', 'worlds/a.toml');
      shell.setActiveLayer('Entity', 'entities/b.toml');
      expect(shell.getActiveLayer('World')).toBe('worlds/a.toml');
      expect(shell.getActiveLayer('Entity')).toBe('entities/b.toml');
    });

    it('setActiveLayer with invalid mode is silently ignored', () => {
      const shell = new ModeShell();
      shell.setActiveLayer('NonExistent', 'file.toml');
      expect(shell.getActiveLayer('NonExistent')).toBeNull();
    });
  });

  describe('undo history per file per mode', () => {
    it('starts with empty undo history', () => {
      const shell = new ModeShell();
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([]);
    });

    it('pushUndoEntry appends to history', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { snapshot: 'v1' });
      shell.pushUndoEntry('World', 'a.toml', { snapshot: 'v2' });
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([
        { snapshot: 'v1' },
        { snapshot: 'v2' },
      ]);
    });

    it('undo history is independent per file', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { snapshot: 'a1' });
      shell.pushUndoEntry('World', 'b.toml', { snapshot: 'b1' });
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([{ snapshot: 'a1' }]);
      expect(shell.getUndoHistory('World', 'b.toml')).toEqual([{ snapshot: 'b1' }]);
    });

    it('undo history persists across mode switches', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { snapshot: 'v1' });

      shell.switchMode('Entity');
      shell.pushUndoEntry('Entity', 'b.toml', { snapshot: 'e1' });

      shell.switchMode('World');
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([{ snapshot: 'v1' }]);
    });

    it('getUndoHistory for unknown mode returns empty array', () => {
      const shell = new ModeShell();
      expect(shell.getUndoHistory('NonExistent', 'x.toml')).toEqual([]);
    });
  });

  describe('undoActive / redoActive / clearUndoHistory (UndoStack integration)', () => {
    it('undoActive pops the most recent entry', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { v: 1 });
      shell.pushUndoEntry('World', 'a.toml', { v: 2 });

      const popped = shell.undoActive('World', 'a.toml');
      expect(popped).toEqual({ v: 2 });
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([{ v: 1 }]);
    });

    it('undoActive returns null with no history', () => {
      const shell = new ModeShell();
      expect(shell.undoActive('World', 'a.toml')).toBeNull();
    });

    it('redoActive restores the last undone entry', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { v: 1 });
      shell.undoActive('World', 'a.toml');

      const restored = shell.redoActive('World', 'a.toml');
      expect(restored).toEqual({ v: 1 });
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([{ v: 1 }]);
    });

    it('redoActive returns null with empty redo stack', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { v: 1 });
      // No undo yet → nothing to redo.
      expect(shell.redoActive('World', 'a.toml')).toBeNull();
    });

    it('pushUndoEntry after undo clears the redo stack', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { v: 1 });
      shell.undoActive('World', 'a.toml');
      shell.pushUndoEntry('World', 'a.toml', { v: 99 });
      // Redo should now be empty.
      expect(shell.redoActive('World', 'a.toml')).toBeNull();
    });

    it('clearUndoHistory empties both stacks for that file', () => {
      const shell = new ModeShell();
      shell.pushUndoEntry('World', 'a.toml', { v: 1 });
      shell.pushUndoEntry('World', 'a.toml', { v: 2 });
      shell.clearUndoHistory('World', 'a.toml');
      expect(shell.getUndoHistory('World', 'a.toml')).toEqual([]);
      expect(shell.undoActive('World', 'a.toml')).toBeNull();
      expect(shell.redoActive('World', 'a.toml')).toBeNull();
    });

    it('clearUndoHistory on unknown file is a no-op', () => {
      const shell = new ModeShell();
      expect(() => shell.clearUndoHistory('World', 'never.toml')).not.toThrow();
    });
  });

  describe('full persistence scenario (AC integration test)', () => {
    it('open file A in World → switch to Entity → open file B → switch back → file A is still open and active', () => {
      const shell = new ModeShell();

      // Open file A in World Mode and mark it active
      shell.setOpenFiles('World', ['worlds/scenario-a.toml']);
      shell.setActiveFile('World', 'worlds/scenario-a.toml');
      shell.markDirty('World', 'worlds/scenario-a.toml', true);

      // Switch to Entity Mode
      shell.switchMode('Entity');
      expect(shell.getCurrentMode()).toBe('Entity');

      // Open file B in Entity Mode
      shell.setOpenFiles('Entity', ['entities/entity-b.toml']);
      shell.setActiveFile('Entity', 'entities/entity-b.toml');

      // Switch back to World Mode
      shell.switchMode('World');
      expect(shell.getCurrentMode()).toBe('World');

      // File A is still open
      expect(shell.getOpenFiles('World')).toContain('worlds/scenario-a.toml');

      // File A is still the active file
      expect(shell.getActiveFile('World')).toBe('worlds/scenario-a.toml');

      // File A is still dirty
      expect(shell.isDirty('World', 'worlds/scenario-a.toml')).toBe(true);
    });
  });
});
