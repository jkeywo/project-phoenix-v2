// Issue #1098 — Smoke test: Comms as a hosted-tab-only visiting Station on the
// Alliance Destroyer.
//
// `comms.spec.js` already covers the direct-hosting message flow (hail →
// respond → objective) against the cruiser's older Comms wiring. This spec
// is scoped to the destroyer's Comms, which is now an AUXILIARY
// `human_seeking` Station resolved by `host_order` (see
// `assets/entities/alliance_destroyer.toml`'s `[[station]] id = "comms"`),
// same as Navigation. Auxiliary means Comms has NO dedicated claimable lobby
// seat — it is never a direct holder; it is only ever reached as a Hero Bar
// tab hosted by a resolved seat, or AI-operated. This spec demonstrates both
// remaining host outcomes the acceptance criteria name — visiting and
// AI-hosted — plus the hosted-tab resolution onto a non-owning seat, via
// `SimState.snapshot.station_hosts` (the wire projection of
// `resolve_visiting_station`, `src/ship/coordination.rs`) plus a live
// `CommsState`/`Hail` round trip to prove a visiting host can actually act,
// not just get named as host. The prior "direct: a player seated on Comms is
// its own host" case is gone: Comms is no longer a claimable seat, so its
// first case now pins hosted-tab resolution onto a seated Captain instead.
//
// Navigation's equivalent migration (#1097, same host_order machinery) has
// no smoke coverage of its own visiting/AI-hosted behavior either — nothing
// under tests/smoke/ references `host_order` or `station_hosts` before this
// file. This spec follows the closest existing smoke pattern instead
// (`comms.spec.js`'s hail/respond flow and `combat-test-scenario.spec.js`'s
// solo-captain auto-start), since there is no prior visiting-station smoke
// spec to mirror directly.

import {
  test, expect, readHostPeerId, createTestClient, waitForWasmReady,
} from './fixtures';

// Minimal destroyer-hulled world, mirroring MINIMAL_DEFAULT_WORLD in
// fixtures.js but swapping the cruiser for the alliance_destroyer hull this
// spec actually needs (comms `host_order` is only authored on the
// destroyer's `[[station]]` block, not the cruiser's). Comms range is 1000
// (alliance_destroyer.toml `[comms] range = 1000`), the starbase's own range
// is 800 (assets/entities/station_axiom.toml), so the effective hail range
// is 800 — the starbase sits at [500, 0, 0], well inside it, the player ship
// at the origin.
const DESTROYER_WORLD = `
[global]
seed = 42
title = "Comms Visiting Station Smoke World"
description = "Minimal destroyer-hulled world for tests/smoke/comms-visiting-station.spec.js."

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[anchors]
starbase_alpha = [500.0, 0.0, 0.0]

# A single [[available_ships]] entry auto-selects and skips the lobby ship
# picker (server.html's 'auto-select' branch of resolveShipSelection). This
# also matters for validate_ship_stations: with no available_ships at all,
# server.html falls back to a hardcoded alliance_cruiser.toml gate-check
# (its "legacy-fallback" branch) regardless of which hull the world actually
# spawns, and that cruiser's include closure was never queued by the asset
# preload discovery this world drives (which only walks THIS world's
# available_ships / [[entity]] template paths) — an unrelated "include not
# found" fault. Authoring the destroyer here avoids that path entirely.
[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"

[[entity]]
template_path = "assets/entities/alliance_destroyer.toml"
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

async function bootDestroyerServer(context) {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: DESTROYER_WORLD }));
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  return serverPage;
}

async function waitForStation(client, timeout = 8_000) {
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

/** The most recent `SimState.snapshot.station_hosts` entry for `stationId`. */
async function lastCommsHost(client, stationId, timeout = 8_000) {
  await client.page.waitForFunction(
    (id) => (window.__messages || []).some(
      (m) => m.type === 'SimState'
        && (m.data.snapshot.station_hosts || []).some((h) => h.station === id),
    ),
    stationId,
    { timeout },
  );
  return client.page.evaluate((id) => {
    const msgs = (window.__messages || []).filter((m) => m.type === 'SimState');
    const last = msgs[msgs.length - 1];
    return last.data.snapshot.station_hosts.find((h) => h.station === id) ?? null;
  }, stationId);
}

/** The latest `SimState.snapshot.station_health` array, once one arrives. */
async function lastStationHealth(client, timeout = 8_000) {
  await client.page.waitForFunction(
    () => (window.__messages || []).some(
      (m) => m.type === 'SimState' && (m.data.snapshot.station_health || []).length > 0,
    ),
    null,
    { timeout },
  );
  return client.page.evaluate(() => {
    const msgs = (window.__messages || []).filter(
      (m) => m.type === 'SimState' && (m.data.snapshot.station_health || []).length > 0,
    );
    return msgs[msgs.length - 1].data.snapshot.station_health;
  });
}

/** The latest non-empty `SimState.snapshot.station_importance` array (#1101). */
async function lastStationImportance(client, timeout = 8_000) {
  await client.page.waitForFunction(
    () => (window.__messages || []).some(
      (m) => m.type === 'SimState' && (m.data.snapshot.station_importance || []).length > 0,
    ),
    null,
    { timeout },
  );
  return client.page.evaluate(() => {
    const msgs = (window.__messages || []).filter(
      (m) => m.type === 'SimState' && (m.data.snapshot.station_importance || []).length > 0,
    );
    return msgs[msgs.length - 1].data.snapshot.station_importance;
  });
}

// Issue #1100: the host publishes authoritative per-Station health station-level
// in `SimState.snapshot.station_health`, exactly like `station_hosts`, so every
// client shows a Station's health from the host's own sum rather than inferring
// it from recipient-scoped damage rows. This asserts the wire projection reaches
// a client, covering the two states a freshly-spawned, undamaged destroyer can
// demonstrate cheaply: HEALTHY stations (summed hull at full) and NO-DAMAGE-MODEL
// stations (owning no damageable Systems, so they never appear — the neutral
// state the Hero Bar renders as a definite cue). The DAMAGED state and the
// explicit `None` wire encoding are pinned by the Rust projection tests
// (`repair::visibility`) and the vitest client tests (hero-bar / sim-state),
// since the smoke harness has no cheap lever to damage a player System.
test('station health — the host publishes authoritative per-Station health station-level', async ({ context }) => {
  const serverPage = await bootDestroyerServer(context);
  const hostId = await readHostPeerId(serverPage);

  const tactical = await createTestClient(context, hostId, { name: 'Tactical' });
  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);
  await tactical.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);

  const health = await lastStationHealth(tactical);
  const byStation = Object.fromEntries(health.map((e) => [e.station, e.health]));

  // HEALTHY: on a fresh, undamaged hull every damageable Station sums to full.
  // These three own the destroyer's hull Systems (helm engines, weapons, power),
  // and `core` is the ownerless bucket — all report exactly 1.
  expect(byStation.helm).toBe(1);
  expect(byStation.tactical).toBe(1);
  expect(byStation.engineering).toBe(1);
  expect(byStation.core).toBe(1);

  // NO-DAMAGE-MODEL: Comms and Navigation own no damageable Systems on this
  // hull, so the host emits no health figure for them — the neutral state. A
  // client must therefore render their Hero Bar tab from this absence, never
  // from summed damage rows it is not entitled to hold (AC #2/#3).
  expect(typeof byStation.comms).not.toBe('number');
  expect(typeof byStation.navigation).not.toBe('number');

  await tactical.close();
});

// Issue #1101: the host publishes an authoritative per-Station importance stream
// in `SimState.snapshot.station_importance`, held apart from health, with two
// independent-lifecycle flags — one-off `unread` (cleared on visit) and
// continuing `critical` (cleared only on resolve). This asserts the wire
// projection reaches a client for the state a solo captain can trigger cheaply
// off-screen: a raised Red Alert is a continuing `critical` condition attributed
// to the ship-wide `core` bucket. It also pins AC3's immunity — a `StationVisited`
// clears only one-off unread events, so the critical mark survives a visit. The
// unread lifecycle and the simultaneous-health-and-importance case are pinned by
// the Rust projection tests (`server_app`, `station_importance`) and the vitest
// client tests (hero-bar / sim-state), which can drive an objective transition
// the smoke harness has no cheap lever for.
test('station importance — a Red Alert reaches a client as a continuing critical mark that survives a visit', async ({ context }) => {
  const serverPage = await bootDestroyerServer(context);
  const hostId = await readHostPeerId(serverPage);

  const captain = await createTestClient(context, hostId, { name: 'Captain' });
  await captain.send('SelectStation', { station: 'Captain' });
  await waitForStation(captain);
  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);

  // Raise Red Alert from the captain — a continuing critical condition, derived
  // host-side and attributed to the ship-wide core bucket.
  await captain.send('ControlSystem', {
    target: 'red-alert',
    payload: { type: 'SetRedAlert', data: { active: true } },
  });

  const importance = await lastStationImportance(captain);
  const core = importance.find((e) => e.station === 'core');
  expect(core, 'a raised Red Alert must publish a critical importance mark on core').toBeDefined();
  expect(core.critical).toBe(true);
  // Kept apart from a one-off unread event on the SAME wire entry (AC1).
  expect(core.unread).toBe(false);

  // AC3: visiting a Station clears ONLY one-off unread events; a continuing
  // critical condition is immune. Send StationVisited and confirm the critical
  // mark still arrives on a later broadcast.
  await captain.page.evaluate(() => { window.__messages = []; });
  await captain.send('StationVisited', { station: 'core' });
  const afterVisit = await lastStationImportance(captain);
  const coreAfter = afterVisit.find((e) => e.station === 'core');
  expect(coreAfter?.critical, 'a visit must not clear a continuing Red Alert').toBe(true);

  await captain.close();
});

test('comms — hosted tab: an unclaimable Comms resolves onto a seated Captain host', async ({ context }) => {
  const serverPage = await bootDestroyerServer(context);
  const hostId = await readHostPeerId(serverPage);

  // Comms is auxiliary now: no dedicated seat, so a player claims a real seat
  // (Captain) and Comms is reached as a Hero Bar tab. With only Captain held,
  // Comms resolves through its host_order (`["tactical", "engineering",
  // "captain", "helm"]`) past the two unheld seats to Captain — the first
  // held candidate. `SetReady`/solo start work exactly as before.
  const captain = await createTestClient(context, hostId, { name: 'Captain' });

  await captain.send('SelectStation', { station: 'Captain' });
  await waitForStation(captain);
  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);

  const host = await lastCommsHost(captain, 'comms');
  expect(host, 'station_hosts must carry a comms entry').not.toBeNull();
  expect(host.host).toBe('captain');
  // Comms authors only one rating everywhere it appears (no automated tier),
  // so a visiting host gets the same full "Std" surface.
  expect(host.rating).toBe('Std');

  // The resolved host is admitted to act: CommsState arrives on the first
  // InProgress tick and lands on the Captain-seated token hosting Comms.
  await captain.waitForMessage('CommsState', 8_000);

  await captain.close();
});

test('comms — visiting: a Tactical-seated player is admitted as the resolved Comms host and can act', async ({ context }) => {
  const serverPage = await bootDestroyerServer(context);
  const hostId = await readHostPeerId(serverPage);

  // Solo crew: only Tactical is claimed, so Comms (unheld) resolves through
  // its host_order (`["tactical", "engineering", "captain", "helm"]`)
  // straight to Tactical — the first compatible seat that is actually held.
  // `sessions.all_ready()` only requires every CONNECTED
  // player ready, not every station filled, so a solo Tactical officer can
  // start the game alone.
  const tactical = await createTestClient(context, hostId, { name: 'Tactical' });
  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);
  await tactical.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);

  const host = await lastCommsHost(tactical, 'comms');
  expect(host, 'station_hosts must carry a comms entry').not.toBeNull();
  expect(host.host).toBe('tactical');
  // Comms authors only one rating everywhere it appears (no automated tier),
  // so a visiting host gets the same full "Std" surface a direct holder does.
  expect(host.rating).toBe('Std');

  // Admission actually follows the resolved host, not just the station
  // roster: the Tactical-seated token can hail the in-range starbase and
  // receive the resulting CommsState, exactly as a direct Comms holder would
  // in comms.spec.js.
  const initialComms = await tactical.waitForMessage('CommsState', 8_000);
  const starbase = (initialComms?.data?.contacts ?? [])[0];
  expect(starbase, 'at least one contact must be present in initial CommsState').toBeDefined();

  await tactical.page.evaluate(() => {
    window.__messages = window.__messages.filter((m) => m.type !== 'CommsState');
  });
  await tactical.send('ControlSystem', {
    target: 'comms',
    payload: { type: 'Hail', data: { target_uuid: starbase.uuid } },
  });
  const hailComms = await tactical.waitForMessage('CommsState', 8_000);
  const messages = hailComms?.data?.messages ?? [];
  expect(messages.length, 'the visiting host must be able to hail and see the response thread').toBeGreaterThan(0);

  await tactical.close();
});

test('comms — AI-hosted: falls back to AI once every host_order seat is unheld', async ({ context }) => {
  const serverPage = await bootDestroyerServer(context);
  const hostId = await readHostPeerId(serverPage);

  // Same solo-Tactical start as the visiting case above, so Comms starts out
  // visiting Tactical...
  const tactical = await createTestClient(context, hostId, { name: 'Tactical' });
  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);
  await tactical.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);

  const visitingHost = await lastCommsHost(tactical, 'comms');
  expect(visitingHost?.host).toBe('tactical');

  // ...then releases Tactical mid-game. host_order = ["tactical",
  // "engineering", "captain", "helm"] is every crewable seat other than Comms,
  // so with Tactical released and nobody else ever having claimed a seat,
  // every candidate in the chain is unheld and resolve_visiting_station
  // (src/ship/coordination.rs) falls all the way through to `host: None`,
  // `rating: BACKFILL_RATING` ("Backfill") — the pure AI-operated verdict.
  await tactical.page.evaluate(() => {
    window.__messages = window.__messages.filter((m) => m.type !== 'SimState');
  });
  await tactical.send('ReleaseStation');

  await tactical.page.waitForFunction(
    () => (window.__messages || []).some(
      (m) => m.type === 'SimState'
        && (m.data.snapshot.station_hosts || []).some(
          (h) => h.station === 'comms' && h.host === undefined,
        ),
    ),
    { timeout: 8_000 },
  );

  const aiHost = await lastCommsHost(tactical, 'comms');
  expect(aiHost, 'station_hosts must still carry a comms entry once AI-operated').not.toBeNull();
  expect(aiHost.host ?? null).toBeNull();
  expect(aiHost.rating).toBe('Backfill');

  await tactical.close();
});

/** Wait until `comms`'s resolved host equals `expected` (a seat id, or null). */
async function waitForCommsHost(client, expected, timeout = 8_000) {
  await client.page.waitForFunction(
    (exp) => (window.__messages || []).some(
      (m) => m.type === 'SimState'
        && (m.data.snapshot.station_hosts || []).some(
          (h) => h.station === 'comms' && (h.host ?? null) === exp,
        ),
    ),
    expected,
    { timeout },
  );
}

// Issue #1099 AC3: a disconnect relocates a human-seeking Station to the next
// held seat while the vacated seat stays RECOVERABLE, and a reconnect returns
// the same Station state to its holder — all observed through station_hosts and
// live admission, never internal state. Comms is the destroyer's auxiliary
// human-seeking Station (host_order = ["tactical", "engineering", "captain",
// "helm"]), so a Tactical holder dropping off must move Comms down that order to
// the still-seated Captain, and a same-token reconnect must bring it back to
// Tactical.
test('comms — a disconnect relocates the visiting host, and a reconnect restores it', async ({ context }) => {
  const serverPage = await bootDestroyerServer(context);
  const hostId = await readHostPeerId(serverPage);

  const TAC_TOKEN = 'visiting-station-tactical';
  const tactical = await createTestClient(context, hostId, { token: TAC_TOKEN, name: 'Tactical' });
  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);

  const captain = await createTestClient(context, hostId, { name: 'Captain' });
  await captain.send('SelectStation', { station: 'Captain' });
  await waitForStation(captain);

  await tactical.send('SetReady', { ready: true });
  await captain.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);

  // Comms visits Tactical — the first held seat in host_order.
  let host = await lastCommsHost(tactical, 'comms');
  expect(host.host).toBe('tactical');

  // Tactical disconnects. Its seat is no longer directly held, so Comms
  // relocates down host_order past the unheld Engineering to the seated
  // Captain. Watched on Captain's own SimState stream so the assertion is
  // through an independent observer.
  await captain.page.evaluate(() => {
    window.__messages = window.__messages.filter((m) => m.type !== 'SimState');
  });
  await tactical.close();
  await waitForCommsHost(captain, 'captain');
  host = await lastCommsHost(captain, 'comms');
  expect(host.host, 'a disconnect relocates Comms to the next held seat').toBe('captain');

  // Tactical reconnects with the SAME token. Nobody claimed the seat in the
  // meantime, so it is restored and Comms returns to Tactical — the same
  // Station state back on its holder.
  const tacticalBack = await createTestClient(context, hostId, { token: TAC_TOKEN, name: 'Tactical' });
  await tacticalBack.waitForMessage('Welcome', 10_000);
  await waitForCommsHost(tacticalBack, 'tactical');
  host = await lastCommsHost(tacticalBack, 'comms');
  expect(host.host, 'a reconnect returns the visiting Station to its holder').toBe('tactical');

  // Admission follows the restored host: the reconnected Tactical token is
  // admitted as the live Comms surface, exactly as before the drop.
  await tacticalBack.waitForMessage('CommsState', 8_000);

  await captain.close();
  await tacticalBack.close();
});
