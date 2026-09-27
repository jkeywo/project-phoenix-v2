# Convoy route pacing check — 2026-09-27

The full-length route was checked after fixing a low-detail movement mismatch.
The shipped convoy has a 2,300-unit crossing, a hauler top speed of 12 units/s,
and a route fraction of 0.18. Its civilian override now declines impulse;
other civilian routes keep their existing default. The low-detail movement
path reads the active authored non-impulse directive's speed and decelerates
toward it on demotion. Impulse-enabled routes retain their prior low-detail
movement. The earlier build carried impulse velocity through low detail and
ended the same seeded four-escort run in 24.7 simulated seconds.

## Reproduction

Source world SHA-256:
`9A0BFF5930646FE8647AF8D21650ED19E8F6197C8AD63BDC6980274CF4186B92`

Built `target/debug/phoenix-headless.exe` SHA-256:
`85261E2BE84B5A0FD366FA33EE7F470793D72B251AAE152551F4764D88F3E898`

For a travel-only probe, copy the shipped world to an ignored temporary file,
replace only the first `unclaimed = "absent"` with `"backfill"`, and move the
first raider from `[-180.0, 0.0, -710.0]` to `[-20000.0, 0.0, -710.0]`.
The variant SHA-256 was
`F4852A0591EAF265ABAA97207BCB59DE07726768E89A2EA7578DC4D042F5A556`.
Run `phoenix-headless --world <variant> --sim-seconds 1200 --seed 1548
--report <result.json>`. All three transports reached safety; GameOver was
at tick 63,926, **1,065.4334 simulated seconds** (17m 45.4s). There is no
elapsed-time failure.

For a four-escort Backfill probe, replace all four absent settings with
`"backfill"` and leave the raider untouched. That variant SHA-256 was
`D92F2B14713DCCAE8E0344AB31BB73BE9CDD7EA40DFE3FE7E9EF2DABC6A9C4A2`.
At seed 1548, the first raider destroyed all three unprotected transports
by tick 7,835, **130.5833 simulated seconds**. The report gave the authored
zero-saved convoy row, a separate completed recovery row, and a separate
escaped-pursuit row. This is a Backfill-only combat outcome, not a measured
human-crew normal-run duration. Playable combat pacing and presentation still
need the human acceptance run.

Both probes used the same built binary. Their raw summaries are retained as
[`1551-convoy-travel-result.json`](1551-convoy-travel-result.json) and
[`1551-convoy-combat-result.json`](1551-convoy-combat-result.json). The
temporary worlds and run logs are ignored local files under
`target/convoy-probe/`; this recipe and hashes make the source/content
distinction explicit.
