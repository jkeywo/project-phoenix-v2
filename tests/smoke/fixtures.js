// tests/smoke/fixtures.js — shared Playwright fixtures for the smoke tier.
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
//        * `MINIMAL_TEST_WORLD` in `tactical-fire-flow.spec.js` — player ship
//          plus one stationary hostile in phaser range.
//        * `PATROL_TEST_WORLD` in `patrol.spec.js` — player ship plus one
//          NPC raider, for the entity-spawn pipeline.
//        * `MESH_TEST_WORLD` in `ship-mesh-load.spec.js` — player ship plus one
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
//      exist for exactly that, and `strings.js` does the same for string ids.
//
// What is NOT acceptable is replacing a pinned production number with an
// assertion that cannot fail (`toBeGreaterThan(0)` over production data).

import { test as base, expect } from '@playwright/test';
import fs from 'fs';
import path from 'path';
import {
  installTransportFixture,
  readHostJoinTarget,
  connectTestClient,
} from './transport-fixture';

// The transport stand-in itself is no longer re-exported from here. It is
// installed by installTransportFixture() below, and a spec that wants to drive
// it directly (transport-shim.spec.js) does so through the pages this fixture
// already gives it — window.PhoenixTransportFactories and
// window.__transportShim — rather than by injecting a second copy of the
// script into a context of its own.

// The shim runs the REAL rendezvous registry inside the host page rather than a
// second implementation of it, so that module has to be reachable as a URL. It
// is not part of dist/ (it is a Cloudflare Worker's source), so it is served
// from disk through a route. Its own `../../gui/join-code.js` import resolves
// against the site root, where the built gui/ directory already sits.
//
// Its SIBLING imports do not, and that is worth stating because it cost a whole
// smoke run. The registry is served at `/__rendezvous-registry.js`, so the
// `import './relay.js'` #1113 added resolves to `/relay.js` at the site root —
// a URL dist/ has no file for. The dynamic import rejected, the host page's
// registry promise stayed rejected, every `callOwner` silently dropped, no host
// socket ever reached the registry, no `hosted` frame, no five-letter code, and
// EVERY spec timed out waiting for `__wasmReady`. One 404 on one sibling module
// reads as seventeen unrelated failures with nothing in the logs connecting
// them. So each worker-rendezvous module the registry pulls in is routed below,
// at the path its own specifier resolves to.
const workerSrc = (name) =>
  fs.readFileSync(path.join(__dirname, '..', '..', 'worker-rendezvous', 'src', name), 'utf-8');

export const RENDEZVOUS_REGISTRY_JS = workerSrc('registry.js');
export const RENDEZVOUS_RELAY_JS = workerSrc('relay.js');

// Stub the QR CDN script so it doesn't block execution. In CI environments the
// jsdelivr CDN can be slow or blocked, and a synchronous <script src="...">
// tag blocks all inline scripts below it. The transport needs no such stub any
// more — #1112 retired the PeerJS CDN tag, and the replacement is a
// same-origin module island.
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
//   - "Starbase Alpha" (one ~16 MB station GLB) — `comms.spec.js` hails it
//     and `world-bootstrap.spec.js` asserts on its tag. It reaches the hail
//     ROSTER through `station_axiom.toml`'s template-level `[comms] hailable
//     = true` (issue #985), not through this world;
//   - a `[script]` block whose `on_hailed` handler opens a thread with one
//     response, whose `on_pick` adds an objective — required by
//     `comms.spec.js`. It was an `[[comms]]` block with a
//     `[[comms.response.action]]` until issue #985 deleted that front-end.
//
// Position constraint: the player ship's `[comms] range = 1200` and the
// station's `[comms] range = 800` (assets/entities/station_axiom.toml)
// give an effective comms range of 800. The hail in `comms.spec.js`
// requires `in_range = true` on first CommsState, so the starbase must
// sit at distance ≤ 800 from the player. Player spawns at the origin;
// place the starbase well inside the gate at [500, 0, 0].
//
// Tests that need a different scenario keep routing their own world over the
// same `assets/worlds/default.toml` path — `tactical-fire-flow.spec.js` with
// its inline `MINIMAL_TEST_WORLD`, `patrol.spec.js` with `PATROL_TEST_WORLD`
// and `ship-mesh-load.spec.js` with `MESH_TEST_WORLD`. All three are inline
// fixtures; none of them touches the shipped `patrol.toml` any more (issue
// #941). Playwright matches the most-recently-added route first, so the
// per-test override wins over the fixture default below.
export const MINIMAL_DEFAULT_WORLD = `
[global]
seed = 42
title = "Smoke Test World"
description = "Minimal world used by tests/smoke; see fixtures.js."

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

[script]
setup = """
on_hailed("Starbase Alpha", "on_starbase_hailed");

fn on_starbase_hailed(ctx) {
    ctx.effects.open_comms(#{ from: "Starbase Alpha", node_fn: "starbase_hail" });
}

fn starbase_hail(ctx) {
    #{ message: "USS Phoenix, this is Starbase Alpha. Please state your business.",
       responses: [ #{ text: "We are on a survey mission.", on_pick: "on_survey" } ] }
}

fn on_survey(ctx) {
    ctx.effects.add_objective(#{
        id: "obj-survey",
        text: "Complete the survey in this sector."
    });
}
"""
`;

// Override the default context fixture to inject the transport stand-in into
// every page that the test creates. The override keeps Playwright's own
// `context` fixture shape — it still hands each spec a BrowserContext, just one
// with the shim, the QR stub, the registry route and the default world route
// already installed.
export const test = base.extend({
  context: async ({ browser }, use) => {
    const ctx = await browser.newContext();
    await installTransportFixture(ctx);
    await ctx.addInitScript({ content: STUB_QRCODE });

    // Serve the rendezvous service's own registry module to the host page,
    // and the sibling it imports at the path that import resolves to. See the
    // note beside RENDEZVOUS_RELAY_JS — this second route is the difference
    // between the whole suite running and the whole suite timing out.
    await ctx.route('**/__rendezvous-registry.js', (route) =>
      route.fulfill({ contentType: 'application/javascript', body: RENDEZVOUS_REGISTRY_JS }),
    );
    await ctx.route('**/relay.js', (route) =>
      route.fulfill({ contentType: 'application/javascript', body: RENDEZVOUS_RELAY_JS }),
    );

    // Intercept the QR CDN load — stub QRCode so it doesn't block.
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

/** Default timeout for waiting on __wasmReady (PhoenixReady + the host's
 *  rendezvous socket open).
 *
 * In CI / long-running test suites Chrome throttles rAF on non-active pages
 * (~1 fps), which can delay the Bevy init → PhoenixReady dispatch by 20-40 s.
 * 60 s gives enough headroom without making tests flaky locally.
 */
export const WASM_READY_TIMEOUT = 60_000;

/** Bring a server page to front and wait for __wasmReady.
 *  Replaces the ad-hoc 3-line pattern spread across spec files.
 */
export async function waitForWasmReady(page, timeout = WASM_READY_TIMEOUT) {
  await page.bringToFront();
  await page.waitForFunction(() => !!window.__wasmReady, { timeout });
}

/** Capture real server-page crashes while ignoring Bevy WASM's expected
 * run-loop handoff trap, which Chromium reports as a bare "unreachable".
 *
 * Returns the live `string[]` of messages seen so far — assert on it after
 * the work under test, not at the point of the call.
 */
export function captureServerPageErrors(page) {
  const errors = [];
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

/** Capture asset fetches that came back 4xx/5xx, plus the page's own report of
 * them, for the whole life of `page`.
 *
 * A 404 is not a Playwright `requestfailed` — the request succeeded and the
 * server answered "no" — so this listens on `response`, and separately on the
 * console line `server.html` prints when a TOML fetch does not resolve.
 *
 * The defect this exists for: the renderer used to work out a model's LOD-ladder
 * convention by FETCHING the first generated tier's rig sidecar and seeing
 * whether it 404'd. Hull ladders deliberately ship none, so every ship in the
 * scenario begged for a file that was never written and logged an error for it —
 * shipped content, behaving as designed, filling the console with what looked
 * like a broken build. The ladder now states its convention (`tier_rig` in the
 * primary sidecar), so nothing asks. A run that starts asking again is a
 * regression, and it is invisible to every native test: on native the same
 * question is a filesystem read that costs nothing and says nothing.
 *
 * Returns the live `string[]` seen so far — assert on it AFTER the work under
 * test, not at the point of the call.
 */
export function captureFetchFailures(page) {
  const failures = [];
  page.on('response', (res) => {
    if (res.status() < 400) return;
    failures.push(`HTTP ${res.status()} ${new URL(res.url()).pathname}`);
  });
  page.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const text = msg.text();
    if (text.includes('Failed to fetch TOML')) failures.push(text);
  });
  return failures;
}

/**
 * The origin this checkout's `dist/` is served on — 3000 unless
 * `PHOENIX_SMOKE_PORT` moved it, which is how a second worktree avoids silently
 * testing the other one's bundle (see playwright.config.js).
 *
 * Every blank page a spec opens has to be on THIS origin: BroadcastChannel is
 * same-origin, and the transport stand-in pairs pages over one. It was once
 * written as a literal `http://localhost:3000` in three places, so
 * PHOENIX_SMOKE_PORT moved the host page and left every test client dialling a
 * port with nothing on it — 95 specs failing as "Welcome timeout", which reads
 * exactly like a broken join path rather than a wrong port.
 */
export const SMOKE_ORIGIN = `http://localhost:${Number(process.env.PHOENIX_SMOKE_PORT || 3000)}`;

// ── Test client helper ────────────────────────────────────────────────────────
// Creates a blank page on SMOKE_ORIGIN (same BroadcastChannel origin),
// joins the host through the rendezvous service with its join code, completes
// the compatibility handshake, sends Identify, and waits for Welcome.
// Exposes helpers for sending messages and waiting for specific message types.

/**
 * The object `createTestClient` returns. Its shape is the contract every spec
 * that takes a "client" argument relies on:
 *
 *   page                      Page — the blank page this client owns
 *   token                     string — the session token it identified with
 *   send(type, data?)         → Promise<void>, sends `{ type, data }` on the
 *                               reliable channel (`data` omitted when undefined)
 *   waitForMessage(type, ms?) → Promise<object>, the FIRST message of `type`
 *   lastMessage(type)         → Promise<object|null>, the MOST RECENT one, or
 *                               null if none has arrived
 *   close()                   → Promise<void>
 *
 * @typedef {object} TestClient
 */

/**
 * @param {import('@playwright/test').BrowserContext} ctx
 * @param {string} hostId  join code read from the server page's QR link
 * @param {{ token?: string, name?: string, waitFor?: string, snapshot?: boolean }} [opts]
 *   `waitFor` is the message type to block on before resolving — defaults to
 *   `'Welcome'` (the world has loaded). A phone that connects DURING the
 *   QR-first scenario stage never gets a Welcome (Bevy is not running yet); the
 *   host answers `Identify` with a synthesized `ScenarioCatalog` (server.html
 *   `sendCatalogTo`), so those specs pass `waitFor: 'ScenarioCatalog'`.
 *   `snapshot` (default true) negotiates the lossy unordered channel alongside
 *   the reliable one, as the shipped client does; `false` models a client whose
 *   lossy channel never came up.
 * @returns {Promise<TestClient>}
 */
export async function createTestClient(
  ctx,
  hostId,
  opts = {},
) {
  const token = opts.token ?? 'tc-' + Math.random().toString(16).slice(2, 10);
  const name = opts.name ?? 'Tester';
  const waitFor = opts.waitFor ?? 'Welcome';

  const page = await ctx.newPage();
  const routeKey = Math.random().toString(16).slice(2, 10);

  await page.route(`**/blank-${routeKey}`, (r) =>
    r.fulfill({ contentType: 'text/html', body: '<!DOCTYPE html><html><body></body></html>' }),
  );
  await page.goto(`${SMOKE_ORIGIN}/blank-${routeKey}`);

  // Connect to host and wait for the readiness message before returning.
  // Every transport-shaped detail of "connect" lives in transport-fixture.js
  // — this function only owns the Playwright page lifecycle and the stable
  // façade shape below.
  const { send, waitForMessage, lastMessage } = await connectTestClient(
    page,
    hostId,
    { token, name, waitFor, snapshot: opts.snapshot },
  );

  const client = {
    page,
    token,
    send,
    waitForMessage,
    lastMessage,

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
export function stripHeavyEntities(toml) {
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

// Boots a fresh server page and resolves to it once __wasmReady is set.
export async function createServerPage(
  ctx,
) {
  const page = await ctx.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await page.bringToFront();
  await page.waitForFunction(() => !!window.__wasmReady, { timeout: WASM_READY_TIMEOUT });
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
export function countTableArray(toml, name) {
  const re = new RegExp(String.raw`^\[\[${name}\]\]\s*$`, 'gm');
  return toml.match(re)?.length ?? 0;
}

/** Every value of `key` across the `[[name]]` blocks in `toml`, in order. */
export function tableArrayValues(toml, name, key) {
  const blocks = toml.split(new RegExp(String.raw`^\[\[${name}\]\]\s*$`, 'm')).slice(1);
  const out = [];
  for (const block of blocks) {
    // Stop at the next table header so a block's fields can't leak into it.
    const body = block.split(/^\[/m)[0];
    const m = body.match(new RegExp(String.raw`^\s*${key}\s*=\s*"([^"]*)"`, 'm'));
    if (m) out.push(m[1]);
  }
  return out;
}

/** The body of the `[section]` table in `toml`, up to the next table header. */
function tableBody(toml, section) {
  const escaped = section.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const start = toml.match(new RegExp(String.raw`^\[${escaped}\]\s*$`, 'm'));
  if (!start || start.index === undefined) return undefined;
  const rest = toml.slice(start.index + start[0].length);
  return rest.split(/^\[/m)[0];
}

/** A quoted string value from `[section]`. Throws if absent — a missing key is
 *  a broken expectation, not a reason to silently assert `undefined`. */
export function tomlString(toml, section, key) {
  const body = tableBody(toml, section);
  const m = body?.match(new RegExp(String.raw`^\s*${key}\s*=\s*"([^"]*)"`, 'm'));
  if (!m) throw new Error(`TOML has no string [${section}].${key}`);
  return m[1];
}

/** A numeric value from `[section]`. Throws if absent (see `tomlString`). */
export function tomlNumber(toml, section, key) {
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
export function expectFixtureWorld(worldSetupMsg, fixtureToml) {
  const title = tomlString(fixtureToml, 'global', 'title');
  const served = worldSetupMsg?.data?.world?.scenario_title;
  expect(
    served,
    `WorldSetup carries scenario_title ${JSON.stringify(served)}, not this spec's ` +
      `fixture world ${JSON.stringify(title)} — the context.route glob almost ` +
      'certainly stopped matching and production assets/worlds/ was served instead',
  ).toBe(title);
}

// ── Committed mod-pack fixtures (issues #986–#991) ───────────────────────────
//
// The smoke spec (mod-pack.spec.js) uploads the committed `.zip` archives under
// tests/fixtures/mod-packs/ and derives its expectations (pack name/version,
// scenario id, mod world path) from each pack's own manifest rather than pinning
// literals — the same derive-don't-pin rule the rest of this file follows. The
// archives are byte-reproducible via `scripts/build-mod-pack-fixtures.mjs`.

/** Absolute path to a committed fixture archive by bare name (no `.zip`). */
export function modPackFixturePath(name) {
  return path.join(__dirname, '../fixtures/mod-packs', `${name}.zip`);
}

/**
 * Extract the `scenarios.toml` manifest TEXT from a committed fixture archive.
 *
 * A deliberately tiny store-only ZIP reader that does NOT verify CRCs — so it
 * still reads the manifest out of the `corrupt-crc` fixture, whose corruption is
 * confined to another entry's CRC. Returns the manifest as a string, or throws
 * if the archive carries no `scenarios.toml`. Feed the result to `tomlString` /
 * `tableArrayValues` to read the pack + scenario fields.
 */
export function readModPackManifest(name) {
  const bytes = fs.readFileSync(modPackFixturePath(name));
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const dec = new TextDecoder();
  let pos = 0;
  while (pos + 4 <= bytes.length && view.getUint32(pos, true) === 0x04034b50) {
    const compSize = view.getUint32(pos + 18, true);
    const nameLen = view.getUint16(pos + 26, true);
    const extraLen = view.getUint16(pos + 28, true);
    const nameStart = pos + 30;
    const dataStart = nameStart + nameLen + extraLen;
    const entry = dec.decode(bytes.subarray(nameStart, nameStart + nameLen));
    if (entry === 'scenarios.toml') {
      return dec.decode(bytes.subarray(dataStart, dataStart + compSize));
    }
    pos = dataStart + compSize;
  }
  throw new Error(`mod-pack fixture ${name}.zip has no scenarios.toml`);
}

// Reads the host's join target from the server page's QR-link href, which is
// set once the rendezvous service has issued this host a code. Thin wrapper
// kept under its original name/signature for the ~44 spec files that import it
// — the actual scrape lives in transport-fixture.js's readHostJoinTarget, the
// neutral seam this name delegates to. The NAME is now historical: what comes
// back is a structured join code, not a peer id (issue #1112).
export async function readHostPeerId(serverPage) {
  return readHostJoinTarget(serverPage);
}
