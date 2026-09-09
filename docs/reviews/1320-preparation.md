# GM two-ship candidate preparation (#1320 / #1323)

For #1449 E1, 8 September 2026. Both `node scripts/prepare-gm-live-event.mjs`
and `node scripts/prepare-gm-live-event.mjs --check` exited 0.

The exact ignored runtime precheck was explicitly selected:

```powershell
cargo test --features headless --test gm_live_event_precheck prepared_event_has_two_authored_fleet_hulls_and_equal_peer_digests -- --ignored --exact --nocapture
```

**1 passed, 0 failed, 0 ignored, 0 filtered**, exit 0, runtime 0.92 seconds.
The original run was after #1248 and before subsequent mechanical/browser-edge
changes. The exact test was run again on integrated main `1f8fc0e6`:
**1 passed, 0 failed, 0 ignored, 0 filtered**, exit 0, runtime **1.06s**.
`1320-precheck.json` now retains that final run, with the same digest and topology.

The `GM_LIVE_EVENT_PRECHECK` result reported schema 1, seed 1320, initial tick 1,
90 compared rounds, and final tick 90 / digest `613cccc16d5570e0` on both peers.
Both loaded the hail AST. The two authored hulls were:

| Slot | UUID | Position | Local projection |
| --- | --- | --- | --- |
| 1 | `00000000-0000-8000-8000-000100000007` | `[400, 0, 200]` | Peer 1 |
| 2 | `00000000-0000-8000-8000-000100000008` | `[400, 0, 250]` | Peer 2 |

Each peer reported both spawns at those positions, with its own distinct
`local_ship` and `local_slot`. Logger-already-set and unresolved patrol-anchor
warnings appeared; the precheck's assertions passed.

SHA-256 provenance:

| File | Hash |
| --- | --- |
| `assets/worlds/combat_test.toml` | `db0b71e9eb31297faf6841dcdb264e911b565195fb5e0770f650f88d11dc8eec` |
| `docs/acceptance/fixtures/1320-second-ship.toml` | `cfaa204b0d5d67557e297876209f5c9e7c0b1b7b68c43d96fa2e18cc38a8e0b0` |
| `assets/worlds/prepared/gm_live_event_two_ship.toml` | `53a0a2632d50d54ea1c33c11c1ae0314fbf7904626fce3bb0b4a72d53abf0bf3` |
| `assets/scenarios.gm-live-event.toml` | `06cb0479d3cbe5cdb3479f3697f621b61feff5869896d1e0d61a935d7291a126` |

The generated world and manifest in the served `dist/assets/` were hashed and
matched those source hashes. That bundle is
`project-phoenix-5f8fb494bdf5d61a`, built for the browser edge validation.
Automated fleet-lobby smoke on that bundle passed five cases, including two
host assembly, separated crew stars, typed-code refusal, compatibility refusal,
closed admission and frozen slot roster. It uses the real rendezvous registry
with transport shims; it is not an actual network or human-session result.

**NOT RUN:** ordinary human join/claim/Comms on both prepared Fleet hulls and
the #1320/#1323 acceptance sessions. They are not marked ready or accepted by
this supporting precheck. #1273 remains dependent on accepted #1323; #930 retains
its later milestone scope.

## Prepared browser topology and Comms receipt

The opt-in `gm-page.spec.js` case **prepared GM event admits crew and delivers
Comms to both live Fleet hulls** passed: **1 passed, 0 failed**, exit 0, 26.3s
(31.0s including setup), Chromium on port 3171, `PHOENIX_GM_PRECHECK=1`.
It ran on bundle `project-phoenix-5f8fb494bdf5d61a` with the generated world above,
its authored seed 475, and explicit Alliance Cruiser selections on both hosts.
This differs from the native precheck's explicit seed 1320.

Two ordinary crew clients joined their own host stars and received token-specific
`comms` station assignments. Crew ready commands and the GM's ready control
started all three peers. Both crews received the fixture's WorldSetup; the GM
reported two distinct recipient IDs in Fleet slots 1 and 2. The actual authored
`starbase-selected` control delivered `Prepared Fleet Comms receipt` to both
crew CommsState inboxes, and each recipient ID matched that crew's own Fleet
slot. The test attaches its concrete slots, recipients and deliveries as JSON.
Independent read-only review passed after adding world and per-slot assertions.
The receipt-persistence rerun on port 3172 also passed (15.4s / 19.1s total).
`1320-browser-precheck.json` retains its matching source hash and concrete IDs:
`00000000-0000-8000-8000-000800000000` for slot 1 and
`00000000-0000-8000-8000-000800000001` for slot 2, each delivered to its own crew.

Earlier attempts exposed test setup errors: the ship picker needed an explicit
hull; destroyer Comms is auxiliary and cannot be selected as an ordinary seat;
the production route is `starbase-selected`, not the probe world's `selected`.
Those were corrected without changing production readiness or routing.
The transport fixture still substitutes the network; this automated result does
not claim ordinary human operation, a real network session, or two human GMs.
