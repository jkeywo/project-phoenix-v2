# Falling Skyway current automated candidate (#1269 / #1044 / #1352)

8 September 2026, simulation revision `18013e6b`. No scenario tuning was performed.
The design digest was read first; its Falling Skyway hash was
`48f89f0f608431ea5c640a3fe1b0f8a5b90bfca6cd14b3edeab7e610b4f87d9f`.

The manual suite selected 94 cases and passed all 94, with zero failed or
ignored (110 filtered), exit 0, 1349.80 seconds at `RUST_TEST_THREADS=2`.
The exact command and named rescue/lift results are in the batch evidence file.

The current debug binary built by that suite then ran:

```
target/debug/phoenix-headless.exe --world assets/worlds/falling_skyway.toml --ship assets/entities/alliance_destroyer.toml --seed 1 --hz 30 --sim-seconds 1850 --report target/issue-1449/falling-skyway-seed1-final.json --report-format json --log off
```

Exit 0. Full result: [1352-seed1-final.json](1352-seed1-final.json).
This is a deterministic narrative run, not a release performance measurement.
The baseline is the preserved #1338/#1344 commentary in
`scripts/balance-runs.falling-skyway.toml`; its original raw JSON was not recovered.

| Measure | #1344 preserved | Current |
| --- | ---: | ---: |
| Final tick | 110163 | 110163 |
| Classification | reported | reported |
| Narrative events | 137 | 207 |
| Objectives posted / completed / failed | 25 / 22 / 3 | 33 / 22 / 7 |
| Comms opened / answered | 46 / 23 | 45 / 21 |
| Deadlines / beats | 13 / 0 | 12 / 8 |
| Report rows / hidden total | 1 / 6 | 9 / 28 |
| Marked spawned / rescued / destroyed | 1 / 1 / 1 | 2 / 1 / 1 |

The current census adds 35 task and 10 computer-message events. More recorded
events alone do not establish richer play. The original #1338 entry had 132
narrative events; #1344 added five recording events without changing its run shape.

## Current path and alternate coverage

- Negotiation settles the strike; labour is `negotiated`, survey `corroborated`.
  The eleven opening Comms choices still occur within roughly 0.13 seconds.
  Backfill refuses force; picket remains `unresolved`. The manual suite separately
  covers talking the picket away and authored force/report paths.
- Both debris objects strike their ladders, at 1322.47 and 1462.70 seconds;
  their scans fail. This run does not prove successful interception. The suite
  includes the authored corridor and ladder probes.
- Demolition sets charges at 338.03 seconds and detonates safely at 1312.93;
  the obstruction is destroyed and the final row is `safe`. This is a real
  mission path beyond the synthetic four-outcome report-row fixture. The latter
  still proves scoring, not all four full-mission executions.
- Lyra rescue completes at 433.30 seconds, with outcome `crew_saved`. Unlike
  the old account, Lyra has no subsequent marked destruction. This run's sole
  destruction belongs to the obstruction. Rescue/loss alternatives are covered
  by the separately selected suite, including the named pre-band rescue case.
- The first tether fails at 1292.10 seconds; the second tether and head complete
  at 1312.87. The failure deadline is cancelled and Skyhook is `held`, with the
  evacuation flag set. The old account fired failure alongside success.
- Backfill chooses berth-hold twice. Window readiness, window, berth and
  confrontation fail; demand and shortfall are both 66. Lifts is `none` and all
  three claimants remain. Alternate allocation and exhaustion are separate
  suite cases, not outcomes of this seed.
- Commitments finish `mixed`: two kept, safe passage broken.

## Station activity comparison

These are the bounded tail counters, **not whole-mission totals**. Current data
retains 33 fifteen-second buckets starting at tick 81000, including a partial
final bucket. All retained human/offline counts are zero. The historical values
were copied from #1338 to #1344, explicitly not remeasured there.

| Station | Preserved AI commands | Current AI commands |
| --- | ---: | ---: |
| Captain | 1 | 4861 |
| Comms | 6 | 2 |
| Engineering | 1 | 0 |
| Helm | 59373 | 42084 |
| Navigation | 2 | 0 |
| Tactical | 17575 | 0 |

Hull is now 390/390 versus historical 2527/2690; damage taken is 1422.043 versus
1511.0. This is not a controlled tuning-only comparison across identical hull
and runtime definitions. Command counts do not establish player engagement.

Independent read-only comparison completed. Competent human crew performance,
three-person doubling, per-console/per-act experience, and narrative acceptance
remain NOT RUN. The suite and this path do not replace those requirements.
