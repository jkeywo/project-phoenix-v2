// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, it, expect, vi } from 'vitest';

const host = readFileSync(resolve('server.html'), 'utf8');
const boot = readFileSync(resolve('src/native_host/native_gm/boot.js'), 'utf8')
  .replace(/^import[^\n]+\n/, '');
const queue = readFileSync(resolve('src/native_host/native_gm/queue.js'), 'utf8');

describe('native GM private bridge boot', () => {
  it('bounds reliable actions and faults the batch rather than keeping a suffix', () => {
    new Function(queue)();
    for (let i = 0; i < 256; i += 1) expect(window.phoenixNativeGmOut.send('action')).toBe(true);
    expect(window.phoenixNativeGmOut.send('overflow')).toBe(false);
    expect(JSON.parse(window.__phoenixNativeGmOutDrain())).toEqual([JSON.stringify({kind: 'surface-fault'})]);
    expect(window.phoenixNativeGmOut.send('suffix')).toBe(false);
    new Function(queue)();
    expect(window.phoenixNativeGmOut.send('x'.repeat(128 * 1024 + 1))).toBe(false);
    expect(JSON.parse(window.__phoenixNativeGmOutDrain())).toEqual([JSON.stringify({kind: 'surface-fault'})]);
    new Function(queue)();
    for (let i = 0; i < 4; i += 1) expect(window.phoenixNativeGmOut.send('x'.repeat(128 * 1024))).toBe(true);
    expect(window.phoenixNativeGmOut.send('x')).toBe(false);
  });
  it('uses the real GM workspace and readiness elements without a simulation boot', () => {
    const parsed = new DOMParser().parseFromString(host, 'text/html');
    document.body.innerHTML = parsed.body.innerHTML;
    const send = vi.fn();
    window.phoenixNativeGmOut = { send };
    const mount = vi.fn();
    new Function('mountNativeGmWorkspace', boot)(mount);
    expect(document.documentElement.classList.contains('phoenix-gm-page')).toBe(true);
    expect(document.getElementById('gm-console').contains(document.getElementById('gm-start-controls'))).toBe(true);
    expect(mount).toHaveBeenCalledWith({ bridge: window.phoenixNativeGm, win: window, doc: document });
    expect(send).toHaveBeenCalledWith(JSON.stringify({ kind: 'loaded' }));
    const receive = vi.fn();
    window.phoenixNativeGm.subscribe(receive);
    window.__phoenixNativeGmChannels.metadata(JSON.stringify({ phase: 'InProgress', gms: [
      { id: 'native-gm', name: 'GM', connected: true, ready: false },
    ] }));
    expect(window.phoenixNativeGm.getOperator().id).toBe('native-gm');
    const request = { operator_id: 'native-gm', correlation: 'pause-1', action: 'set_session_paused', active: true };
    expect(window.phoenixNativeGm.submitAction(request)).toBe(true);
    expect(JSON.parse(send.mock.lastCall[0])).toEqual({kind: 'action', request: JSON.stringify(request)});
    window.__phoenixNativeGmChannels.metadata(JSON.stringify({phase: 'InProgress', gms: []}));
    expect(window.phoenixNativeGm.submitAction(request)).toBe(false);
    expect(receive).toHaveBeenCalledTimes(2);
  });
});
