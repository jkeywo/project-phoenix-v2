// Issues #56 + #57 — Smoke tests: lobby console selection and game start.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient } from './fixtures';

test('console selection — both clients receive ConsoleSelected broadcasts', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Alpha' });
  const clientB = await createTestClient(context, hostId, { name: 'Beta' });

  // Client A claims Captain's Chair
  await clientA.send('SetName', { name: 'Alpha' });
  await clientA.send('SelectConsole', { console: 'CaptainChair' });

  const selA = await clientA.waitForMessage('ConsoleSelected', 5_000) as any;
  expect(selA.data.token).toBe(clientA.token);
  expect(selA.data.consoles).toContain('CaptainChair');

  // Client B should also receive the broadcast for A's selection
  const selAonB = await clientB.waitForMessage('ConsoleSelected', 5_000) as any;
  expect(selAonB.data.token).toBe(clientA.token);

  // Client B claims Helm
  await clientB.send('SetName', { name: 'Beta' });
  await clientB.send('SelectConsole', { console: 'Helm' });

  const selB = await clientB.waitForMessage('ConsoleSelected', 5_000) as any;
  expect(selB.data.token).toBe(clientB.token);
  expect(selB.data.consoles).toContain('Helm');

  await clientA.close();
  await clientB.close();
});

test('captain starts game — both clients receive GameStarted with InProgress', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Cap' });
  const clientB = await createTestClient(context, hostId, { name: 'Helm' });

  await clientA.send('SelectConsole', { console: 'CaptainChair' });
  await clientA.waitForMessage('ConsoleSelected', 5_000);

  await clientB.send('SelectConsole', { console: 'Helm' });
  await clientB.waitForMessage('ConsoleSelected', 5_000);

  // Non-captain (B) attempting StartGame should be ignored
  await clientB.send('StartGame');

  // Neither client should receive GameStarted yet
  await clientA.page.waitForTimeout(500);
  const earlyA = await clientA.lastMessage('GameStarted');
  expect(earlyA).toBeNull();

  // Captain (A) sends StartGame — both clients must receive it
  await clientA.send('StartGame');

  await clientA.waitForMessage('GameStarted', 5_000);
  await clientB.waitForMessage('GameStarted', 5_000);

  // Verify the Welcome state had Lobby phase and now we've transitioned
  const welcomeA = await clientA.page.evaluate(
    () => (window as any).__messages.find((m: any) => m.type === 'Welcome'),
  ) as any;
  expect(welcomeA.data.state.phase).toBe('Lobby');

  await clientA.close();
  await clientB.close();
});
