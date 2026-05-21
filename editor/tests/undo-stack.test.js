import { describe, it, expect } from 'vitest';
import { UndoStack } from '../undo-stack.js';

describe('UndoStack', () => {
  describe('empty stack', () => {
    it('canUndo returns false', () => {
      const stack = new UndoStack();
      expect(stack.canUndo()).toBe(false);
    });

    it('canRedo returns false', () => {
      const stack = new UndoStack();
      expect(stack.canRedo()).toBe(false);
    });

    it('undo returns null', () => {
      const stack = new UndoStack();
      expect(stack.undo()).toBeNull();
    });

    it('redo returns null', () => {
      const stack = new UndoStack();
      expect(stack.redo()).toBeNull();
    });
  });

  describe('push one', () => {
    it('canUndo returns true after push', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      expect(stack.canUndo()).toBe(true);
    });

    it('undo returns the entry', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      expect(stack.undo()).toEqual({ change: 'a' });
    });

    it('canUndo returns false after undo', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.undo();
      expect(stack.canUndo()).toBe(false);
    });
  });

  describe('push then undo then redo', () => {
    it('redo returns the entry', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.undo();
      expect(stack.redo()).toEqual({ change: 'a' });
    });

    it('canRedo returns false after redo', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.undo();
      stack.redo();
      expect(stack.canRedo()).toBe(false);
    });
  });

  describe('push three, undo all three', () => {
    it('returns entries in reverse order (LIFO)', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.push({ change: 'b' });
      stack.push({ change: 'c' });

      expect(stack.undo()).toEqual({ change: 'c' });
      expect(stack.undo()).toEqual({ change: 'b' });
      expect(stack.undo()).toEqual({ change: 'a' });
    });
  });

  describe('edit after undo clears redo branch', () => {
    it('push after undo makes canRedo false', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.undo();
      expect(stack.canRedo()).toBe(true);
      stack.push({ change: 'b' });
      expect(stack.canRedo()).toBe(false);
    });
  });

  describe('cap at maxOps evicts oldest', () => {
    it('push 101 entries keeps undoCount at 100', () => {
      const stack = new UndoStack(100);
      for (let i = 0; i < 101; i++) {
        stack.push({ change: i });
      }
      expect(stack.getUndoCount()).toBe(100);
    });

    it('oldest entry is evicted after overflow', () => {
      const stack = new UndoStack(3);
      stack.push({ change: 'a' });
      stack.push({ change: 'b' });
      stack.push({ change: 'c' });
      stack.push({ change: 'd' });
      expect(stack.getUndoCount()).toBe(3);
      expect(stack.undo()).toEqual({ change: 'd' });
      expect(stack.undo()).toEqual({ change: 'c' });
      expect(stack.undo()).toEqual({ change: 'b' });
    });
  });

  describe('save clears stack', () => {
    it('clear makes canUndo false and undoCount 0', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.push({ change: 'b' });
      stack.clear();
      expect(stack.canUndo()).toBe(false);
      expect(stack.getUndoCount()).toBe(0);
    });

    it('clear also empties redo stack', () => {
      const stack = new UndoStack();
      stack.push({ change: 'a' });
      stack.undo();
      expect(stack.canRedo()).toBe(true);
      stack.clear();
      expect(stack.canRedo()).toBe(false);
      expect(stack.getRedoCount()).toBe(0);
    });
  });

  describe('push null entry', () => {
    it('still increments undo count', () => {
      const stack = new UndoStack();
      stack.push(null);
      expect(stack.getUndoCount()).toBe(1);
      expect(stack.undo()).toBeNull();
    });

    it('undefined entry also increments count', () => {
      const stack = new UndoStack();
      stack.push(undefined);
      expect(stack.getUndoCount()).toBe(1);
      expect(stack.undo()).toBeUndefined();
    });
  });

  describe('multiple undo/redo cycles', () => {
    it('maintains correct state through alternating cycles', () => {
      const stack = new UndoStack();

      stack.push({ change: 'a' });
      stack.push({ change: 'b' });
      stack.push({ change: 'c' });

      // undo twice: c, b
      expect(stack.undo()).toEqual({ change: 'c' });
      expect(stack.undo()).toEqual({ change: 'b' });
      expect(stack.getUndoCount()).toBe(1);
      expect(stack.getRedoCount()).toBe(2);

      // redo once: b
      expect(stack.redo()).toEqual({ change: 'b' });
      expect(stack.getUndoCount()).toBe(2);
      expect(stack.getRedoCount()).toBe(1);

      // undo once: b
      expect(stack.undo()).toEqual({ change: 'b' });
      expect(stack.getUndoCount()).toBe(1);
      expect(stack.getRedoCount()).toBe(2);

      // redo twice: b, c
      expect(stack.redo()).toEqual({ change: 'b' });
      expect(stack.redo()).toEqual({ change: 'c' });
      expect(stack.getUndoCount()).toBe(3);
      expect(stack.getRedoCount()).toBe(0);
    });
  });

  describe('default maxOps', () => {
    it('defaults to 100', () => {
      const stack = new UndoStack();
      expect(stack.getUndoCount()).toBe(0);
      for (let i = 0; i < 100; i++) {
        stack.push({ change: i });
      }
      expect(stack.getUndoCount()).toBe(100);
      stack.push({ change: 'overflow' });
      expect(stack.getUndoCount()).toBe(100);
    });
  });
});
