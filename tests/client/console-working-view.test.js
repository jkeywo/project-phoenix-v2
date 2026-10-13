import { describe, it, expect, vi } from 'vitest';
import { createConsoleViewSession, normalizeConsoleView } from '../../gui/console-working-view.js';
import { ACTION_MAP } from '../../gui/action-map.js';

describe('Console working-view continuity', () => {
  it('merges local field changes and never sends operator preferences', () => {
    const send = vi.fn(), apply = vi.fn();
    const session = createConsoleViewSession({ send, apply });
    session.receive({ inspected_record: 'record', overlay: 'intel' }, 'pilot');
    session.change({ navigation: { zoom: 2, center_x: 300, center_y: -200, selected_uuid: null }, contrast: 'high' });
    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0][0]).toEqual({ inspected_record: 'record', overlay: 'intel', comms_thread: null,
      navigation: { zoom: 2, center_x: 300, center_y: -200, selected_uuid: null } });
  });
  it('applies without echo and ignores duplicate old snapshots during a local gesture', () => {
    const send = vi.fn();
    let session;
    session = createConsoleViewSession({ send, apply: () => session.change({ overlay: 'echo' }) });
    session.receive({ overlay: 'old' }, 'pilot');
    expect(send).not.toHaveBeenCalled();
    session.change({ overlay: 'new' });
    expect(session.receive({ overlay: 'old' }, 'pilot')).toBe(false);
    expect(session.value.overlay).toBe('new');
    expect(session.receive({ overlay: 'old' }, 'engineer')).toBe(true);
    expect(session.value.overlay).toBe('old');
    expect(send).toHaveBeenCalledTimes(1);
  });
  it('keeps a newer local gesture while an earlier gesture is acknowledged', () => {
    const send = vi.fn(), apply = vi.fn();
    const session = createConsoleViewSession({ send, apply });
    session.receive({}, 'helm');
    session.change({ inspected_record: 'first' });
    const first = session.value;
    session.change({ inspected_record: 'second' });
    const second = session.value;
    expect(session.receive(first, 'helm')).toBe(false);
    expect(session.value.inspected_record).toBe('second');
    expect(session.receive(second, 'helm')).toBe(true);
    expect(session.value.inspected_record).toBe('second');
  });
  it('bounds supported chart zoom and removes unsupported data', () => {
    expect(normalizeConsoleView({ navigation: { zoom: 900, center_x: 1, center_y: 2 }, secret: true }).navigation.zoom).toBe(8);
    expect(normalizeConsoleView({ navigation: { zoom: 1, center_x: Infinity, center_y: 2 } }).navigation).toBeNull();
  });
  it('uses the ordinary admitted message without issuing a System command', () => {
    const send = vi.fn(), view = normalizeConsoleView({ overlay: 'intel' });
    ACTION_MAP.set_console_view({ console: 'science-left', view }, send);
    expect(send.mock.calls).toEqual([['SetConsoleView', { console: 'science-left', view }]]);
  });
});
