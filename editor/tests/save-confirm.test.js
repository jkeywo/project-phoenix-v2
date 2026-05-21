import { describe, it, expect, beforeEach } from 'vitest';
import {
  getCommentWarning,
  confirmSaveIfCommented,
  resetCommentWarningOnRootChange,
  _resetSingletonForTest,
} from '../save-confirm.js';

describe('save-confirm', () => {
  beforeEach(() => {
    _resetSingletonForTest();
  });

  it('first commented save prompts and returns true when user accepts', () => {
    const calls = [];
    const confirmFn = (msg) => { calls.push(msg); return true; };
    const ok = confirmSaveIfCommented('# hi\nkey = 1', { confirmFn });
    expect(ok).toBe(true);
    expect(calls.length).toBe(1);
  });

  it('declining returns false and does not acknowledge', () => {
    const confirmFn = () => false;
    const ok = confirmSaveIfCommented('# hi', { confirmFn });
    expect(ok).toBe(false);
    expect(getCommentWarning().isAcknowledged()).toBe(false);
  });

  it('after accepting, subsequent commented saves are silent', () => {
    let count = 0;
    const confirmFn = () => { count++; return true; };
    confirmSaveIfCommented('# first', { confirmFn });
    confirmSaveIfCommented('# second', { confirmFn });
    confirmSaveIfCommented('# third', { confirmFn });
    expect(count).toBe(1);
  });

  it('uncommented saves never prompt', () => {
    let count = 0;
    const confirmFn = () => { count++; return true; };
    const ok = confirmSaveIfCommented('key = "value"', { confirmFn });
    expect(ok).toBe(true);
    expect(count).toBe(0);
  });

  it('reset (via onRootChanged listener) re-arms the prompt', () => {
    let count = 0;
    const confirmFn = () => { count++; return true; };
    confirmSaveIfCommented('# first', { confirmFn });
    expect(count).toBe(1);

    // Wire a fake onRootChanged surface and fire it.
    let listener = null;
    const onRootChanged = (cb) => {
      listener = cb;
      return { unsubscribe: () => { listener = null; } };
    };
    resetCommentWarningOnRootChange({ onRootChanged });
    listener();

    confirmSaveIfCommented('# second', { confirmFn });
    expect(count).toBe(2);
  });
});
