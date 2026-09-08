// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mountNativeGmWorkspace } from '../../gui/native-gm-workspace.js';
import { t } from '../../gui/strings.js';

function mount() {
  let operator = { id: 'native-gm', name: 'GM', connected: true, ready: false };
  let listener;
  const bridge = {
    getOperator: () => operator,
    submitAction: vi.fn(() => true),
    setReady: vi.fn(() => true),
    forceStart: vi.fn(() => true),
    subscribe: fn => { listener = fn; return () => { listener = null; }; },
  };
  const view = mountNativeGmWorkspace({ bridge, win: window, doc: document });
  return { bridge, view,
    receive: (channel, payload) => listener?.(channel, payload),
    operator: value => { operator = { ...operator, ...value }; },
  };
}

describe('native GM workspace over the shared GM presenters', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-start-controls"><button id="gm-ready-btn"></button>
        <button id="gm-force-start-btn"></button><p id="gm-start-policy"></p>
        <p id="gm-start-result"></p></section>
      <section id="gm-session-controls"><h2 id="gm-session-heading"></h2>
        <button id="gm-session-pause"></button><button id="gm-session-resume"></button>
        <p id="gm-session-state"></p><p id="gm-session-feedback"></p>
        <h3 id="gm-session-log-heading"></h3><ol id="gm-session-log"></ol></section>`;
  });

  it('uses the ordinary GM session projection and submits attributed absolute pause state', () => {
    const app = mount();
    app.receive('gm_session', { paused: false, results: [] });
    document.getElementById('gm-session-pause').click();
    expect(app.bridge.submitAction).toHaveBeenCalledWith(expect.objectContaining({
      operator_id: 'native-gm', action: 'set_session_paused', active: true,
      correlation: expect.any(String),
    }));
    app.view.dispose();
  });

  it('retains the existing station-command shape and refuses a different operator', () => {
    const app = mount();
    const request = { ship: 'ship-1', station: 'helm', target: 'drive',
      correlation: 'native-action-1', payload: { type: 'SetThrottle', value: 0 } };
    expect(window.__hostIssueStationCommand(request)).toBe(true);
    expect(app.bridge.submitAction).toHaveBeenCalledWith({ ...request,
      action: 'issue_station_command', operator_id: 'native-gm' });
    expect(window.__hostIssueStationCommand({ ...request, operator_id: 'another-gm' })).toBe(false);
    app.operator({ connected: false });
    expect(window.__hostSetSessionPaused(false, 'lost-screen')).toBe(false);
    expect(app.bridge.submitAction).toHaveBeenCalledTimes(1);
    app.view.dispose();
  });

  it('uses authoritative readiness and permits no launch action after the lobby', () => {
    const app = mount();
    document.getElementById('gm-ready-btn').click();
    expect(app.bridge.setReady).toHaveBeenCalledWith(true);
    app.operator({ ready: true });
    app.receive('metadata', { phase: 'Lobby', gms: [] });
    expect(document.getElementById('gm-ready-btn').textContent).toBe(t('server.gm.start.unready'));
    document.getElementById('gm-force-start-btn').click();
    expect(app.bridge.forceStart).toHaveBeenCalledTimes(1);
    app.receive('metadata', { phase: 'InProgress', gms: [] });
    expect(document.getElementById('gm-ready-btn').disabled).toBe(true);
    expect(document.getElementById('gm-force-start-btn').disabled).toBe(true);
    document.getElementById('gm-force-start-btn').click();
    expect(app.bridge.forceStart).toHaveBeenCalledTimes(1);
    app.view.dispose();
  });

  it('does not turn screen recovery into a resume command', () => {
    const app = mount();
    app.receive('metadata', { phase: 'InProgress', gms: [] });
    app.receive('gm_session', { paused: true, results: [] });
    expect(app.bridge.submitAction).not.toHaveBeenCalled();
    document.getElementById('gm-session-resume').click();
    expect(app.bridge.submitAction).toHaveBeenCalledWith(expect.objectContaining({
      action: 'set_session_paused', active: false, operator_id: 'native-gm',
    }));
    app.view.dispose();
    expect(window.__hostSetSessionPaused(false, 'closed-screen')).toBe(false);
  });
});
