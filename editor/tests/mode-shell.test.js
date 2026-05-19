import { describe, it, expect } from 'vitest';
import { ModeShell } from '../mode-shell.js';

describe('mode-shell', () => {
  describe('constructor', () => {
    it('starts with Scenario as default mode', () => {
      const shell = new ModeShell();
      expect(shell.getCurrentMode()).toBe('Scenario');
    });

    it('accepts custom modes', () => {
      const shell = new ModeShell(['Red', 'Green', 'Blue']);
      expect(shell.getCurrentMode()).toBe('Red');
    });

    it('starts with empty open files for all modes', () => {
      const shell = new ModeShell();
      expect(shell.getOpenFiles('Scenario')).toEqual([]);
      expect(shell.getOpenFiles('Entity')).toEqual([]);
      expect(shell.getOpenFiles('Definitions')).toEqual([]);
    });
  });

  describe('switchMode', () => {
    it('switches to a valid mode', () => {
      const shell = new ModeShell();
      expect(shell.getCurrentMode()).toBe('Scenario');

      const result = shell.switchMode('Entity');
      expect(result).toBe(true);
      expect(shell.getCurrentMode()).toBe('Entity');
    });

    it('refuses to switch to an invalid mode', () => {
      const shell = new ModeShell();
      const result = shell.switchMode('InvalidMode');
      expect(result).toBe(false);
      expect(shell.getCurrentMode()).toBe('Scenario');
    });

    it('switching to the current mode is a no-op but returns true', () => {
      const shell = new ModeShell();
      const result = shell.switchMode('Scenario');
      expect(result).toBe(true);
      expect(shell.getCurrentMode()).toBe('Scenario');
    });
  });

  describe('getModes', () => {
    it('returns the list of registered modes', () => {
      const shell = new ModeShell();
      expect(shell.getModes()).toEqual(['Scenario', 'Entity', 'Definitions']);
    });
  });

  describe('open files per mode', () => {
    it('getOpenFiles returns empty array for a mode with no files', () => {
      const shell = new ModeShell();
      expect(shell.getOpenFiles('Entity')).toEqual([]);
    });

    it('setOpenFiles stores files for the current mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('Scenario', ['worlds/default.toml', 'worlds/patrol.toml']);
      expect(shell.getOpenFiles('Scenario')).toEqual(['worlds/default.toml', 'worlds/patrol.toml']);
    });

    it('setOpenFiles stores files for a non-current mode', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('Definitions', ['defs.toml']);
      expect(shell.getOpenFiles('Definitions')).toEqual(['defs.toml']);
    });

    it('mode switching preserves open files per mode', () => {
      const shell = new ModeShell();

      shell.setOpenFiles('Scenario', ['scenario.toml']);
      shell.switchMode('Entity');
      shell.setOpenFiles('Entity', ['entity.toml']);

      expect(shell.getOpenFiles('Scenario')).toEqual(['scenario.toml']);
      expect(shell.getOpenFiles('Entity')).toEqual(['entity.toml']);

      shell.switchMode('Scenario');
      expect(shell.getOpenFiles('Scenario')).toEqual(['scenario.toml']);
      expect(shell.getCurrentMode()).toBe('Scenario');
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
      shell.setOpenFiles('Scenario', ['a.toml']);
      expect(shell.isDirty('Scenario', 'a.toml')).toBe(false);
    });

    it('markDirty sets a file as dirty', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('Scenario', ['a.toml']);
      shell.markDirty('Scenario', 'a.toml', true);
      expect(shell.isDirty('Scenario', 'a.toml')).toBe(true);
    });

    it('markDirty(false) clears the dirty flag', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('Scenario', ['a.toml']);
      shell.markDirty('Scenario', 'a.toml', true);
      shell.markDirty('Scenario', 'a.toml', false);
      expect(shell.isDirty('Scenario', 'a.toml')).toBe(false);
    });

    it('hasAnyDirty returns true when any file in any mode is dirty', () => {
      const shell = new ModeShell();
      shell.setOpenFiles('Scenario', ['a.toml']);
      shell.setOpenFiles('Entity', ['b.toml']);
      expect(shell.hasAnyDirty()).toBe(false);
      shell.markDirty('Entity', 'b.toml', true);
      expect(shell.hasAnyDirty()).toBe(true);
    });
  });
});
