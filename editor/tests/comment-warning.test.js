import { describe, it, expect } from 'vitest';
import { CommentWarning } from '../comment-warning.js';

describe('CommentWarning', () => {
  it('shouldWarn returns true for first save of a file with comments', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('# this is a comment\nkey = value')).toBe(true);
  });

  it('shouldWarn returns false for file without comments', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('key = value\nother = 42')).toBe(false);
  });

  it('after acknowledge, shouldWarn returns false', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('# comment')).toBe(true);
    w.acknowledge();
    expect(w.shouldWarn('# comment')).toBe(false);
  });

  it('isAcknowledged returns correct state', () => {
    const w = new CommentWarning();
    expect(w.isAcknowledged()).toBe(false);
    w.acknowledge();
    expect(w.isAcknowledged()).toBe(true);
  });

  it('reset clears the state', () => {
    const w = new CommentWarning();
    w.acknowledge();
    expect(w.isAcknowledged()).toBe(true);
    w.reset();
    expect(w.isAcknowledged()).toBe(false);
    expect(w.shouldWarn('# comment')).toBe(true);
  });

  it('files without # anywhere never trigger warning', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('title = "Hello World"\ncount = 3')).toBe(false);
    expect(w.shouldWarn('')).toBe(false);
    expect(w.shouldWarn('   \n\n')).toBe(false);
  });

  it('comment inside a string value is still detected (simple heuristic)', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('key = "value with # inside"')).toBe(true);
  });

  it('multiple calls to shouldWarn for same file without acknowledge still returns true', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('# comment')).toBe(true);
    expect(w.shouldWarn('# comment')).toBe(true);
    expect(w.shouldWarn('# comment')).toBe(true);
  });

  it('indented comments are detected', () => {
    const w = new CommentWarning();
    expect(w.shouldWarn('  # indented comment\nkey = 1')).toBe(true);
    expect(w.shouldWarn('\t# tab-indented comment\nkey = 1')).toBe(true);
  });
});
