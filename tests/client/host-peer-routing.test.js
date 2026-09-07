import { describe, it, expect, vi } from 'vitest';
import { createHostConnections, hostConnectionRegistryReady } from '../../gui/host-peer-routing.js';

/** Scripted registry decisions: these tests cover physical event/readiness
 * application only. The shared transcript runs the REAL Rust owner on native
 * and in the rebuilt-WASM smoke; no JS policy imitation is used here. */
function registryStub(handles = ['first', 'second']) {
  return {
    open: vi.fn(() => handles.shift()),
    bind: vi.fn(() => JSON.stringify({ ok: true, previous: null })),
    sender: vi.fn(() => null),
    close: vi.fn(() => null),
    recipients: vi.fn(() => '[]'),
  };
}

function connection({ open = true, snapshotChannel, delayedClose = false } = {}) {
  const events = new Map();
  const conn = {
    peer: 'same-physical-peer-id',
    open, snapshotChannel,
    send: vi.fn(),
    on(type, fn) {
      const handlers = events.get(type) || [];
      handlers.push(fn);
      events.set(type, handlers);
    },
    emit(type, value) { for (const fn of events.get(type) || []) fn(value); },
    close: vi.fn(() => { if (!delayedClose) { conn.open = false; conn.emit('close'); } }),
  };
  return conn;
}

describe('physical host connection adapter', () => {
  it('applies owner replacement before synchronous old close, with no stale departure', () => {
    const registry = registryStub();
    const onMessage = vi.fn(), onDeparture = vi.fn();
    const host = createHostConnections(registry, { onMessage, onDeparture });
    const old = connection(), current = connection();
    host.attach(old);
    host.attach(current);
    registry.bind.mockImplementation(() => {
      registry.sender.mockImplementation(id => id === 'second' ? 'A' : null);
      registry.recipients.mockReturnValue('["second"]');
      return JSON.stringify({ ok: true, previous: 'first' });
    });
    current.emit('data', { type: 'Identify', data: { token: 'A' } });
    expect(old.close).toHaveBeenCalledOnce();
    expect(registry.close).toHaveBeenCalledWith('first');
    expect(onDeparture).not.toHaveBeenCalled();
    expect(host.targets('all', 'reliable')).toEqual([current]);
    old.emit('data', { type: 'ReleaseStation' });
    old.emit('close');
    expect(onMessage).toHaveBeenCalledExactlyOnceWith('A', JSON.stringify({ type: 'Identify', data: { token: 'A' } }), 'second');
  });

  it('refuses through the Rust verdict, forgets immediately, and cannot rename on a live link', () => {
    const registry = registryStub();
    const onMessage = vi.fn(), onDeparture = vi.fn();
    const host = createHostConnections(registry, { onMessage, onDeparture });
    const conn = connection({ delayedClose: true });
    host.attach(conn);
    registry.sender.mockReturnValue('A');
    registry.bind.mockReturnValue('{"ok":false,"code":"invalid-token"}');
    registry.close.mockImplementation(() => { registry.sender.mockReturnValue(null); return 'A'; });
    conn.emit('data', { type: 'Identify', data: { token: 'B' } });
    expect(registry.bind).toHaveBeenCalledWith('first', 'B');
    expect(JSON.parse(conn.send.mock.calls[0][0])).toEqual({ type: 'JoinRefused', data: { code: 'invalid-token' } });
    expect(onDeparture).toHaveBeenCalledExactlyOnceWith('A');
    conn.emit('data', { type: 'SetReady', data: { ready: true } });
    conn.emit('close');
    expect(onMessage).not.toHaveBeenCalled();
    expect(onDeparture).toHaveBeenCalledTimes(1);
  });

  it('gates pre-Identify picks, bad JSON and invalid shapes before host callbacks', () => {
    const registry = registryStub();
    const onMessage = vi.fn(), onIdentified = vi.fn();
    const host = createHostConnections(registry, { onMessage, onIdentified });
    const conn = connection();
    host.attach(conn);
    for (const raw of ['garbage', 'null', { type: 'SelectScenario', data: { scenario_id: 'early' } }]) conn.emit('data', raw);
    expect(onMessage).not.toHaveBeenCalled();
    expect(onIdentified).not.toHaveBeenCalled();
    registry.bind.mockReturnValue('{"ok":false,"code":"invalid-token"}');
    conn.emit('data', { type: 'Identify', data: { token: { bad: true } } });
    expect(registry.bind).toHaveBeenCalledWith('first', '');
    expect(onMessage).not.toHaveBeenCalled();
  });

  it('uses only Rust recipients and falls back independently to each ready reliable link', () => {
    const registry = registryStub(['a', 'b', 'closed']);
    const host = createHostConnections(registry, { onMessage: vi.fn() });
    const snapshotA = { readyState: 'open' };
    const a = connection({ snapshotChannel: snapshotA });
    const b = connection({ snapshotChannel: { readyState: 'connecting' } });
    const closed = connection({ open: false, snapshotChannel: { readyState: 'open' } });
    host.attach(a); host.attach(b); host.attach(closed);
    registry.recipients.mockReturnValue('["a","b","closed"]');
    expect(host.targets('except:some-token', 'snapshot')).toEqual([snapshotA, b, closed.snapshotChannel]);
    expect(registry.recipients).toHaveBeenLastCalledWith('except:some-token');
    expect(host.targets('all', 'reliable')).toEqual([a, b]);
    registry.recipients.mockReturnValue('["b"]');
    expect(host.targets('token:opaque', 'snapshot')).toEqual([b]);
    b.snapshotChannel = { readyState: 'open' };
    expect(host.targets('token:opaque', 'snapshot')).toEqual([b.snapshotChannel]);
    b.snapshotChannel = null;
    expect(host.targets('token:opaque', 'snapshot')).toEqual([b]);
    registry.recipients.mockReturnValue('[]');
    expect(host.targets('all', 'snapshot')).toEqual([]);
  });
});

describe('registry bootstrap before ECS', () => {
  it('waits for Trunk exports when ICE is ready first, without waiting for PhoenixReady', async () => {
    const hostWindow = new EventTarget();
    let ready = false;
    const pending = hostConnectionRegistryReady(hostWindow).then(value => { ready = true; return value; });
    await Promise.resolve();
    expect(ready).toBe(false);
    class BrowserConnections {}
    hostWindow.wasmBindings = { BrowserConnections };
    hostWindow.dispatchEvent(new Event('TrunkApplicationStarted'));
    expect(await pending).toBeInstanceOf(BrowserConnections);
  });

  it('handles exports arriving before ICE without requiring a second event', async () => {
    class BrowserConnections {}
    const hostWindow = { wasmBindings: { BrowserConnections }, addEventListener: vi.fn() };
    expect(await hostConnectionRegistryReady(hostWindow)).toBeInstanceOf(BrowserConnections);
    expect(hostWindow.addEventListener).not.toHaveBeenCalled();
  });

  it('refuses startup when the built module does not export the owner', async () => {
    await expect(hostConnectionRegistryReady({ wasmBindings: {} })).rejects.toThrow();
  });
});


describe('the shipped pre-ECS message queue', () => {
  it('forgets stale incarnations at the real flush and keeps current reliable order', async () => {
    const { readFileSync } = await import('node:fs');
    const html = readFileSync(new URL('../../server.html', import.meta.url), 'utf8');
    const dispatch = html.match(/function dispatchToWasm\(handle, json\) \{[\s\S]*?\n    \}/)[0];
    const flush = html.match(/for \(const \[handle, json\] of msgQueue\.splice\(0\)\) dispatchToWasm\(handle, json\);/)[0];
    const owners = new Map([['old', 'A']]);
    const host = { sender: handle => owners.get(handle) };
    const bridge = new Function('hostConnections', [
      'let wasm_receive_message;',
      'const msgQueue = [];',
      dispatch,
      'return { dispatch: dispatchToWasm, start(send) { wasm_receive_message = send;',
      flush,
      '} };',
    ].join('\n'))(host);
    bridge.dispatch('old', '{"type":"Identify"}');
    bridge.dispatch('old', '{"type":"SetReady"}');
    owners.delete('old');
    owners.set('new', 'A');
    bridge.dispatch('new', '{"type":"Identify"}');
    bridge.dispatch('new', '{"type":"SelectStation"}');
    const receive = vi.fn();
    bridge.start(receive);
    expect(receive.mock.calls).toEqual([
      ['A', '{"type":"Identify"}'], ['A', '{"type":"SelectStation"}'],
    ]);
  });
});
