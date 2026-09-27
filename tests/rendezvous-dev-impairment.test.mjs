// Real loopback sockets exercise the local adapter around the shipped registry.
import { afterEach, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import net from 'node:net';
import { once } from 'node:events';
import { RENDEZVOUS_PROTOCOL } from '../gui/rendezvous-protocol.js';

const services = [];

async function freePort() {
  const probe = net.createServer();
  probe.listen(0, '127.0.0.1');
  await once(probe, 'listening');
  const port = probe.address().port;
  probe.close();
  await once(probe, 'close');
  return port;
}

async function service(flags = []) {
  const port = await freePort();
  const child = spawn(process.execPath, ['scripts/rendezvous-dev-server.mjs', '--port', String(port), ...flags], {
    cwd: process.cwd(), stdio: ['ignore', 'pipe', 'pipe'],
  });
  services.push(child);
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 60; i += 1) {
    if (child.exitCode !== null) throw new Error(`server exited ${child.exitCode}`);
    try {
      const response = await fetch(`${base}/v1/health`);
      if (response.ok) return base;
    } catch { /* listening shortly */ }
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  throw new Error('rendezvous dev server did not listen');
}

function peer(base, role) {
  const ws = new WebSocket(base.replace('http:', 'ws:') + `/v1/${role}`);
  const inbox = [];
  ws.addEventListener('message', event => inbox.push(JSON.parse(event.data)));
  return {
    ws, inbox,
    async open() { if (ws.readyState !== WebSocket.OPEN) await once(ws, 'open'); },
    send(type, fields = {}) { ws.send(JSON.stringify({ v: RENDEZVOUS_PROTOCOL, type, ...fields })); },
    async next(type, after = 0) {
      for (let i = 0; i < 150; i += 1) {
        const item = inbox.slice(after).find(frame => frame.type === type);
        if (item) return item;
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      throw new Error(`no ${type}: ${JSON.stringify(inbox)}`);
    },
  };
}

async function joinedPair(base) {
  const host = peer(base, 'host');
  await host.open();
  host.send('host-open', { namespace: 'server', transports: ['ws-relay'] });
  const hosted = await host.next('hosted');
  const client = peer(base, 'join');
  await client.open();
  client.send('join', { code: hosted.code.full, namespace: 'server' });
  await client.next('joined');
  client.send('relay-open');
  await client.next('relay-ready');
  await host.next('relay-peer');
  return { host, client };
}

async function metrics(base) {
  return (await fetch(`${base}/__impairment`)).json();
}

afterEach(async () => {
  for (const child of services.splice(0)) {
    if (child.exitCode === null) {
      child.kill();
      await once(child, 'exit');
    }
  }
});

describe('local rendezvous impairment', () => {
  it('delays reliable relay frames, drops snapshot frames, and reports actual boundary counts', async () => {
    const base = await service(['--delay-ms', '150', '--loss-percent', '100', '--seed', '7']);
    const { host, client } = await joinedPair(base);
    const before = host.inbox.length;
    client.send('relay', { class: 'reliable', payload: 'first' });
    client.send('relay', { class: 'reliable', payload: 'second' });
    client.send('relay', { class: 'snapshot', payload: 'stale' });
    let queued;
    for (let i = 0; i < 50; i += 1) {
      queued = await metrics(base);
      if (queued.counters.relay_snapshot_seen === 1) break;
      await new Promise(resolve => setTimeout(resolve, 5));
    }
    assert.equal(queued.profile.delay_ms, 150);
    assert.equal(queued.profile.loss_percent, 100);
    assert.equal(queued.profile.seed, 7);
    assert.equal(queued.counters.relay_reliable_seen, 2);
    assert.equal(queued.counters.relay_snapshot_seen, 1);
    assert.equal(queued.counters.relay_snapshot_dropped, 1);
    assert.equal(queued.counters.relay_reliable_written, 0);
    assert.equal(queued.counters.pending, 2);
    assert.equal(queued.counters.relay_frames_delayed, 2);
    await host.next('relay', before);
    const relay = host.inbox.slice(before).filter(frame => frame.type === 'relay');
    await host.next('relay', before + 1);
    assert.deepEqual(host.inbox.slice(before).filter(frame => frame.type === 'relay').map(frame => frame.payload),
      ['first', 'second']);
    assert.equal(relay[0].class, 'reliable');
    const final = (await metrics(base)).counters;
    assert.equal(final.relay_reliable_written, 2);
    assert.equal(final.relay_snapshot_written, 0);
    assert.equal(final.pending, 0);
    host.ws.close(); client.ws.close();
  });

  it('suppresses only RTC offers while join and WebSocket relay remain usable', async () => {
    const base = await service(['--block-rtc-offers']);
    const { host, client } = await joinedPair(base);
    const before = host.inbox.length;
    client.send('signal', { payload: { sdp: { type: 'offer', sdp: 'fake' } } });
    client.send('relay', { class: 'reliable', payload: 'after-offer' });
    await host.next('relay', before);
    assert.equal(host.inbox.slice(before).some(frame => frame.type === 'signal'), false);
    const final = (await metrics(base)).counters;
    assert.equal(final.signal_offers_dropped, 1);
    assert.equal(final.relay_reliable_written, 1);
    host.ws.close(); client.ws.close();
  });

  it('replays snapshot loss decisions for the same seed and arrival order', async () => {
    async function sample() {
      const base = await service(['--loss-percent', '50', '--seed', '37']);
      const { host, client } = await joinedPair(base);
      client.send('relay', { class: 'reliable', payload: 'must-arrive' });
      for (let i = 0; i < 16; i += 1) {
        client.send('relay', { class: 'snapshot', payload: String(i) });
      }
      let counts;
      for (let i = 0; i < 150; i += 1) {
        counts = (await metrics(base)).counters;
        if (counts.relay_snapshot_seen === 16
            && host.inbox.filter(frame => frame.type === 'relay').length
              === counts.relay_snapshot_written + counts.relay_reliable_written) break;
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      assert.equal(counts.relay_snapshot_seen, 16);
      assert.equal(counts.relay_reliable_written, 1);
      assert.equal(counts.relay_snapshot_written + counts.relay_snapshot_dropped, 16);
      const received = host.inbox.filter(frame => frame.type === 'relay').map(frame => frame.payload);
      host.ws.close(); client.ws.close();
      return { received, dropped: counts.relay_snapshot_dropped };
    }
    const first = await sample();
    const second = await sample();
    assert.deepEqual(second, first);
    assert(first.dropped > 0 && first.dropped < 16);
    assert.equal(first.received[0], 'must-arrive');
  });

  it('rejects out-of-range profiles at startup', async () => {
    const child = spawn(process.execPath,
      ['scripts/rendezvous-dev-server.mjs', '--loss-percent', '101'],
      { cwd: process.cwd(), stdio: 'ignore' });
    const [code] = await once(child, 'exit');
    assert.notEqual(code, 0);
  });
});
