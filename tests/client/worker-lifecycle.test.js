import { expect, it, vi } from 'vitest';
import { RendezvousRegistry } from '../../worker-rendezvous/src/index.js';
import worker, { acquireTurnSource } from '../../worker/src/index.js';
import { fetchIceServers } from '../../gui/connection-manager.js';
it('TURN provider timeout covers hung body acquisition and aborts only that source', async () => {
  vi.useFakeTimers(); let signal;
  const result = acquireTurnSource(value => { signal = value; return new Promise(() => {}); }, 10);
  const assertion = expect(result).rejects.toThrow('timed out');
  await vi.advanceTimersByTimeAsync(10); await assertion;
  expect(signal.aborted).toBe(true); vi.useRealTimers();
});
it('browser ICE acquisition completes on hung body with OpenRelay fallback', async () => {
  vi.useFakeTimers(); const original = globalThis.fetch;
  globalThis.fetch = vi.fn(async () => ({ ok: true, json: () => new Promise(() => {}) }));
  try {
    const result = fetchIceServers({ timeoutMs: 10 }); await vi.advanceTimersByTimeAsync(10);
    expect((await result).relaySource).toBe('openrelay');
    expect(globalThis.fetch.mock.calls[0][1].signal.aborted).toBe(true);
  } finally { globalThis.fetch = original; vi.useRealTimers(); }
});
it('worker retains successful provider results when the other provider hangs', async () => {
  vi.useFakeTimers(); const original = globalThis.fetch;
  globalThis.fetch = vi.fn(url => url.includes('metered.live') ? Promise.resolve({ok:true, json:async () => [{urls:'turn:ok'}]}) : new Promise(() => {}));
  try {
    const result = worker.fetch(new Request('https://credentials/'), { METERED_APP:'test', METERED_KEY:'secret', CF_TURN_KEY_ID:'id', CF_TURN_API_TOKEN:'secret' });
    await vi.advanceTimersByTimeAsync(5000); const response = await result;
    expect(response.status).toBe(200); expect(await response.json()).toEqual([{urls:'turn:ok'}]);
    expect(response.headers.get('X-Turn-Source-Errors')).toBe('TURN provider timed out');
  } finally { globalThis.fetch = original; vi.useRealTimers(); }
});
it('rendezvous dispatch retires send-throw sockets immediately and keeps dispatching', () => {
  const adapter = new RendezvousRegistry({}, {});
  adapter.registry = { setWritable:vi.fn(), disconnect:vi.fn(() => []) };
  const bad = { readyState:1, send:vi.fn(() => { throw new Error('gone'); }), close:vi.fn() };
  const good = { readyState:1, send:vi.fn(), close:vi.fn() };
  adapter.sockets.set('bad',bad); adapter.sockets.set('good',good);
  adapter.dispatch([{to:'bad',frame:{type:'ready'}},{to:'good',frame:{type:'ready'}}]);
  expect(adapter.sockets.has('bad')).toBe(false); expect(adapter.registry.disconnect).toHaveBeenCalledExactlyOnceWith('bad');
  expect(good.send).toHaveBeenCalledOnce(); adapter.drop('bad'); expect(adapter.registry.disconnect).toHaveBeenCalledOnce();
});
it('rendezvous refusal reaches the socket before its terminal retirement', () => {
  const adapter = new RendezvousRegistry({}, {}), order = [];
  adapter.registry = { setWritable:vi.fn(), disconnect:vi.fn(() => {order.push('drop'); return [];}) };
  adapter.sockets.set('peer',{readyState:1,send:()=>order.push('send'),close:()=>order.push('close')});
  adapter.dispatch([{to:'peer',frame:{type:'refused'},close:true}]);
  expect(order).toEqual(['send','drop','close']);
});

it('accepted Worker socket wiring decodes messages and drops duplicate terminal events once', async () => {
  const originalPair = globalThis.WebSocketPair, OriginalResponse = globalThis.Response;
  let server;
  class Socket extends EventTarget {
    readyState = 1; sent = []; accept = vi.fn();
    send(raw) { this.sent.push(JSON.parse(raw)); }
    close() { this.readyState = 3; this.dispatchEvent(new Event('close')); }
  }
  globalThis.WebSocketPair = class { constructor() { this[0] = new Socket(); this[1] = server = new Socket(); } };
  globalThis.Response = class extends OriginalResponse {
    constructor(body, options) { if (options?.status === 101) return { status:101, webSocket:options.webSocket }; super(body,options); }
  };
  try {
    const adapter = new RendezvousRegistry({}, {});
    const disconnect = vi.spyOn(adapter.registry,'disconnect');
    const response = await adapter.fetch(new Request('https://rendezvous/v1/join'));
    expect(response.status).toBe(101); expect(server.accept).toHaveBeenCalledOnce();
    expect(server.sent[0].type).toBe('ready');
    server.dispatchEvent(new MessageEvent('message',{data:'not-json'}));
    expect(server.sent.at(-1).type).toBe('error');
    server.dispatchEvent(new Event('error')); server.dispatchEvent(new Event('close'));
    expect(adapter.sockets.size).toBe(0); expect(disconnect).toHaveBeenCalledOnce();
    const count = server.sent.length;
    server.dispatchEvent(new MessageEvent('message',{data:'{}'}));
    expect(server.sent).toHaveLength(count);
  } finally { globalThis.WebSocketPair = originalPair; globalThis.Response = OriginalResponse; }
});

it.each(['success','refusal'])('browser credential deadline is cleared after %s', async mode => {
  vi.useFakeTimers(); const original = globalThis.fetch;
  globalThis.fetch = vi.fn(async () => ({ok:mode === 'success',status:502,json:async () => [{urls:'turn:ok'}]}));
  try {
    await fetchIceServers({timeoutMs:10}); expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(20);
    expect(globalThis.fetch.mock.calls[0][1].signal.aborted).toBe(false);
  } finally {globalThis.fetch = original; vi.useRealTimers();}
});
