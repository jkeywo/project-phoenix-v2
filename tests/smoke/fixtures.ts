// tests/smoke/fixtures.ts — shared Playwright fixtures for the smoke tier.
//
// ── Where a spec's world data comes from (issue #941) ────────────────────────
//
// Smoke specs must not assert on the *current contents* of production
// `assets/` — an entity count, a hull HP total, a scenario roster or a
// per-system tuning value moves whenever a designer edits a TOML, and a spec
// pinned to it breaks for reasons that have nothing to do with the code under
// test. Two sanctioned patterns, in order of preference:
//
//   1. **Self-contained fixture world** — a small TOML string served over the
//      requested world path with `context.route`. This is the convention this
//      suite already uses; the fixtures are inline TOML template literals, NOT
//      files under `tests/smoke/fixtures/` (no such directory exists — do not
//      invent one without moving all of the below into it). The committed
//      fixture worlds are:
//
//        * `MINIMAL_DEFAULT_WORLD` (below) — served for
//          `assets/worlds/default.toml` by the `context` fixture, so it is the
//          default world for every spec that does not route its own. Used by
//          `world-bootstrap`, `nav-chart-pipeline`, `comms`, `stations`,
//          `lobby`, `engineering`, `sim-state`, `view-selector`, …
//        * `MINIMAL_TEST_WORLD` in `tactical-fire-flow.spec.ts` — player ship
//          plus one stationary hostile in phaser range.
//        * `PATROL_TEST_WORLD` in `patrol.spec.ts` — player ship plus one
//          NPC raider, for the entity-spawn pipeline.
//        * `MESH_TEST_WORLD` in `ship-mesh-load.spec.ts` — player ship plus one
//          NPC whose hull declares a GLB, for the model-path transform.
//
//      Two rules for anything served this way:
//
//        a. **Give `[global]` a title and assert it** — call
//           `expectFixtureWorld(worldSetupMsg, THE_FIXTURE)` before asserting on
//           the world's contents. A `context.route` glob that stops matching
//           falls through to production `assets/` *silently*, and production
//           `default.toml` happens to supply both a `station`-tagged entity and
//           an `npc`-tagged raider — so without this the fallthrough leaves the
//           specs green while testing exactly the content #941 decoupled them
//           from.
//        b. **Author tags under `overrides`, not on the block.** `WorldEntity`
//           (src/world/config.rs) has no `tags` field, and serde ignores
//           unknown keys, so a bare `tags = [...]` on an `[[entity]]` block is
//           silently dropped and the entity keeps whatever its production
//           template authored. `overrides = { tags = [...] }` is the form the
//           shipped worlds use; the instance layer *replaces* the array rather
//           than unioning it (src/entities/entity_override.rs), so it can take
//           a tag away as well as add one.
//
//   2. **Derive the expectation from the TOML the test itself serves** — for
//      the few specs that genuinely exercise a *shipped* asset (the demo
//      scenario manifest, the combat_test world), read the value out of the
//      TOML instead of pinning a literal. `assertion == what the TOML says`
//      still fails when the pipeline drops or mangles the value, but survives
//      a designer retuning it. The helpers at the bottom of this file
//      (`countTableArray`, `tomlString`, `tomlNumber`, `tableArrayValues`)
//      exist for exactly that, and `strings.ts` does the same for string ids.
//
// What is NOT acceptable is replacing a pinned production number with an
// assertion that cannot fail (`toBeGreaterThan(0)` over production data).

import { test as base, expect, BrowserContext, Page } from '@playwright/test';
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
// of that on a backgrounded server page within the `GameStarted` timeout
// used by most specs.
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
// Tests that need a different scenario keep routing their own world over the
// same `assets/worlds/default.toml` path — `tactical-fire-flow.spec.ts` with
// its inline `MINIMAL_TEST_WORLD`, `patrol.spec.ts` with `PATROL_TEST_WORLD`
// and `ship-mesh-load.spec.ts` with `MESH_TEST_WORLD`. All three are inline
// fixtures; none of them touches the shipped `patrol.toml` any more (issue
// #941). Playwright matches the most-recently-added route first, so the
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

export { expect };

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
    'planet_',
    'moon_',
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

// ── Reading an expectation out of TOML instead of pinning it (issue #941) ────
//
// Deliberately tiny, deliberately not a TOML parser: these read the handful of
// shapes the smoke specs need out of the exact text the spec is serving to the
// page. A spec that needs more than this should be using a fixture world
// instead (see the header of this file).

/** Number of `[[name]]` array-of-table blocks in `toml`.
 *
 *  Anchored to start-of-line so `[[name]]` inside a `#` comment is ignored.
 */
export function countTableArray(toml: string, name: string): number {
  const re = new RegExp(String.raw`^\[\[${name}\]\]\s*$`, 'gm');
  return toml.match(re)?.length ?? 0;
}

/** Every value of `key` across the `[[name]]` blocks in `toml`, in order. */
export function tableArrayValues(toml: string, name: string, key: string): string[] {
  const blocks = toml.split(new RegExp(String.raw`^\[\[${name}\]\]\s*$`, 'm')).slice(1);
  const out: string[] = [];
  for (const block of blocks) {
    // Stop at the next table header so a block's fields can't leak into it.
    const body = block.split(/^\[/m)[0];
    const m = body.match(new RegExp(String.raw`^\s*${key}\s*=\s*"([^"]*)"`, 'm'));
    if (m) out.push(m[1]);
  }
  return out;
}

/** The body of the `[section]` table in `toml`, up to the next table header. */
function tableBody(toml: string, section: string): string | undefined {
  const escaped = section.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const start = toml.match(new RegExp(String.raw`^\[${escaped}\]\s*$`, 'm'));
  if (!start || start.index === undefined) return undefined;
  const rest = toml.slice(start.index + start[0].length);
  return rest.split(/^\[/m)[0];
}

/** A quoted string value from `[section]`. Throws if absent — a missing key is
 *  a broken expectation, not a reason to silently assert `undefined`. */
export function tomlString(toml: string, section: string, key: string): string {
  const body = tableBody(toml, section);
  const m = body?.match(new RegExp(String.raw`^\s*${key}\s*=\s*"([^"]*)"`, 'm'));
  if (!m) throw new Error(`TOML has no string [${section}].${key}`);
  return m[1];
}

/** A numeric value from `[section]`. Throws if absent (see `tomlString`). */
export function tomlNumber(toml: string, section: string, key: string): number {
  const body = tableBody(toml, section);
  const m = body?.match(new RegExp(String.raw`^\s*${key}\s*=\s*(-?[0-9.]+)`, 'm'));
  if (!m) throw new Error(`TOML has no number [${section}].${key}`);
  return parseFloat(m[1]);
}

/** Fail unless the `WorldSetup` that arrived was built from `fixtureToml`.
 *
 *  Route interception is a silent failure mode: `context.route` never reports
 *  that a glob matched nothing, so a typo'd or out-of-date pattern simply lets
 *  the request through to the real file under `assets/worlds/`. Production
 *  `default.toml` supplies both a `station`-tagged entity and an `npc`-tagged
 *  raider, which is exactly what the fixture-served specs look for — so the
 *  fallthrough leaves them green while asserting on the production content
 *  issue #941 exists to decouple them from.
 *
 *  `WorldData.scenario_title` is `[global].title` verbatim (declared in
 *  src/core/messages.rs, populated in src/world/server.rs), so it identifies
 *  the world that was actually parsed. The expected value is read back out of
 *  the fixture text rather than written down a second time — same
 *  derive-don't-pin rule as the rest of this file — which also makes a fixture
 *  that forgets to declare a title throw here instead of asserting `undefined`.
 */
export function expectFixtureWorld(worldSetupMsg: unknown, fixtureToml: string): void {
  const title = tomlString(fixtureToml, 'global', 'title');
  const served = (worldSetupMsg as any)?.data?.world?.scenario_title;
  expect(
    served,
    `WorldSetup carries scenario_title ${JSON.stringify(served)}, not this spec's ` +
      `fixture world ${JSON.stringify(title)} — the context.route glob almost ` +
      'certainly stopped matching and production assets/worlds/ was served instead',
  ).toBe(title);
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
