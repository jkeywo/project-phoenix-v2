import { expect, it, vi } from 'vitest';
import { createGridLifecycle, createGridPeers } from '../../examples/grid/host-lifecycle.js';

it.each(['disconnect', 'join', 'host'])('cancelled Grid startup stays cancelled after %s', async mode => {
  const lifecycle = createGridLifecycle(), free = vi.fn();
  const operation = lifecycle.begin();
  let resolve; const pending = new Promise(done => { resolve = done; });
  const completion = pending.then(() => { if (operation.current()) operation.own(free); });
  if (mode === 'disconnect') lifecycle.stop(); else lifecycle.begin();
  resolve(); await completion;
  expect(operation.current()).toBe(false); expect(free).not.toHaveBeenCalled();
  operation.own(free); expect(free).toHaveBeenCalledOnce(); lifecycle.stop();
});

it('Grid releases peer, transport and WASM resources once in reverse ownership order', () => {
  const lifecycle = createGridLifecycle(), events = [];
  const operation = lifecycle.begin();
  for (const label of ['WASM', 'peers', 'transport', 'timer']) operation.own(() => events.push(label));
  lifecycle.stop(); lifecycle.stop();
  expect(events).toEqual(['timer', 'transport', 'peers', 'WASM']);
});

it('Grid physical peers use admitted recipients and close the superseded socket', () => {
  const listeners = new Map();
  const peer = () => ({ send: vi.fn(), close: vi.fn(), on(event, callback) { listeners.set(this, { ...listeners.get(this), [event]: callback }); } });
  let next = 0;
  const grid = { open_peer: () => String(++next), close_peer: vi.fn(),
    receive: vi.fn(() => JSON.stringify({ previous: '1', output: { type: 'Recovery' } })), recipients: () => '["2"]' };
  const peers = createGridPeers(grid), first = peer(), second = peer();
  peers.attach(first); peers.attach(second); listeners.get(second).data('{}');
  expect(first.close).toHaveBeenCalledOnce();
  peers.publish('state'); expect(second.send).toHaveBeenCalledWith('state', 'snapshot');
  expect(first.send).not.toHaveBeenCalled();
  peers.close(); expect(second.close).toHaveBeenCalledOnce();
});

it('late Grid close and data events never call a freed WASM host', () => {
  let freed = false; const events = {};
  const grid = {open_peer: () => '1', close_peer:vi.fn(() => {if (freed) throw new Error('freed WASM');}),
    receive:vi.fn(), recipients:vi.fn(() => '[]')};
  const peer = {on:(event,handler) => {events[event] = handler;}, close:vi.fn(), send:vi.fn()};
  const peers = createGridPeers(grid); peers.attach(peer); peers.close(); freed = true;
  expect(() => { events.close(); events.data('{}'); peers.close(); peers.publish('state'); }).not.toThrow();
  expect(grid.close_peer).toHaveBeenCalledExactlyOnceWith('1'); expect(grid.receive).not.toHaveBeenCalled();
});
