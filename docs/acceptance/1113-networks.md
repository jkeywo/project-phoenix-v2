# Acceptance kit — issue #1113, connecting across real networks

**This is the human half of issue #1113.** The automated half is done and is
listed at the bottom; it proves the transport ladder works, that the fallback
does not fork the game protocol, and that the diagnostics say the right things.
None of it can prove the only question that matters here: **what a real network
does to real traffic.** A CI runner has no captive portal, no CGNAT, no hotspot
that blocks UDP, and no coffee shop that permits nothing but HTTPS.

So this is a script for four sessions with real devices on real networks. Each
one is a handful of minutes. Work through them in order; each is a step further
from the easy case.

---

## 0. Preconditions — do these first, in this order

- [ ] **§3a of `docs/delivery-checklist.md` is complete.** The rendezvous
      workers are deployed and their `ALLOWED_ORIGIN` lists include the origin
      you will be testing from. Until this is done there is **no join path at
      all** — not a degraded one — and every scenario below fails identically
      and uninformatively.
- [ ] **§3 of the same file is complete.** The TURN credential workers are
      deployed with at least one working credential source. Without it every
      scenario past the first one fails, and scenario 3 cannot run at all.
- [ ] **Run the deploy check.** This is §3/§3a's curl recipe as code, and it
      catches the invisible-drift failures before a room full of people meets
      them:

      ```
      node scripts/check-rendezvous.mjs \
        --rendezvous https://phoenix-rendezvous.project-phoenix.workers.dev \
        --turn       https://phoenix-turn-credentials.project-phoenix.workers.dev \
        --origin     https://pp-dev.kiwigamedesign.co.uk
      ```

      Exit 0 means both contracts hold. **Do not start a session on a non-zero
      exit** — every one of its findings is a failure you would otherwise spend
      the session rediscovering, and the two `origin` ones are exactly the
      2026-08 outage.
- [ ] **Two devices minimum**, plus whatever a scenario names. A laptop for the
      host (the viewscreen), phones for the crew.

---

## How to record a result

Every scenario ends the same way: **copy the diagnostics dump from both ends and
paste both into the issue thread** ([#1113]).

* On the **host** (viewscreen), the dump button sits under the join code —
  labelled "Copy diagnostics".
* On a **phone**, it is in the bottom-right corner of the join screen.

The dump is plain text and carries no join code, no session token and no full
peer id, so it is safe to paste in public. It looks like this:

```
phoenix connection diagnostics
  page         client
  when         2026-08-28T09:14:02.881Z
  network      hotel wifi, 2.4GHz
  transport    ws-relay
  why          direct-exhausted
  relay src    worker
  relay probe  reachable
  attempt      5
  ice          timeout
  candidates   host, srflx
  timeout      at attempt 4
```

If a scenario fails, paste the dumps **anyway** — a failure dump is the whole
point of the exercise, and it is worth more than a passing one.

---

## Scenario 1 — Same LAN *(gate)*

The easy case, and the control for everything after it. If this does not work,
nothing else will and the problem is not the network.

**Setup.** Host laptop and two phones on the same Wi-Fi.

**Steps.**
1. Open the host page and wait for the five-letter code.
2. On phone A, scan the QR.
3. On phone B, type the five letters instead.
4. Claim a station on each and start a mission.

**The diagnostics must show.**
* Host: nothing at all under the join code. Silence is the healthy state — no
  relay warning, no per-client ICE line, no "carried by the join service".
* Phones: a `route:` line naming a **host → host** pair (both devices are on the
  same subnet, so ICE should pick direct candidates and never touch a relay).

**Pass when.** Both phones reach a console, both can drive their station, and
neither dump says `transport ws-relay`.

**Fails as.** If a phone lands on `ws-relay` here, the LAN is doing client
isolation (common on guest and hotel networks) — note that in the thread,
because it changes what scenario 4 is testing.

---

## Scenario 2 — Several devices on one mobile hotspot *(gate)*

Three or more devices behind one phone's NAT. The case that used to fail, and
the reason the TURN worker exists.

**Setup.** One phone sharing its mobile connection; the host laptop and **at
least two other phones** joined to that hotspot. The sharing phone can be one of
the crew as well.

**Steps.**
1. Everything joins the hotspot's Wi-Fi *first*, before the host page is opened.
2. Open the host page, wait for the code, join every phone.
3. Start a mission and fly for a minute — this is a latency check as much as a
   connection one.

**The diagnostics must show.**
* Host: **no** "Using free fallback relay" notice. If you see it, the credential
  worker is unreachable and §3 is not actually finished — the session will
  "work" on a shared public relay and tell you nothing.
* Phones: `relay src` reads `worker` in the dump. `route:` may legitimately be
  `srflx → srflx` (the hotspot's NAT was traversable) or `relay → …`; either is
  a pass, and which one it is is worth recording.

**Pass when.** Every device reaches a console and stays connected for the whole
minute, with no station flipping to Backfill.

---

## Scenario 3 — Forced relay *(gate)*

Proves the TURN path actually works, rather than being merely configured. This
is the one that cannot be inferred: on a friendly network ICE picks a direct
pair and the relay is never exercised, so it can be broken for months without
anyone noticing — until the one session where it is the only path.

**Setup.** Any network. Simplest at a desk.

**Steps.**
1. Open the host page with the lever on:
   `https://pp-dev.kiwigamedesign.co.uk/?forceRelay=1`
2. Join a phone with the same lever. Either append it to the QR link's query —
   `…/client/index.html?forceRelay=1#<code>` — or open
   `…/client/?forceRelay=1` and type the five letters.
3. Claim a station.

**Why both ends.** ICE only negotiates a relayed pair when **both** ends offer
relay candidates. Pinning only the phone proves nothing: a host candidate from
the other side would still win, and the session would pass while testing the
ordinary path.

**The diagnostics must show.**
* Both ends: "Transport pinned to turn by this link". If that line is absent,
  the lever did not take and the run is invalid — check the URL.
* Phone: `route:` naming **relay** on at least the local side, and a `via`
  naming a `turn:`/`turns:` address. **Record which address** — that is how you
  tell the dedicated credential worker from the free shared fallback without
  trusting a config field.

**Pass when.** The phone reaches a console over a relayed pair and can drive its
station.

**Fails as.** A phone that sits on "attempt 1 — ICE: checking" and times out
means no TURN allocation was possible. That is a §3 problem (credentials, or the
CORS allowlist), not a network one — and it is precisely the failure this
scenario exists to surface before a real session.

---

## Scenario 4 — Public Wi-Fi *(gate)*

The hostile case: a café, a hotel, a library, an airport. Captive portals,
client isolation, blocked UDP, deep-packet inspection that permits little beyond
HTTPS. This is the scenario the WebSocket relay was built for.

**Setup.** Host laptop and one phone on the same public network. Accept the
captive portal on **both** devices before starting.

**Steps.**
1. Open the host page. If no code appears within ten seconds, the network is
   blocking the rendezvous socket itself — record that; it is the one failure
   nothing in the product can route around.
2. Join the phone by typing the code.
3. Wait. **This one is slow on purpose**: the phone tries the direct ladder for
   up to about ninety seconds (four attempts, 8 s → 16 s → 30 s → 30 s) before
   falling back. Do not intervene; the wait is the measurement.
4. Once connected, claim a station and fly for a minute.

**The diagnostics must show.**
* Phone: either a working direct/relayed pair (the network was friendlier than
  expected — record that), **or** the fallback, reading
  "No direct link on this network — the join service is carrying the game" with
  `why direct-exhausted` in the dump.
* Host: "client …: no direct link — carried by the join service" for that phone.

**Pass when.** The phone reaches a console by *some* route and stays connected
for the minute. Higher latency is expected and is not a failure — note roughly
how it felt.

**Also record.** How long the fallback took end to end, from typing the code to
reaching a console. If that wait turns out to be unbearable in the field, the
fix is a shorter direct ladder before escalation, and this number is the
evidence for changing it.

**Shortcut, for a second run only.** `?transport=ws-relay` skips the direct
ladder and goes straight to the fallback. Useful for checking that the relay
carries a *mission* acceptably on this network without waiting out the ladder
again — but a run using it does **not** satisfy this scenario, because the
automatic escalation is half of what is being tested.

---

## Scenario 5 — Separate mobile networks *(log when the opportunity arises; NOT a gate)*

Host on one carrier, phone on another, no shared network at all. Two CGNATs with
nothing between them — the case a relay is strictly required for.

This is **not** a gate on #1113: it needs two people, two carriers and no Wi-Fi,
which is a scheduling problem rather than an engineering one, and scenarios 2
and 3 between them already exercise both mechanisms it depends on.

If the opportunity arises, run it and paste the dumps in the thread. Note the
carriers.

---

## What the automated half already covers

So the sessions above are read as the remaining gap rather than as the whole
test, and so nobody re-runs by hand what is already gated.

| Path | Covered by | What it honestly proves |
| --- | --- | --- |
| Direct WebRTC | `tests/smoke/transport-paths.spec.js` | End to end in a real browser: both channels negotiate, the lossy one really is unordered and non-retransmitting, and neither readout mentions a relay |
| TURN-only | same, plus `tests/client/transport-levers.test.js` | That `?forceRelay` reaches **both** peer connections' `iceTransportPolicy` and is named on both readouts. **Shim-level**: the smoke suite's fake peer connection has no ICE to restrict, so that a real TURN allocation succeeds is scenario 3's and nothing else's |
| Signalling reconnect | `tests/smoke/transport-paths.spec.js`, `tests/client/rendezvous-transport.test.js` | A dropped link really re-resolves the same code, re-sends `Identify` with the same token, and reports each attempt on the readout |
| WebSocket game relay | `tests/client/rendezvous-relay.test.js`, `tests/client/rendezvous-relay-channels.test.js`, `tests/smoke/transport-paths.spec.js` | The service's bounded mailbox (snapshot sheds oldest-first, reliable never sheds), and a whole session carried over the relay through the real registry — same admission gate, same delivery classes, same `Identify` |
| Automatic escalation | `tests/client/rendezvous-transport.test.js` | Four direct attempts, then the fallback, then a working session — under fake timers, because at real speed it is the ninety seconds scenario 4 spends |
| Native host crew path | `src/native_host/relay_transport_tests.rs`, `tests/native_relay_protocol.rs`, `tests/native_relay_live.rs` | The protocol against a fake socket; the Rust and JavaScript vocabularies pinned together as text; and — `#[ignore]`d, with a local service — a real socket end to end |
| Deployed worker config | `scripts/check-rendezvous.mjs`, `tests/client/rendezvous-checks.test.js` | TLS, the origin allowlist in both directions, the protocol revision, the upgrade gate, and the TURN worker's CORS echo and relay contents |

### The one gap the automation genuinely leaves

**A browser client joining a NATIVE host** (`phoenix-host --world … --rendezvous
…`). Each half is covered — the browser half against a browser host in the smoke
suite, the native half against a real socket in `tests/native_relay_live.rs` —
but nothing runs all three at once, because it needs a browser, a GPU window and
a service simultaneously and no CI runner here has any of them.

It is a five-minute manual check, and worth doing once alongside scenario 1:

```
# Terminal 1 — the service, locally, no Cloudflare account needed.
node scripts/rendezvous-dev-server.mjs --port 8788

# Terminal 2 — a native host, registering with it.
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --world assets/worlds/combat_test.toml \
  --client-dir dist --rendezvous http://127.0.0.1:8788 --origin http://localhost:3000

# Terminal 3 — serve the built client.
npx serve dist -p 3000
```

The native host prints `crew join code XXXXX` at startup. Open
`http://localhost:3000/client/?rendezvous=http://localhost:8788` and type those
five letters. (`?rendezvous=` is honoured for loopback origins only, which is
what makes it safe to ship.) The phone should reach a console **without waiting
out the direct ladder at all** — a native host advertises that the relay is the
only way in, and the joiner skips straight to it.

[#1113]: https://github.com/jkeywo/project-phoenix-v2/issues/1113
