import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { initConsole } from '../../gui/console-core.js';

// ── Global window shim ───────────────────────────────────────────────────────
// Node has no `window`. Set global.window before each test so that
// console-core.js (which checks `typeof window !== 'undefined'`) uses it as
// `_root`, and bare `window` references in the tests resolve correctly.
beforeEach(() => {
  global.window = {};
  global.window.parent = global.window; // non-iframe baseline: window === window.parent
});
afterEach(() => {
  delete global.window;
});

// ── Helpers ──────────────────────────────────────────────────────────────────

/**
 * Minimal BroadcastChannel stub for the test environment (no real BC in Node).
 * Returns { instance, emit } where emit(data) fires onmessage on the instance.
 */
function makeBCStub() {
  const instance = { postMessage: vi.fn(), onmessage: null };
  const emit = (data) => { if (instance.onmessage) instance.onmessage({ data }); };
  return { instance, emit };
}

// ── Transport selection ───────────────────────────────────────────────────────

describe('sendAction — transport selection', () => {
  let sendAction, cleanup;

  function setup(windowOverrides = {}) {
    const bc = makeBCStub();
    const savedBC = global.BroadcastChannel;
    global.BroadcastChannel = function() { return bc.instance; };

    const savedWindow = { ...global.window };
    Object.assign(global.window, windowOverrides);

    const result = initConsole({ name: 'Test', render: () => {} });
    sendAction = result.sendAction;

    cleanup = () => {
      global.BroadcastChannel = savedBC;
      // Restore window properties
      for (const key of Object.keys(windowOverrides)) {
        delete global.window[key];
      }
    };
    return { bc };
  }

  afterEach(() => { if (cleanup) cleanup(); cleanup = null; });

  it('posts to parent when window !== window.parent (iframe mode)', () => {
    const parentPostMessage = vi.fn();
    // Simulate being inside an iframe: parent is a different object
    const fakeParent = { postMessage: parentPostMessage };
    const savedParent = Object.getOwnPropertyDescriptor(global.window, 'parent');
    Object.defineProperty(global.window, 'parent', { value: fakeParent, configurable: true });

    const { bc } = setup();
    sendAction('fire_phaser', { bank: 'fore' });

    expect(parentPostMessage).toHaveBeenCalledWith(
      { type: 'console_action', payload: JSON.stringify({ action: 'fire_phaser', console: 'Test', bank: 'fore' }) },
      '*'
    );
    expect(bc.instance.postMessage).not.toHaveBeenCalled();

    if (savedParent) {
      Object.defineProperty(global.window, 'parent', savedParent);
    } else {
      delete global.window.parent;
    }
  });

  it('calls window.ipc when present (wry mode)', () => {
    const ipcPost = vi.fn();
    setup({ ipc: { postMessage: ipcPost } });
    sendAction('helm_input', { thrust: 0.5, steering: 0.0 });
    const expected = JSON.stringify({ action: 'helm_input', console: 'Test', thrust: 0.5, steering: 0.0 });
    expect(ipcPost).toHaveBeenCalledWith(expected);
  });

  it('calls wasmBindings.wasm_ui_action when present (WASM mode)', () => {
    const wasmFn = vi.fn();
    setup({ wasmBindings: { wasm_ui_action: wasmFn } });
    sendAction('toggle_red_alert', {});
    const expected = JSON.stringify({ action: 'toggle_red_alert', console: 'Test' });
    expect(wasmFn).toHaveBeenCalledWith(expected);
  });

  it('does not call wasmBindings if wasm_ui_action is not a function', () => {
    const { bc } = setup({ wasmBindings: { wasm_ui_action: 'not-a-function' } });
    sendAction('toggle_red_alert', {});
    // Falls through to BroadcastChannel
    expect(bc.instance.postMessage).toHaveBeenCalled();
  });

  it('uses BroadcastChannel as final fallback', () => {
    const { bc } = setup();
    sendAction('dispatch_repair_team', { team_idx: 0, target: 'Helm' });
    const expected = JSON.stringify({ action: 'dispatch_repair_team', console: 'Test', team_idx: 0, target: 'Helm' });
    expect(bc.instance.postMessage).toHaveBeenCalledWith(
      { type: 'console_action', payload: expected }
    );
  });
});

// ── Action envelope shape ─────────────────────────────────────────────────────

describe('sendAction — envelope shape', () => {
  let sendAction;

  beforeEach(() => {
    const bc = makeBCStub();
    global.BroadcastChannel = function() { return bc.instance; };
    const result = initConsole({ name: 'Helm', render: () => {} });
    sendAction = result.sendAction;
    // Capture via BC
    global._testBC = bc;
  });

  function capturedPayload() {
    const call = global._testBC.instance.postMessage.mock.calls[0];
    return JSON.parse(call[0].payload);
  }

  it('injects console: name automatically', () => {
    sendAction('helm_input', { thrust: 0.1 });
    expect(capturedPayload().console).toBe('Helm');
  });

  it('sets action field from the first argument', () => {
    sendAction('start_impulse_charge', {});
    expect(capturedPayload().action).toBe('start_impulse_charge');
  });

  it('merges payload fields into the envelope', () => {
    sendAction('set_target', { uuid: 'abc-123' });
    const env = capturedPayload();
    expect(env.uuid).toBe('abc-123');
    expect(env.action).toBe('set_target');
    expect(env.console).toBe('Helm');
  });

  it('omits extra fields when payload is empty or absent', () => {
    sendAction('cancel_impulse');
    const env = capturedPayload();
    expect(Object.keys(env).sort()).toEqual(['action', 'console']);
  });
});

// ── Inbound: name filtering (BroadcastChannel) ───────────────────────────────

describe('initConsole — BroadcastChannel name filtering', () => {
  let bc, emit, renderCalls;

  beforeEach(() => {
    const stub = makeBCStub();
    bc = stub.instance;
    emit = stub.emit;
    global.BroadcastChannel = function() { return bc; };
    renderCalls = [];
    initConsole({ name: 'Repair', render: (s) => renderCalls.push(s) });
  });

  it('calls render when name matches', () => {
    const state = { teams: [], console_hull: [], travel_duration_secs: 5 };
    emit({ type: 'console_state', name: 'Repair', json: JSON.stringify(state) });
    expect(renderCalls).toHaveLength(1);
    expect(renderCalls[0]).toEqual(state);
  });

  it('ignores messages for a different console', () => {
    emit({ type: 'console_state', name: 'Helm', json: JSON.stringify({ heading: 0 }) });
    expect(renderCalls).toHaveLength(0);
  });

  it('ignores messages with a different type', () => {
    emit({ type: 'sim_state', name: 'Repair', json: '{}' });
    expect(renderCalls).toHaveLength(0);
  });

  it('ignores null/malformed data', () => {
    emit(null);
    emit({ type: 'console_state' });   // missing name
    expect(renderCalls).toHaveLength(0);
  });
});

// ── Inbound: parse-failure handling ──────────────────────────────────────────

describe('initConsole — JSON parse failure', () => {
  beforeEach(() => {
    global.BroadcastChannel = function() { return makeBCStub().instance; };
  });

  it('does not throw and logs a warn when stateJson is invalid', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const render = vi.fn();
    initConsole({ name: 'Power', render });

    window.__updateConsole('Power', 'not-valid-json');

    expect(render).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Power'),
      expect.anything()
    );
    warnSpy.mockRestore();
  });

  it('calls render when stateJson is valid', () => {
    const render = vi.fn();
    initConsole({ name: 'Power', render });
    window.__updateConsole('Power', JSON.stringify({ locked: false }));
    expect(render).toHaveBeenCalledWith({ locked: false });
  });
});

// ── window.__updateConsole registration ──────────────────────────────────────

describe('initConsole — window.__updateConsole', () => {
  beforeEach(() => {
    global.BroadcastChannel = function() { return makeBCStub().instance; };
  });

  it('registers window.__updateConsole', () => {
    initConsole({ name: 'CaptainChair', render: () => {} });
    expect(typeof window.__updateConsole).toBe('function');
  });

  it('passes the parsed state object to render', () => {
    const render = vi.fn();
    initConsole({ name: 'CaptainChair', render });
    const state = { red_alert: true, view_direction: 'Fore' };
    window.__updateConsole('CaptainChair', JSON.stringify(state));
    expect(render).toHaveBeenCalledWith(state);
  });
});
