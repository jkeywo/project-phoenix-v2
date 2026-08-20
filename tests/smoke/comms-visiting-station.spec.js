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
