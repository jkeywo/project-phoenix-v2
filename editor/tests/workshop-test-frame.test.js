import { MessageChannel } from 'node:worker_threads';
import { describe, expect, it, vi } from 'vitest';
import { createBrowserWorkshopTest, createWorkshopTestPort, transferableTestSnapshot } from '../workshop-test-frame.js';

const selection = { world: 'assets/worlds/test.toml', ship: 'assets/entities/hull.toml', seed: 7 };
const snapshot = () => ({ selection, revision: 'captured', files: {
  'assets/worlds/test.toml': '# exact\r\n[global]\r\n', 'assets/models/hull.glb': Uint8Array.of(0, 255, 3),
} });
const running = { running: true, paused: false, tick: 0, multiplier: 1 };
function fixture() {
  const frames = [];
  const prepare = vi.fn(async () => snapshot());
  const frame = vi.fn(() => {
    const child = { start: vi.fn(async () => running), status: vi.fn(async () => running),
      control: vi.fn(async () => running), visible: vi.fn(), destroy: vi.fn() };
    frames.push(child); return child;
  });
  return { frames, frame, prepare, test: createBrowserWorkshopTest({ prepare, frame, catalog: vi.fn() }) };
}

describe('browser Test frame isolation', () => {
  it('transfers exact byte copies and only explicit source, selection and revision', () => {
    const original = { ...snapshot(), profile: { private: true }, token: 'live-credential' };
    const capture = transferableTestSnapshot(original);
    const received = structuredClone(capture.snapshot, { transfer: capture.transfer });
    expect(original.files['assets/models/hull.glb']).toEqual(Uint8Array.of(0, 255, 3));
    expect(received).toEqual(snapshot());
    expect(received).not.toHaveProperty('profile');
    expect(received).not.toHaveProperty('token');
    expect(capture.snapshot.files['assets/models/hull.glb'].byteLength).toBe(0);
    for (const path of ['assets/../secret', 'C:/secret', 'assets/models/../../secret']) {
      expect(() => transferableTestSnapshot({ ...snapshot(), files: { [path]: new Uint8Array() } })).toThrow('source path');
    }
    expect(() => transferableTestSnapshot({ ...snapshot(), files: { 'assets/model.glb': { asset: 'private-native-reference' } } })).toThrow('source bytes');
  });

  it('retains the previous run through preparation/boot failures and destroys it only after replacement starts', async () => {
    const { test, prepare, frame, frames } = fixture();
    await test.start({}, selection);
    prepare.mockRejectedValueOnce(new Error('Invalid composed hull'));
    await expect(test.start({}, selection)).rejects.toThrow('Invalid composed hull');
    expect(frames[0].destroy).not.toHaveBeenCalled();
    expect(frame).toHaveBeenCalledTimes(1);
    frame.mockImplementationOnce(() => {
      const child = { ...frames[0], start: vi.fn(async () => { throw new Error('Renderer failed'); }), destroy: vi.fn() };
      frames.push(child); return child;
    });
    await expect(test.start({}, selection)).rejects.toThrow('Renderer failed');
    expect(frames[0].destroy).not.toHaveBeenCalled();
    expect(frames[1].destroy).toHaveBeenCalledOnce();
    await test.start({}, selection);
    expect(frames[0].destroy).toHaveBeenCalledOnce();
    expect(frames[2].visible).toHaveBeenCalledWith(true);
    await test.stop();
    expect(frames[2].destroy).toHaveBeenCalledOnce();
  });

  it('releases a cancelled preparation immediately and never creates its late frame', async () => {
    const { test, prepare, frame } = fixture();
    let finish;
    prepare.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const preparing = test.start({}, selection);
    await test.stop();
    await expect(preparing).rejects.toThrow('cancelled');
    expect(frame).not.toHaveBeenCalled();
    finish(snapshot());
    await Promise.resolve();
    expect(frame).not.toHaveBeenCalled();
  });

  it('returns to closed after an unresponsive child instead of retaining a detached current run', async () => {
    const { test, frames } = fixture();
    await test.start({}, selection);
    frames[0].status.mockRejectedValueOnce(new Error('No response'));
    await expect(test.status()).rejects.toThrow('No response');
    expect(frames[0].destroy).toHaveBeenCalledOnce();
    expect(await test.status()).toEqual({ running: false });
  });

  it('destroys an unresolved hidden boot immediately on Stop without waiting for its startup timeout', async () => {
    const { test, frames, frame } = fixture();
    let finish;
    frame.mockImplementationOnce(() => {
      const child = { start: () => new Promise(resolve => { finish = resolve; }), destroy: vi.fn(), visible: vi.fn() };
      frames.push(child); return child;
    });
    const started = test.start({}, selection);
    await vi.waitFor(() => expect(frame).toHaveBeenCalledOnce());
    await test.stop();
    expect(frames[0].destroy).toHaveBeenCalledOnce();
    finish(running);
    await expect(started).rejects.toThrow('cancelled');
    expect(frames[0].destroy).toHaveBeenCalledOnce();
    expect(frames[0].visible).not.toHaveBeenCalled();
  });

  it('reveals a held iframe before awaiting a control frame from the suspended browser renderer', async () => {
    const { test, frames } = fixture();
    await test.start({}, selection);
    await test.control({ command: 'visibility', visible: false });
    frames[0].visible.mockClear();
    let finish;
    frames[0].control.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const showing = test.control({ command: 'visibility', visible: true });
    expect(frames[0].visible).toHaveBeenCalledWith(true);
    finish(running); await showing; await test.stop();
  });

  it('correlates the private port response and ignores unrelated replies', async () => {
    const { port1, port2 } = new MessageChannel();
    const channel = createWorkshopTestPort({ port: port1 });
    port2.once('message', request => {
      expect(request).toEqual({ id: 1, operation: 'status' });
      port2.postMessage({ id: 999, run: { running: false } });
      port2.postMessage({ id: request.id, run: running });
    });
    try { expect(await channel.request({ operation: 'status' })).toEqual(running); }
    finally { channel.close(); port2.close(); }
  });

  it('retires every pending request and the frame owner when a port times out', async () => {
    const { port1, port2 } = new MessageChannel();
    const callbacks = new Map(); let next = 0;
    const timer = { setTimeout: fn => { callbacks.set(++next, fn); return next; }, clearTimeout: id => callbacks.delete(id) };
    const failed = vi.fn();
    const channel = createWorkshopTestPort({ port: port1, timer, failed });
    const first = channel.request({ operation: 'status' });
    const second = channel.request({ operation: 'control', control: { command: 'step' } });
    const checks = [expect(first).rejects.toThrow('did not respond'), expect(second).rejects.toThrow('did not respond')];
    callbacks.get(1)();
    await Promise.all(checks);
    expect(callbacks.size).toBe(0); expect(failed).toHaveBeenCalledOnce();
    await expect(channel.request({ operation: 'status' })).rejects.toThrow('closed');
    port2.close();
  });
});
