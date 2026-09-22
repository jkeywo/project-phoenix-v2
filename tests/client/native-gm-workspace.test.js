// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mountNativeGmWorkspace } from '../../gui/native-gm-workspace.js';
import { t } from '../../gui/strings.js';

function mount(overrides = {}) {
  let operator = { id: 'native-gm', name: 'GM', connected: true, ready: false };
  let listener;
  const bridge = {
    getOperator: () => operator,
    submitAction: vi.fn(() => true),
    setReady: vi.fn(() => true),
    forceStart: vi.fn(() => true),
    returnToHostLobby: vi.fn(() => true),
    subscribe: fn => { listener = fn; return () => { listener = null; }; },
    ...overrides,
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
      <div id="landing-panel">Host landing</div>
      <div id="scenario-panel">Host world picker</div>
      <div id="wasm-spinner">Host simulation loading</div>
      <section id="gm-start-controls"><button id="gm-ready-btn"></button>
        <button id="gm-force-start-btn"></button><p id="gm-start-policy"></p>
        <p id="gm-start-result"></p></section>
      <section id="gm-session-controls"><h2 id="gm-session-heading"></h2>
        <button id="gm-session-pause"></button><button id="gm-session-resume"></button>
        <p id="gm-session-state"></p><p id="gm-session-feedback"></p>
        <h3 id="gm-session-log-heading"></h3><ol id="gm-session-log"></ol></section>`;
  });

  it('sends classification through the real native GM adapter without changing operator scope', () => {
    const app = mount();
    const request = { ship: 'observer', target: 'target', palette: 'freighter', correlation: 'classify-1' };
    expect(window.__hostSetContactClassification(request)).toBe(true);
    expect(app.bridge.submitAction).toHaveBeenCalledExactlyOnceWith({ ...request,
      action: 'set_contact_classification', operator_id: 'native-gm' });
    expect(window.__hostSetContactClassification({ ...request, operator_id: 'other' })).toBe(false);
    app.view.dispose(); expect(window.__hostSetContactClassification(request)).toBe(false);
  });

  it('keeps presentation interest off the authoritative action lane', () => {
    const inspectorInterest = vi.fn(() => true), consoleInterest = vi.fn(() => true);
    const app = mount({inspectorInterest, consoleInterest});
    const request = {consumer:'console', ship:'player', station:'helm', visible:true, mount_generation:2, world_generation:3};
    window.__hostGmConsoleInterest(request);
    window.__hostGmInspectorInterest(['hull-fields']);
    expect(consoleInterest).toHaveBeenCalledWith(request);
    expect(inspectorInterest).toHaveBeenCalledWith(['hull-fields']);
    expect(app.bridge.submitAction).not.toHaveBeenCalled();
    app.view.dispose();
  });
  it('shares one location between Ready, Pause and Resume', () => {
    document.body.insertAdjacentHTML('beforeend', '<div id="gm-session-actions"></div>');
    const app = mount();
    const ready = document.getElementById('gm-header-ready');
    const pause = document.getElementById('gm-session-pause');
    const resume = document.getElementById('gm-session-resume');
    expect(ready.parentElement.id).toBe('gm-session-actions');
    expect([ready.hidden, pause.hidden, resume.hidden]).toEqual([false, true, true]);
    app.receive('metadata', { phase: 'InProgress', gms: [] });
    app.receive('gm_session', { paused: false, results: [] });
    expect([ready.hidden, pause.hidden, resume.hidden]).toEqual([true, false, true]);
    app.receive('gm_session', { paused: true, results: [] });
    expect([ready.hidden, pause.hidden, resume.hidden]).toEqual([true, true, false]);
    app.view.dispose();
  });
  it('waits for native save completion and surfaces write failures', async () => {
    document.body.insertAdjacentHTML('beforeend', '<section id="manual-save-panel"></section>');
    const saveRequest = vi.fn().mockResolvedValue('native-slot');
    const app = mount({ saveRequest });
    app.receive('metadata', { phase: 'InProgress', gms: [] });
    const panel = document.getElementById('manual-save-panel');
    panel.querySelector('input').value = 'Before battle';
    panel.querySelector('button').click();
    await Promise.resolve();
    expect(saveRequest).toHaveBeenCalledWith('create', 'Before battle');
    expect(panel.querySelector('button').disabled).toBe(true);
    app.receive('save_outcomes', [{ slot: 'native-slot', ok: false, error: 'Disk full' }]);
    expect(panel.querySelector('[role="status"]').textContent).toBe('Disk full');
    expect(panel.querySelector('button').disabled).toBe(false);
    app.view.dispose();
  });
  it('requires an explicit native private audio provider even without an operator capability declaration', async () => {
    delete window.PhoenixOperatorCapabilities;
    const audioContext = vi.fn(); window.AudioContext = audioContext;
    const app = mount();
    await window.__privateAudio.ready;
    expect(window.__privateAudio.state().status).toBe('unavailable');
    expect(await window.__privateAudio.enable()).toBe(false);
    expect(audioContext).not.toHaveBeenCalled();
    app.view.dispose(); delete window.AudioContext;
  });

  it('uses the ordinary GM session projection and submits attributed absolute pause state', () => {
    const app = mount();
    app.receive('metadata', { phase: 'InProgress', gms: [] });
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

  it('submits an attributed player-slot backfill through the ordinary GM action bridge', () => {
    const app = mount();
    expect(window.__hostBackfillShipSlot({ slot: 'wing', correlation: 'slot-1' })).toBe(true);
    expect(app.bridge.submitAction).toHaveBeenCalledWith({
      slot: 'wing', correlation: 'slot-1', action: 'backfill_ship_slot', operator_id: 'native-gm',
    });
    app.view.dispose();
  });

  it('uses authoritative readiness and permits no launch action after the lobby', () => {
    const app = mount();
    expect(document.getElementById('landing-panel')).toBeNull();
    expect(document.getElementById('scenario-panel')).toBeNull();
    expect(document.getElementById('wasm-spinner')).toBeNull();
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

  it('shows native readiness totals and attributed authoritative start outcomes', () => {
    const app = mount();
    const metadata = { phase: 'Lobby',
      gms: [{ id: 'native-gm', name: 'GM', connected: true, ready: false }],
      start_policy: { ready_total: 1, connected_total: 3 } };
    app.receive('metadata', { ...metadata, start_result: {
      grant_id: 'start-1', operator_id: 'native-gm', status: 'refused',
      reason: 'validation-failed', tick: 0,
    } });
    expect(document.getElementById('gm-start-policy').textContent).toBe(
      t('server.gm.start.summary', { ready: 1, connected: 3 }));
    expect(document.getElementById('gm-start-result').textContent).toBe(
      t('server.gm.start.validation_failed'));
    app.receive('metadata', { ...metadata, phase: 'InProgress', start_result: {
      grant_id: 'start-2', operator_id: 'native-gm', status: 'applied',
      reason: null, tick: 4,
    } });
    expect(document.getElementById('gm-start-result').textContent).toBe(
      t('server.gm.start.force_applied', { name: 'GM' }));
    app.view.dispose();
  });
  it('offers host-lobby recovery only before launch while the viewscreen is unavailable', () => {
    const app = mount();
    const button = document.getElementById('gm-return-to-host-lobby');
    expect(button.hidden).toBe(true);
    app.receive('metadata', {phase: 'Lobby', host_lobby_unavailable: true, gms: []});
    expect(button.hidden).toBe(false);
    expect(button.textContent).toBe(t('server.gm.return_to_host_lobby'));
    button.click();
    button.dispatchEvent(new Event('click'));
    expect(app.bridge.returnToHostLobby).toHaveBeenCalledTimes(1);
    expect(button.disabled).toBe(true);
    app.receive('metadata', {phase: 'InProgress', host_lobby_unavailable: true, gms: []});
    expect(button.hidden).toBe(true);
    button.dispatchEvent(new Event('click'));
    app.receive('metadata', {phase: 'Lobby', host_lobby_unavailable: false, gms: []});
    expect(button.hidden).toBe(true);
    button.dispatchEvent(new Event('click'));
    expect(app.bridge.returnToHostLobby).toHaveBeenCalledTimes(1);
    expect(app.bridge.submitAction).not.toHaveBeenCalled();
    app.view.dispose();
    expect(document.getElementById('gm-return-to-host-lobby')).toBeNull();
  });
});
