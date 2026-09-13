import { MessageChannel } from 'node:worker_threads';
import { describe, expect, it, vi } from 'vitest';
import { attachWorkshopTestChild, installWorkshopTestChild } from '../workshop-test-child.js';
import { createWorkshopTestPort } from '../workshop-test-frame.js';

describe('disposable Test child port', () => {
  it('boots once and only accepts the finite clock controls after that immutable source', async () => {
    const { port1, port2 } = new MessageChannel();
    let tick = 0;
    const runtime = { status: vi.fn(async () => ({ running: true, tick })), control: vi.fn(async () => ({ running: true, tick: ++tick })), dispose: vi.fn() };
    const launch = vi.fn(async () => runtime);
    const child = attachWorkshopTestChild({ port: port2, launch });
    const parent = createWorkshopTestPort({ port: port1 });
    const snapshot = { files: { 'assets/worlds/test.toml': '[global]' }, selection: { seed: 4 }, revision: 'capture' };
    try {
      expect(await parent.request({ operation: 'start', snapshot })).toEqual({ running: true, tick: 0 });
      expect(launch).toHaveBeenCalledExactlyOnceWith(snapshot, { signal: expect.any(AbortSignal) });
      await expect(parent.request({ operation: 'start', snapshot })).rejects.toThrow('Invalid');
      await expect(parent.request({ operation: 'control', control: { command: 'step', path: 'private-file' } })).rejects.toThrow('Invalid');
      await expect(parent.request({ operation: 'read', path: 'private-file' })).rejects.toThrow('Invalid');
      await expect(parent.request({ operation: 'control', control: { command: 'rate', multiplier: 255 } })).rejects.toThrow('Invalid');
      expect(runtime.control).not.toHaveBeenCalled();
      expect(await parent.request({ operation: 'control', control: { command: 'step' } })).toEqual({ running: true, tick: 1 });
      expect(await parent.request({ operation: 'status' })).toEqual({ running: true, tick: 1 });
    } finally { child.dispose(); parent.close(); }
    expect(runtime.dispose).toHaveBeenCalledOnce();
  });

  it('retires an accepted asynchronous launch when its document goes away', async () => {
    const { port1, port2 } = new MessageChannel();
    let finish, launched;
    const booting = new Promise(resolve => { launched = resolve; });
    const runtime = { dispose: vi.fn(), status: vi.fn() };
    const child = attachWorkshopTestChild({ port: port2, launch: () => {
      launched(); return new Promise(resolve => { finish = resolve; });
    } });
    port1.postMessage({ id: 1, operation: 'start', snapshot: {} });
    await booting;
    child.dispose(); finish(runtime);
    await Promise.resolve(); await Promise.resolve();
    expect(runtime.dispose).toHaveBeenCalledOnce();
    expect(runtime.status).not.toHaveBeenCalled();
    port1.close();
  });

  it('accepts only the owning same-origin parent and removes the global listener after one connection', () => {
    const listeners = new Map();
    const win = { parent: {}, location: { origin: 'http://localhost:8080' },
      addEventListener: (type, fn) => listeners.set(type, fn), removeEventListener: type => listeners.delete(type) };
    const owner = installWorkshopTestChild({ win, launch: vi.fn() });
    const { port1, port2 } = new MessageChannel();
    const value = { source: win.parent, origin: win.location.origin, data: { type: 'phoenix-workshop-test-connect' }, ports: [port2] };
    const connect = listeners.get('message');
    connect({ ...value, source: {} }); expect(listeners.has('message')).toBe(true);
    connect({ ...value, origin: 'https://other.example' }); expect(listeners.has('message')).toBe(true);
    connect(value); expect(listeners.has('message')).toBe(false);
    owner.dispose(); port1.close();
  });
});
