// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { createInspectorSession, bindInspectorNavigation } from '../../gui/inspector-session.js';

describe('Inspector session', () => {
  it('branches from the retraced subject and bounds navigation', () => {
    const session = createInspectorSession(3);
    for (const id of ['a', 'b', 'c']) session.select(id);
    expect(session.back()).toBe(true);
    session.select('d');
    expect(session.state()).toEqual({ selected: 'd', history: ['a', 'b', 'd'], cursor: 2 });
    expect(session.forward()).toBe(false);
    session.select('e');
    expect(session.state().history).toEqual(['b', 'd', 'e']);
    expect(session.back()).toBe(true);
    expect(session.back()).toBe(true);
    expect(session.back()).toBe(false);
    session.select('b');
    expect(session.canForward).toBe(true);
    const snapshot = session.state(); snapshot.history.length = 0;
    expect(session.state().history).toEqual(['b', 'd', 'e']);
  });

  it('recalls only the named identity and prunes readings abandoned by navigation', () => {
    const session = createInspectorSession(2);
    for (const id of ['a', 'b', 'c']) {
      session.select(id); session.remember(id, { name: id });
      session.pruneReadingsToHistory();
    }
    expect(session.reading('a')).toBeUndefined();
    expect(session.reading('unknown')).toBeUndefined();
    session.back();
    expect(session.reading(session.selected)).toEqual({ name: 'b' });
    session.select('d'); session.pruneReadingsToHistory();
    expect(session.reading('c')).toBeUndefined();
    expect(session.reading('b')).toEqual({ name: 'b' });
    session.reset();
    expect(session.state()).toEqual({ selected: null, history: [], cursor: -1 });
    expect(session.reading('b')).toBeUndefined();
    expect(session.canBack || session.canForward).toBe(false);
  });

  it('offers overflow eviction only when the identity is no longer in history', () => {
    const session = createInspectorSession(3);
    for (const id of ['a', 'b', 'a']) session.select(id);
    expect(session.select('c')).toBeNull();
    expect(session.select('d')).toBe('b');
    session.remember('b', { name: 'b' }); session.forget('b');
    expect(session.reading('b')).toBeUndefined();
  });

  it('repaints before focusing only when navigation moves', () => {
    document.body.innerHTML = '<button id="back"></button><button id="forward"></button><p id="status" tabindex="-1"></p>';
    const session = createInspectorSession(20);
    const back = document.getElementById('back'), forward = document.getElementById('forward');
    const status = document.getElementById('status');
    const render = vi.fn(() => { status.textContent = session.selected; refresh(); });
    const refresh = bindInspectorNavigation({ session, back, forward, status, render });
    session.select('a'); session.select('b'); refresh();
    expect(document.activeElement).not.toBe(status);
    back.click();
    expect(status.textContent).toBe('a');
    expect(document.activeElement).toBe(status);
    expect(back.disabled).toBe(true); expect(forward.disabled).toBe(false);
    back.dispatchEvent(new Event('click'));
    expect(render).toHaveBeenCalledTimes(1);
    forward.click();
    expect(status.textContent).toBe('b');
    session.reset(); refresh();
    expect(back.disabled && forward.disabled).toBe(true);
  });
});
