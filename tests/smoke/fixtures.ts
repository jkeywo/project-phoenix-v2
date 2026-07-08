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

// Minimal default world used by every smoke test that doesn't route its own
// scenario. The production `assets/worlds/default.toml` references a planet
// (~36 MB GLB) and an asteroid field that pulls in 12 asteroid templates
// (~150 MB of GLBs), plus a sun and a nebula region. The lobby preload gate
// waits for every GLB to reach a terminal `LoadState` before allowing
// `StartGame`, and headless Chromium can't realistically fetch + parse all
// of that on a backgrounded server page within the 5 s `GameStarted`
// timeout used by most specs.
//
// This minimal world keeps only what the smoke suite actually inspects:
//
  //   - the player ship (no GLB — the ship TOML is icon-only);
//   - "Starbase Alpha" (one ~16 MB station GLB) — `comms.spec.ts` hails it
//     and `world-bootstrap.spec.ts` asserts on its tag;
//   - an `[[comms]] on_hailed` block with a response carrying an
//     `add_objective` action — required by `comms.spec.ts`.
//
// Position constraint: the player ship's `[comms] range = 1200` and the
// station's `[comms] range = 800` (assets/entities/station_axiom.toml)
// give an effective comms range of 800. The hail in `comms.spec.ts`
// requires `in_range = true` on first CommsState, so the starbase must
// sit at distance ≤ 800 from the player. Player spawns at the origin;
// place the starbase well inside the gate at [500, 0, 0].
//
// Tests that need a different scenario (`tactical-fire-flow.spec.ts` with
// its inline `MINIMAL_TEST_WORLD`, `patrol.spec.ts` and
// `ship-mesh-load.spec.ts` with `patrol.toml`) keep routing their own
// world; Playwright matches the most-recently-added route first, so the
// per-test override wins over the fixture default below.
export const MINIMAL_DEFAULT_WORLD = `
[global]
seed = 42
title = "Smoke Test World"
description = "Minimal world used by tests/smoke; see fixtures.ts."

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[anchors]
starbase_alpha = [500.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id            = "player-ship"
transform     = { position = [0.0, 0.0, 0.0] }
spawn_on      = "game_start"

[[entity]]
template_path = "assets/entities/station_axiom.toml"
name          = "Starbase Alpha"
transform     = { anchor = "starbase_alpha" }

[[comms]]
from    = "Starbase Alpha"
trigger = "on_hailed"
entity  = "Starbase Alpha"
message = "USS Phoenix, this is Starbase Alpha. Please state your business."

  [[comms.response]]
  text = "We are on a survey mission."
    [[comms.response.action]]
    type = "add_objective"
    id   = "obj-survey"
    text = "Complete the survey in this sector."
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

    // Default scenario: serve the minimal smoke-test world above instead of
    // the production `assets/worlds/default.toml`. See MINIMAL_DEFAULT_WORLD
    // for the full rationale. Tests that route their own scenario register
    // their handler later, so it matches first (most-recently-added wins).
    await ctx.route('**/assets/worlds/default.toml', (route) =>
      route.fulfill({ contentType: 'text/plain', body: MINIMAL_DEFAULT_WORLD }),
    );

    await use(ctx);
    await ctx.close();
  },
});

export { expect } from '@playwright/test';

/** Default timeout for waiting on __wasmReady (PhoenixReady + Peer open).
 *
 * In CI / long-running test suites Chrome throttles rAF on non-active pages
 * (~1 fps), which can delay the Bevy init → PhoenixReady dispatch by 20-40 s.
 * 60 s gives enough headroom without making tests flaky locally.
 */
export const WASM_READY_TIMEOUT = 60_000;

/** Bring a server page to front and wait for __wasmReady.
 *  Replaces the ad-hoc 3-line pattern spread across spec files.
 */
export async function waitForWasmReady(page: Page, timeout = WASM_READY_TIMEOUT): Promise<void> {
  await page.bringToFront();
  await page.waitForFunction(() => !!(window as any).__wasmReady, { timeout });
}

/** Capture real server-page crashes while ignoring Bevy WASM's expected
 * run-loop handoff trap, which Chromium reports as a bare "unreachable".
 */
export function captureServerPageErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (err) => {
    if (err.message === 'unreachable') return;
    errors.push(err.message);
  });
  page.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const text = msg.text();
    if (
      text.includes('panicked at') ||
      text.includes('RuntimeError') ||
      text.includes('memory access out of bounds')
    ) {
      errors.push(text);
    }
  });
  return errors;
}

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

  const client: TestClient = {
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
  return client;
}

/**
 * Strips heavy `[[entity]]` blocks from a world TOML so the lobby preload
 * gate clears in CI. Specifically removes any block whose `template_path`
 * references the asteroid field, planet, sun, or nebula region — large-GLB
 * templates that aren't load-bearing for the smoke suite.
 *
 * The asteroid_field_main template alone pulls in 12 asteroid GLBs
 * (~150 MB total); the gate waits for every GLB to reach a terminal
 * `LoadState` before allowing `StartGame`, and the tests time out long
 * before that happens.
 *
 * Use this in per-test routes that fulfil a real world file:
 *
 * ```ts
 * await context.route('**\/assets/worlds/default.toml', (route) =>
 *   route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(PATROL_TOML) }),
 * );
 * ```
 *
 * The regex is anchored to start-of-line (`m` flag) so `[[entity]]` text
 * inside a `#` comment doesn't trigger the match, and the block body uses
 * a tempered greedy `(?:(?!^\[\[)[\s\S])*?` so the match never crosses
 * into a sibling `[[...]]` block — even if an intermediate non-heavy
 * `[[entity]]` block sits between two heavy ones.
 */
export function stripHeavyEntities(toml: string): string {
  const HEAVY_TEMPLATES = [
    'asteroid_field_',
    'planet_earth',
    'star_sun',
    'region_nebula',
  ];
  const heavyPattern = HEAVY_TEMPLATES.map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|');
  const blockRegex = new RegExp(
    String.raw`^\[\[entity\]\](?:(?!^\[\[)[\s\S])*?template_path\s*=\s*"[^"]*(?:${heavyPattern})[^"]*"(?:(?!^\[\[)[\s\S])*`,
    'gm',
  );
  return toml.replace(blockRegex, '').replace(/\n{3,}/g, '\n\n');
}

// Boots a fresh server page.
export async function createServerPage(
  ctx: BrowserContext,
): Promise<Page> {
  const page = await ctx.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await page.bringToFront();
  await page.waitForFunction(() => !!(window as any).__wasmReady, { timeout: WASM_READY_TIMEOUT });
  return page;
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
