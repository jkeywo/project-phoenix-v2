// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createGmSessionWidget } from '../../gui/gm-session-widget.js';
import { createGmRolePresets } from '../../gui/gm-role-presets.js';

describe('single-owner session widget', () => {
  let widget, operator;
  beforeEach(() => {
    document.body.innerHTML = '<div id="gm-session-actions"><button id="gm-session-pause"></button><button id="gm-session-resume"></button></div><button id="gm-ready-btn"></button><select id="gm-role-preset-select"></select>';
    operator = { id: 'gm-1', ready: false, connected: true };
    widget = createGmSessionWidget({ doc: document, t: id => id, getOperator: () => operator });
  });
  const visible = () => [...document.querySelectorAll('#gm-session-actions button')].filter(node => !node.hidden).map(node => node.id);
  it.each([
    [{ phase: 'Lobby' }, 'gm-header-ready'],
    [{ phase: 'InProgress', paused: false }, 'gm-session-pause'],
    [{ phase: 'InProgress', paused: true }, 'gm-session-resume'],
    [{ phase: 'GameOver', paused: false }, null],
    [{ phase: 'GameOver', paused: true }, null],
  ])('renders exactly the world-state control %j regardless of preset switching', (state, expected) => {
    const presets = createGmRolePresets({ doc: document });
    presets.setAvailablePresets([{ id: 'director', panels: [], quick_actions: ['pause', 'resume'] }]);
    widget.update(state);
    for (let i = 0; i < 6; i++) {
      presets.select(i % 2 ? 'all' : 'director');
      expect(visible()).toEqual(expected ? [expected] : []);
    }
  });
  it('disables unknown pause state and disconnected controls', () => {
    widget.update({ phase: 'InProgress' });
    expect(document.getElementById('gm-session-pause').disabled).toBe(true);
    widget.update({ paused: false });
    expect(document.getElementById('gm-session-pause').disabled).toBe(false);
    operator.connected = false; widget.render();
    expect(document.getElementById('gm-session-pause').disabled).toBe(true);
  });
  it('forwards Ready without optimistically changing authoritative readiness', () => {
    const ready = document.getElementById('gm-header-ready'), send = vi.fn();
    document.getElementById('gm-ready-btn').addEventListener('click', send);
    widget.update({ phase: 'Lobby' }); ready.click();
    expect(send).toHaveBeenCalledOnce();
    expect(ready.textContent).toBe('server.gm.start.ready');
    operator.ready = true; widget.render();
    expect(ready.textContent).toBe('server.gm.start.unready');
  });
  it.each([true, false])('accepts phase and pause channels in either order (pause first: %s)', pauseFirst => {
    const messages = [{ paused: false }, { phase: 'InProgress' }];
    for (const message of pauseFirst ? messages : messages.toReversed()) widget.update(message);
    expect(document.getElementById('gm-session-pause').disabled).toBe(false);
    widget.reset();
    widget.update({ phase: 'InProgress' });
    expect(document.getElementById('gm-session-pause').disabled).toBe(true);
  });
});
