// The same transcript enters actual native RelayTransport adapters and this
// browser adapter backed by real WASM. Fake only physical links/events here;
// the live reconnect spec also crosses rendezvous and authoritative Session.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { test, expect, readHostPeerId, createTestClient, WASM_READY_TIMEOUT } from './fixtures';

const fixture = JSON.parse(readFileSync(path.join(__dirname, '../fixtures/session-connections.json'), 'utf8'));

test('the shared owner and crew admission run before a World or ECS App exists', async ({ context }) => {
  const page = await context.newPage();
  await page.goto('/');
  await page.waitForFunction(() => typeof window.wasmBindings?.BrowserConnections === 'function', null, { timeout: WASM_READY_TIMEOUT });
  const code = await readHostPeerId(page);
  const early = await createTestClient(context, code, { token: 'pre-world-registry', waitFor: 'ScenarioCatalog' });
  expect(await page.evaluate(() => !!window.__wasmReady)).toBe(false);
  expect((await early.waitForMessage('ScenarioCatalog')).data.scenarios.length).toBeGreaterThan(0);

  const actual = await page.evaluate(async fixture => {
    const { createHostConnections } = await import('/gui/host-peer-routing.js');
    const runs = [];
    for (const delayedClose of [false, true]) {
      const registry = new window.wasmBindings.BrowserConnections();
      let received = [], departures = [];
      const physical = new Map();
      const host = createHostConnections(registry, {
        onMessage: (token, json) => received.push({ token, message: JSON.parse(json) }),
        onDeparture: token => departures.push(token),
      });
      const outcomes = [];
      for (const step of fixture.steps) {
        received = []; departures = [];
        for (const conn of physical.values()) conn.sent = [];
        if (step.op === 'open') {
          const handlers = new Map();
          const conn = {
            // Reusing peer ids must not alias physical incarnations.
            peer: 'reused-peer-id', open: true, sent: [],
            snapshotChannel: { readyState: 'open', id: step.id },
            id: step.id,
            send(json) { this.sent.push(JSON.parse(json)); },
            on(type, fn) {
              const list = handlers.get(type) || [];
              list.push(fn); handlers.set(type, list);
            },
            emit(type, value) { for (const fn of handlers.get(type) || []) fn(value); },
            close() { if (!delayedClose) { this.open = false; this.emit('close'); } },
          };
          physical.set(step.id, conn);
          host.attach(conn);
        } else if (step.op === 'message') {
          physical.get(step.id).emit('data', step.message);
        } else if (step.op === 'close') {
          physical.get(step.id).open = false;
          physical.get(step.id).emit('close');
        }
        const channels = step.op === 'send'
          ? ['reliable', 'snapshot'].map(delivery => ({
            delivery,
            recipients: host.targets(step.target, delivery).map(conn => conn.id).sort(),
          })) : [];
        const refusals = [...physical.values()].flatMap(conn => conn.sent)
          .filter(message => message.type === 'JoinRefused').map(message => message.data.code);
        outcomes.push({ received, departures, refusals, channels });
      }
      registry.free();
      runs.push(outcomes);
    }
    return runs;
  }, fixture);
  const expected = fixture.steps.map(step => ({
    received: step.sender == null ? [] : [{ token: step.sender, message: step.message }],
    departures: step.departed || [],
    refusals: step.refusal == null ? [] : [step.refusal],
    channels: step.op === 'send'
      ? ['reliable', 'snapshot'].map(delivery => ({ delivery, recipients: step.recipients }))
      : [],
  }));
  expect(actual).toEqual([expected, expected]);
  await early.close();
});

test('browser ingress refuses the shared invalid and reserved token classes before ECS', async ({ context }) => {
  const page = await context.newPage();
  await page.goto('/');
  await page.waitForFunction(() => typeof window.wasmBindings?.BrowserConnections === 'function', null, { timeout: WASM_READY_TIMEOUT });
  const code = await readHostPeerId(page);
  for (const [token, expectedCode] of [
    ['x'.repeat(65), 'invalid-token'],
    ['__local_console__', 'reserved-token'],
    ['ai:helm', 'reserved-token'],
  ]) {
    await expect(createTestClient(context, code, { token })).rejects.toThrow(expectedCode);
  }
  expect(await page.evaluate(() => !!window.__wasmReady)).toBe(false);
});
