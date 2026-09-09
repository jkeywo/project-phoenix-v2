// #1181: synchronous JS callback replacement must not borrow browser storage
// across an invocation. These use the real scheduled bridge flushes.
import { test, expect, createServerPage, readHostPeerId, createTestClient,
  captureServerPageErrors } from './fixtures';

test('host-channel callback can replace itself synchronously', async ({ context }) => {
  const page = await createServerPage(context);
  const errors = captureServerPageErrors(page);
  await page.evaluate(() => {
    window.__edgeCallbacks = [0, 0];
    window.wasmBindings.set_host_channel_callback(() => {
      window.__edgeCallbacks[0]++;
      window.wasmBindings.set_host_channel_callback(() => { window.__edgeCallbacks[1]++; });
    });
  });
  await page.waitForFunction(() => window.__edgeCallbacks.every(count => count > 0));
  expect(errors).toEqual([]);
});

test('outbound callback can replace itself synchronously', async ({ context }) => {
  const page = await createServerPage(context);
  const client = await createTestClient(context, await readHostPeerId(page), { name: 'Edge callback' });
  await client.send('SelectStation', { station: 'Captain' });
  await client.page.waitForFunction(token => window.__messages?.some(message =>
    message.type === 'StationAssigned' && message.data.token === token), client.token);
  const errors = captureServerPageErrors(page);
  await page.evaluate(() => {
    window.__edgeCallbacks = [0, 0];
    window.wasmBindings.set_message_callback(() => {
      window.__edgeCallbacks[0]++;
      window.wasmBindings.set_message_callback(() => { window.__edgeCallbacks[1]++; });
    });
  });
  await client.send('SetReady', { ready: true });
  await page.waitForFunction(() => window.__edgeCallbacks.every(count => count > 0));
  expect(errors).toEqual([]);
  await client.close();
});
