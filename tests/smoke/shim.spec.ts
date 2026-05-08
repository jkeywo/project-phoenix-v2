// Issue #52 — PeerJS shim unit tests.
// Two blank pages at localhost:3000 verify the BroadcastChannel routing without
// touching production WASM at all.

import { test, expect } from '@playwright/test';
import type { BrowserContext } from '@playwright/test';
import { SHIM } from './fixtures';

// Opens a blank page at localhost:3000 (required for BroadcastChannel cross-page
// messaging — null-origin pages can't communicate).
async function blankPage(ctx: BrowserContext, slug: string) {
  const page = await ctx.newPage();
  await page.route(`**/${slug}`, (r: any) =>
    r.fulfill({ contentType: 'text/html', body: '<!DOCTYPE html><html><body></body></html>' }),
  );
  await page.goto(`http://localhost:3000/${slug}`);
  return page;
}

// Creates a fresh context with the shim injected and returns two blank pages.
async function twoPages(browser: any, slugA: string, slugB: string) {
  const ctx = await browser.newContext();
  await ctx.addInitScript({ content: SHIM });
  const a = await blankPage(ctx, slugA);
  const b = await blankPage(ctx, slugB);
  return { ctx, a, b };
}

test.describe('peerjs-shim', () => {
  test('host peer fires open with its own ID', async ({ browser }) => {
    const ctx = await browser.newContext();
    await ctx.addInitScript({ content: SHIM });
    const page = await blankPage(ctx, 'shim-open-1');

    const id = await page.evaluate(() =>
      new Promise<string>((resolve) => {
        new (window as any).Peer('host-open-test').on('open', resolve);
      }),
    );

    expect(id).toBe('host-open-test');
    await ctx.close();
  });

  test('client peer fires open with a generated ID', async ({ browser }) => {
    const ctx = await browser.newContext();
    await ctx.addInitScript({ content: SHIM });
    const page = await blankPage(ctx, 'shim-open-2');

    const id = await page.evaluate(() =>
      new Promise<string>((resolve) => {
        new (window as any).Peer().on('open', resolve);
      }),
    );

    expect(typeof id).toBe('string');
    expect(id.length).toBeGreaterThan(0);
    await ctx.close();
  });

  test('client connects to host — both sides receive open', async ({ browser }) => {
    const { ctx, a: hostPage, b: clientPage } = await twoPages(
      browser, 'shim-connect-host', 'shim-connect-client',
    );

    // Set up host peer; capture incoming connection
    await hostPage.evaluate(() => {
      (window as any).__hostConnOpen = false;
      const host = new (window as any).Peer('conn-host-1');
      host.on('connection', (conn: any) => {
        conn.on('open', () => { (window as any).__hostConnOpen = true; });
      });
    });

    // Wait for host peer.on('open') to fire (async Promise.resolve)
    await hostPage.waitForTimeout(100);

    const clientConnOpen = await clientPage.evaluate(() =>
      new Promise<boolean>((resolve) => {
        const client = new (window as any).Peer();
        client.on('open', () => {
          const conn = client.connect('conn-host-1');
          conn.on('open', () => resolve(true));
        });
        setTimeout(() => resolve(false), 5_000);
      }),
    );

    expect(clientConnOpen).toBe(true);
    await hostPage.waitForFunction(() => (window as any).__hostConnOpen === true, { timeout: 2_000 });
    await ctx.close();
  });

  test('host sends data to client', async ({ browser }) => {
    const { ctx, a: hostPage, b: clientPage } = await twoPages(
      browser, 'shim-data-host', 'shim-data-client',
    );

    await hostPage.evaluate(() => {
      const host = new (window as any).Peer('data-host-1');
      host.on('connection', (conn: any) => {
        conn.on('open', () => conn.send('hello-from-host'));
      });
    });
    await hostPage.waitForTimeout(100);

    const received = await clientPage.evaluate(() =>
      new Promise<string>((resolve) => {
        const client = new (window as any).Peer();
        client.on('open', () => {
          const conn = client.connect('data-host-1');
          conn.on('data', (d: string) => resolve(d));
        });
        setTimeout(() => resolve('__timeout__'), 5_000);
      }),
    );

    expect(received).toBe('hello-from-host');
    await ctx.close();
  });

  test('client sends data to host', async ({ browser }) => {
    const { ctx, a: hostPage, b: clientPage } = await twoPages(
      browser, 'shim-c2h-host', 'shim-c2h-client',
    );

    await hostPage.evaluate(() => {
      (window as any).__hostReceived = null;
      const host = new (window as any).Peer('c2h-host-1');
      host.on('connection', (conn: any) => {
        conn.on('data', (d: string) => { (window as any).__hostReceived = d; });
      });
    });
    await hostPage.waitForTimeout(100);

    await clientPage.evaluate(() =>
      new Promise<void>((resolve) => {
        const client = new (window as any).Peer();
        client.on('open', () => {
          const conn = client.connect('c2h-host-1');
          conn.on('open', () => { conn.send('hello-from-client'); resolve(); });
        });
      }),
    );

    await hostPage.waitForFunction(
      () => (window as any).__hostReceived === 'hello-from-client',
      { timeout: 3_000 },
    );
    await ctx.close();
  });

  test('conn.close() fires close event on the remote side', async ({ browser }) => {
    const { ctx, a: hostPage, b: clientPage } = await twoPages(
      browser, 'shim-close-host', 'shim-close-client',
    );

    await hostPage.evaluate(() => {
      (window as any).__hostClosed = false;
      const host = new (window as any).Peer('close-host-1');
      host.on('connection', (conn: any) => {
        conn.on('close', () => { (window as any).__hostClosed = true; });
      });
    });
    await hostPage.waitForTimeout(100);

    await clientPage.evaluate(() =>
      new Promise<void>((resolve) => {
        const client = new (window as any).Peer();
        client.on('open', () => {
          const conn = client.connect('close-host-1');
          conn.on('open', () => { conn.close(); resolve(); });
        });
      }),
    );

    await hostPage.waitForFunction(
      () => (window as any).__hostClosed === true,
      { timeout: 3_000 },
    );
    await ctx.close();
  });
});
