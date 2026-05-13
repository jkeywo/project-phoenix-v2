import { test as base, BrowserContext, Page } from '@playwright/test';
import fs from 'fs';
import path from 'path';

export const SHIM = fs.readFileSync(path.join(__dirname, 'peerjs-shim.js'), 'utf-8');

// Stub CDN scripts so they don't overwrite the shim or block execution.
// In CI environments the unpkg / jsdelivr CDN can be slow or blocked, and
// synchronous <script src="..."> tags block all inline scripts below them.
const STUB_PEER_JS = `'use strict';
// No-op — window.Peer is already provided by the peerjs-shim addInitScript.
if (typeof window.Peer === 'undefined') { window.Peer = function Peer() {}; };
`;

const STUB_QRCODE = `'use strict';
// Minimal stub so server.html QR rendering code doesn't crash during tests.
window.QRCode = { toCanvas: function () { return Promise.resolve(); } };
`;

// Override the default context fixture to inject the PeerJS shim into every
// page that the test creates.  No type parameter needed when overriding a
// built-in fixture — TypeScript infers BrowserContext from the base signature.
export const test = base.extend({
  context: async ({ browser }, use) => {
    const ctx = await browser.newContext();
    await ctx.addInitScript({ content: STUB_PEER_JS });
    await ctx.addInitScript({ content: STUB_QRCODE });
    await ctx.addInitScript({ content: SHIM });

    // Intercept CDN script loads — stub PeerJS so the real library doesn't
    // overwrite the shim, and stub QRCode so it doesn't block.
    await ctx.route('**/peerjs*.js', (route) =>
      route.fulfill({ contentType: 'application/javascript', body: STUB_PEER_JS }),
    );
    await ctx.route('**/qrcode*.js', (route) =>
      route.fulfill({ contentType: 'application/javascript', body: STUB_QRCODE }),
    );

    await use(ctx);
    await ctx.close();
  },
});

export { expect } from '@playwright/test';

// ── Test client helper ────────────────────────────────────────────────────────
// Creates a blank page at localhost:3000 (same BroadcastChannel origin),
// connects to the host peer, sends Identify, and waits for Welcome.
// Exposes helpers for sending messages and waiting for specific message types.

export interface TestClient {
  readonly page: Page;
  readonly token: string;
  send(type: string, data?: unknown): Promise<void>;
  waitForMessage(type: string, timeout?: number): Promise<Record<string, unknown>>;
  lastMessage(type: string): Promise<Record<string, unknown> | null>;
  close(): Promise<void>;
}

export async function createTestClient(
  ctx: BrowserContext,
  hostId: string,
  opts: { token?: string; name?: string } = {},
): Promise<TestClient> {
  const token = opts.token ?? 'tc-' + Math.random().toString(16).slice(2, 10);
  const name = opts.name ?? 'Tester';

  const page = await ctx.newPage();
  const routeKey = Math.random().toString(16).slice(2, 10);

  await page.route(`**/blank-${routeKey}`, (r) =>
    r.fulfill({ contentType: 'text/html', body: '<!DOCTYPE html><html><body></body></html>' }),
  );
  await page.goto(`http://localhost:3000/blank-${routeKey}`);

  // Connect to host and wait for Welcome before returning.
  await page.evaluate(
    ({ hostId, token, name }) =>
      new Promise<void>((resolve, reject) => {
        (window as any).__messages = [];
        const peer = new (window as any).Peer();
        peer.on('open', () => {
          const conn = peer.connect(hostId);
          (window as any).__conn = conn;
          conn.on('open', () => {
            conn.send(JSON.stringify({ type: 'Identify', data: { token, name } }));
          });
          conn.on('data', (raw: string) => {
            try { (window as any).__messages.push(JSON.parse(raw)); } catch { /* ignore */ }
          });
        });
        const t = setInterval(() => {
          if ((window as any).__messages?.some((m: any) => m.type === 'Welcome')) {
            clearInterval(t);
            resolve();
          }
        }, 50);
        setTimeout(() => { clearInterval(t); reject(new Error(`Welcome timeout (token=${token})`)); }, 15_000);
      }),
    { hostId, token, name },
  );

  return {
    page,
    token,

    async send(type, data?) {
      await page.evaluate(
        ({ type, data }) => {
          const msg = data !== undefined ? { type, data } : { type };
          (window as any).__conn.send(JSON.stringify(msg));
        },
        { type, data },
      );
    },

    async waitForMessage(type, timeout = 15_000) {
      await page.waitForFunction(
        (t) => (window as any).__messages?.some((m: any) => m.type === t),
        type,
        { timeout },
      );
      return page.evaluate(
        (t) => (window as any).__messages.find((m: any) => m.type === t),
        type,
      );
    },

    async lastMessage(type) {
      return page.evaluate(
        (t) => {
          const msgs: any[] = (window as any).__messages || [];
          return msgs.filter((m: any) => m.type === t).pop() ?? null;
        },
        type,
      );
    },

    async close() {
      await page.close();
    },
  };
}

// Reads the host peer ID from the server page's QR-link href, which is set
// after the PeerJS peer opens.
export async function readHostPeerId(serverPage: Page): Promise<string> {
  await serverPage.waitForFunction(
    () => {
      const el = document.getElementById('qr-link') as HTMLAnchorElement | null;
      return el?.href?.includes('#');
    },
    { timeout: 20_000 },
  );
  return serverPage.evaluate(() => {
    const href = (document.getElementById('qr-link') as HTMLAnchorElement).href;
    return href.split('#')[1];
  });
}
