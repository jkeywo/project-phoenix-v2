# Six-peer runtime and transport matrix

The supported workload is four ship simulation peers, two separate GM simulation
peers, three active Station clients on each ship, and ordinary Backfill elsewhere.
The twelve clients must send commands during measurement; readiness counts alone
do not establish this workload. Both GM consoles must perform recorded actions.

## Repeatable protocol checks

Run `npx vitest run tests/client/fleet-matrix.test.js` from the checkout.
`tests/client/fleet-matrix-harness.js` exports the bounded six-peer cases for
subsequent fault injection. They exercise the shipped registry, compatibility
handshake, fleet role/slot admission, Station Rating publication, readiness,
automatic launch grant, and direct/relay route selection. The fallback case
withholds all four RTC offers and observes the real retry ladder reach relay.

These are protocol tests with socket and RTC stand-ins and virtual timers.
They are neither real-browser/native workload measurements nor observations of
mobile networks. Their Station clients are published readiness/Rating inputs;
they do not run twelve client documents or the simulations.

## Native ship membership

A built native host can join a selected ship to an existing fleet:

```powershell
phoenix-host --world assets/worlds/probe_fleet_six_peer.toml --client-dir dist --rendezvous http://127.0.0.1:8788 --origin http://localhost:8080 --fleet-code <fleet-code>
```

Use a different `--addr` for each native process. Start the local rendezvous with
`node scripts/rendezvous-dev-server.mjs --port 8788`. This serves the production
registry on loopback; it does not emulate a deployed service or a mobile network.
Native membership uses the same retained lobby bridge and fleet member protocol
as GM membership, with the `ship` role and selected hull. The native link is
WebSocket relay because this runtime has no WebRTC. Its existing GM landing
join retains the GM role. `--fleet-code` requires a selected world, built bundle,
service and Origin; it refuses `--solo`. Use an Ultralight-enabled native build
for the retained control surface.

## Runtime acceptance ledger

For each run retain revision, dirty diff (if any), complete content identity,
binary/bundle hashes, OS/CPU/RAM/GPU, native SDK and browser versions, all peer
roles, twelve Station clients, GM actions, seed, observed route per link,
impairment configuration, timestamps, and first-divergence/recovery artifacts.
An intended route is not an observed route. Native links should report relay;
a mixed session can contain direct browser links and native relay links.

| Runtime | Direct-capable links | Forced relay | Automatic fallback | Status |
| --- | --- | --- | --- | --- |
| Six browser peers | All host links | All host links | Block RTC, retain WebSocket | Not run |
| Six Windows native peers | Not available in native fleet transport | All host links | Native advertises relay immediately | Not run |
| Mixed browser/native | Browser links where negotiated | All host links | Browser RTC failure plus native relay | Not run |

Run each cell with LAN conditions, then reproducible delay/loss. Preserve the
profile and observed impairment counters beside results; use separate evidence
for ordinary internet, mobile hotspot and separate mobile networks. Do not call
a controlled impairment a real mobile observation. The one-hour mixed run and
shorter-run durations/limits remain governed by #1543 and #1090.

At this checkpoint, no real-runtime matrix cell is passed. Native ship admission
and bounded protocol coverage are prerequisites; the full runtime/workload
matrix remains outstanding for #1530.
