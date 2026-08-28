// The transport stand-in the whole smoke suite runs on (issue #1112, replacing
// the #52 PeerJS-shim tests).
//
// Two blank pages on the served origin drive `window.PhoenixTransportFactories`
// directly — no production WASM, no client bundle — and assert the behaviours
// every other spec silently depends on: one page owns the real
// worker-rendezvous registry and is issued a code, the other resolves that code
// and reaches it, signalling relays both ways, DataChannels carry data both
// ways with the lossy one honestly unordered, close is observed on the far end,
// and sever/revive holds a page off the network until the test says otherwise.
//
// This is a test OF the fake, deliberately. A fake that quietly stops relaying,
// or opens a "snapshot" channel that is really ordered and retransmitting,
// would leave forty specs green while proving something the product does not do.

import { test, expect, SMOKE_ORIGIN } from './fixtures';

// Blank pages must be on the served origin: BroadcastChannel is same-origin, and
// null-origin pages cannot reach each other.
async function blankPage(ctx, slug) {
  const page = await ctx.newPage();
  await page.route(`**/${slug}`, (r) =>
    r.fulfill({ contentType: 'text/html', body: '<!DOCTYPE html><html><body></body></html>' }),
  );
  await page.goto(`${SMOKE_ORIGIN}/${slug}`);
  return page;
}

/** Register `page` as a rendezvous host and resolve to its issued code. */
function hostOn(page) {
  return page.evaluate(() =>
    new Promise((resolve, reject) => {
      const socket = window.PhoenixTransportFactories.socket('https://r.test/v1/host');
      window.__hostSocket = socket;
      window.__peers = [];
      window.__received = [];
      socket.onmessage = (e) => {
        const msg = JSON.parse(e.data);
        if (msg.type === 'ready') socket.send(JSON.stringify({ v: 1, type: 'host-open' }));
        else if (msg.type === 'hosted') resolve(msg.code);
        else if (msg.type === 'peer-joined') window.__peers.push(msg.peer);
        else if (msg.type === 'signal') window.__onHostSignal(msg);
      };
      setTimeout(() => reject(new Error('no code issued')), 10_000);
    }),
  );
}

/** Answer offers on the host page, capturing whatever arrives on each channel. */
function acceptOn(page) {
  return page.evaluate(() => {
    window.__hostChannels = {};
    window.__onHostSignal = async (msg) => {
      if (!msg.payload?.sdp) return;
      const pc = window.PhoenixTransportFactories.peer({ iceServers: [] });
      window.__hostPc = pc;
      pc.ondatachannel = (e) => {
        window.__hostChannels[e.channel.label] = e.channel;
        e.channel.onmessage = (ev) => window.__received.push([e.channel.label, ev.data]);
        e.channel.onclose = () => window.__received.push([e.channel.label, '__closed__']);
      };
      await pc.setRemoteDescription(msg.payload.sdp);
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      window.__hostSocket.send(JSON.stringify({
        v: 1, type: 'signal', to: msg.from, payload: { sdp: pc.localDescription },
      }));
    };
  });
}

/** Join `code` from a client page and open both channels. */
function joinOn(page, code) {
  return page.evaluate((code) =>
    new Promise((resolve, reject) => {
      const factories = window.PhoenixTransportFactories;
      const socket = factories.socket('https://r.test/v1/join');
      window.__clientSocket = socket;
      window.__received = [];
      let pc = null;
      socket.onmessage = async (e) => {
        const msg = JSON.parse(e.data);
        if (msg.type === 'ready') {
          socket.send(JSON.stringify({ v: 1, type: 'join', code }));
        } else if (msg.type === 'joined') {
          pc = factories.peer({ iceServers: [] });
          window.__clientPc = pc;
          const reliable = pc.createDataChannel('reliable', { ordered: true });
          const lossy = pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
          window.__channels = { reliable, lossy };
          for (const [name, ch] of Object.entries(window.__channels)) {
            ch.onmessage = (ev) => window.__received.push([name, ev.data]);
            ch.onclose = () => window.__received.push([name, '__closed__']);
          }
          reliable.onopen = () => resolve('open');
          const offer = await pc.createOffer();
          await pc.setLocalDescription(offer);
          socket.send(JSON.stringify({ v: 1, type: 'signal', payload: { sdp: pc.localDescription } }));
        } else if (msg.type === 'signal' && msg.payload?.sdp) {
          await pc.setRemoteDescription(msg.payload.sdp);
        } else if (msg.type === 'error' || msg.type === 'closed') {
          reject(new Error(`join refused: ${msg.reason}`));
        }
      };
      setTimeout(() => reject(new Error('never linked')), 10_000);
    }), code);
}

async function linkedPair(context) {
  const hostPage = await blankPage(context, 'shim-host');
  const clientPage = await blankPage(context, 'shim-client');
  const code = await hostOn(hostPage);
  await acceptOn(hostPage);
  await joinOn(clientPage, code.full);
  return { hostPage, clientPage, code };
}

test.describe('phoenix transport shim', () => {
  test('the host is issued a five-letter code in the client namespace', async ({ context }) => {
    const hostPage = await blankPage(context, 'shim-code');
    const code = await hostOn(hostPage);
    expect(code.suffix).toMatch(/^[A-Z]{5}$/);
    expect(code.namespace).toBe('client');
    expect(code.full).toContain(code.suffix);
  });

  test('an unknown code is refused rather than silently hanging', async ({ context }) => {
    const hostPage = await blankPage(context, 'shim-unknown-host');
    await hostOn(hostPage);
    const clientPage = await blankPage(context, 'shim-unknown-client');
    const failure = await joinOn(clientPage, 'ZZZZZ').catch((e) => e.message);
    expect(failure).toContain('unknown');
  });

  test('resolving the code links both ends over the signalling relay', async ({ context }) => {
    const { hostPage } = await linkedPair(context);
    // The host really saw the joiner arrive — presence crossed the registry,
    // not just the media bus.
    expect(await hostPage.evaluate(() => window.__peers.length)).toBe(1);
    expect(await hostPage.evaluate(() => Object.keys(window.__hostChannels).sort()))
      .toEqual(['reliable', 'snapshot']);
  });

  test('the lossy channel negotiates unordered and non-retransmitting on both ends', async ({ context }) => {
    const { hostPage, clientPage } = await linkedPair(context);
    const describe = (page) => page.evaluate(() => {
      const out = {};
      for (const [key, c] of Object.entries(window.__transportShim.dataChannels())) {
        out[key.split('@')[0]] = { ordered: c.ordered, maxRetransmits: c.maxRetransmits };
      }
      return out;
    });
    for (const page of [hostPage, clientPage]) {
      const channels = await describe(page);
      expect(channels.reliable).toEqual({ ordered: true, maxRetransmits: null });
      expect(channels.snapshot).toEqual({ ordered: false, maxRetransmits: 0 });
    }
  });

  test('data crosses in both directions, on the channel it was sent on', async ({ context }) => {
    const { hostPage, clientPage } = await linkedPair(context);

    await clientPage.evaluate(() => {
      window.__channels.reliable.send('up-reliable');
      window.__channels.lossy.send('up-lossy');
    });
    await hostPage.waitForFunction(() => window.__received.length === 2, { timeout: 5_000 });
    expect(await hostPage.evaluate(() => window.__received)).toEqual([
      ['reliable', 'up-reliable'],
      ['snapshot', 'up-lossy'],
    ]);

    await hostPage.evaluate(() => {
      window.__hostChannels.reliable.send('down-reliable');
      window.__hostChannels.snapshot.send('down-lossy');
    });
    await clientPage.waitForFunction(() => window.__received.length === 2, { timeout: 5_000 });
    expect(await clientPage.evaluate(() => window.__received)).toEqual([
      ['reliable', 'down-reliable'],
      ['lossy', 'down-lossy'],
    ]);
  });

  test('closing a channel is observed on the far end', async ({ context }) => {
    const { hostPage, clientPage } = await linkedPair(context);
    await clientPage.evaluate(() => window.__channels.reliable.close());
    await hostPage.waitForFunction(
      () => window.__received.some(([label, d]) => label === 'reliable' && d === '__closed__'),
      { timeout: 5_000 },
    );
  });

  test('sever drops the link on both ends and keeps retries failing until revive', async ({ context }) => {
    const { hostPage, clientPage } = await linkedPair(context);

    // Neither side called close(): this is the phone's radio going to sleep.
    await clientPage.evaluate(() => window.__transportShim.sever());
    await hostPage.waitForFunction(
      () => window.__received.some(([, d]) => d === '__closed__'),
      { timeout: 5_000 },
    );

    // A severed page cannot reach the service at all, so an automatic retry
    // cannot race ahead of an assertion the test has not made yet.
    const whileSevered = await clientPage.evaluate(() =>
      new Promise((resolve) => {
        const s = window.PhoenixTransportFactories.socket('https://r.test/v1/join');
        s.onerror = () => resolve('error');
        s.onopen = () => resolve('open');
        setTimeout(() => resolve('silent'), 1_000);
      }),
    );
    expect(whileSevered).toBe('error');

    // Revive, and a fresh attempt gets through — the same path a "retry now"
    // tap takes in the product.
    await clientPage.evaluate(() => window.__transportShim.revive());
    const afterRevive = await clientPage.evaluate(() =>
      new Promise((resolve) => {
        const s = window.PhoenixTransportFactories.socket('https://r.test/v1/join');
        s.onopen = () => resolve('open');
        s.onerror = () => resolve('error');
        setTimeout(() => resolve('silent'), 2_000);
      }),
    );
    expect(afterRevive).toBe('open');
  });
});
