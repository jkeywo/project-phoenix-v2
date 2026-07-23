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

  it('routes via window.__sendAction on the WASM host page (issue #822)', () => {
    // server.html defines __sendAction (dispatching through gui/action-map.js)
    // alongside wasmBindings; the ladder must hand the raw envelope to it.
    const sendActionFn = vi.fn();
    setup({ wasmBindings: { wasm_receive_message: vi.fn() }, __sendAction: sendActionFn });
    sendAction('set_red_alert', {});
    const expected = JSON.stringify({ action: 'set_red_alert', console: 'Test' });
    expect(sendActionFn).toHaveBeenCalledWith(expected);
  });

  it('falls through to BroadcastChannel when wasmBindings exist without __sendAction', () => {
    const { bc } = setup({ wasmBindings: { wasm_receive_message: 'not-a-function' } });
    sendAction('set_red_alert', {});
    // Falls through to BroadcastChannel
    expect(bc.instance.postMessage).toHaveBeenCalled();
  });

  it('calls window.__sendAction when present (host page / test transport)', () => {
    const sendActionFn = vi.fn();
    setup({ __sendAction: sendActionFn });
    sendAction('set_red_alert', {});
    const expected = JSON.stringify({ action: 'set_red_alert', console: 'Test' });
    expect(sendActionFn).toHaveBeenCalledWith(expected);
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
    const state = { teams: [], system_hull: [], travel_duration_secs: 5 };
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

// ── Inbound: BroadcastChannel listener gated by context (#482) ───────────────
//
// When the console runs inside a parent iframe / wry host / browser-WASM,
// it must NOT listen on the BroadcastChannel — a direct caller already
// owns __updateConsole, and turning on the BC listener creates a SECOND
// state source that races with the direct push. Helm regression: in
// same-origin same-browser play, server.html broadcast minimal
// HelmConsoleState (no `blips`) and client.html iframe-pushed the full
// state-with-blips; the iframe received both, alternating, producing
// per-tick canvas flicker as blips disappeared and reappeared.

describe('initConsole — BroadcastChannel inbound context gating (#482)', () => {
  let bcConstructed;

  function withBC(setup, runTest) {
    bcConstructed = 0;
    const stub = makeBCStub();
    const savedBC = global.BroadcastChannel;
    global.BroadcastChannel = function() {
      bcConstructed++;
      return stub.instance;
    };
    try {
      setup();
      runTest(stub);
    } finally {
      global.BroadcastChannel = savedBC;
    }
  }

  it('attaches the BC listener in baseline / separate-tab mode', () => {
    withBC(
      () => { /* baseline: no parent, no ipc, no wasmBindings */ },
      (stub) => {
        const render = vi.fn();
        initConsole({ name: 'Helm', render });
        expect(bcConstructed).toBe(1);
        expect(typeof stub.instance.onmessage).toBe('function');
      },
    );
  });

  it('does NOT attach the BC listener when running in an iframe (window !== parent)', () => {
    withBC(
      () => {
        Object.defineProperty(global.window, 'parent', {
          value: { postMessage: vi.fn() },
          configurable: true,
        });
      },
      () => {
        const render = vi.fn();
        initConsole({ name: 'Helm', render });
        expect(bcConstructed).toBe(0);
      },
    );
  });

  it('does NOT attach the BC listener when window.ipc is present (wry mode)', () => {
    withBC(
      () => { global.window.ipc = { postMessage: vi.fn() }; },
      () => {
        const render = vi.fn();
        initConsole({ name: 'Helm', render });
        expect(bcConstructed).toBe(0);
      },
    );
  });

  it('does NOT attach the BC listener when wasmBindings.wasm_receive_message is present (WASM mode)', () => {
    withBC(
      () => { global.window.wasmBindings = { wasm_receive_message: vi.fn() }; },
      () => {
        const render = vi.fn();
        initConsole({ name: 'Helm', render });
        expect(bcConstructed).toBe(0);
      },
    );
  });

  it('iframe-mode console ignores cross-origin BC pushes for its own name', () => {
    // Regression for #482: even if a BC message for this console fires
    // somehow, the inbound listener must not be attached, so no second
    // render path can interfere with the parent-direct push.
    withBC(
      () => {
        Object.defineProperty(global.window, 'parent', {
          value: { postMessage: vi.fn() },
          configurable: true,
        });
      },
      () => {
        const render = vi.fn();
        initConsole({ name: 'Helm', render });
        // Parent-direct push is still wired and works:
        window.__updateConsole('Helm', JSON.stringify({ heading: 42 }));
        expect(render).toHaveBeenCalledTimes(1);
        expect(render).toHaveBeenCalledWith({ heading: 42 });
        // BC was not constructed, so no second receive path exists.
        expect(bcConstructed).toBe(0);
      },
    );
  });
});
