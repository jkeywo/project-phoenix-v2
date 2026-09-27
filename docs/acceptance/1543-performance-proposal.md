# T5 performance acceptance proposal — awaiting measurements and ratification

This is the reusable plan for #1543. The user chose to decide the numerical
limits after seeing measurements. Nothing below is a ratified limit or a passing
runtime result. Four ship peers, two GM peers, twelve active Station clients and
two active GM consoles remain the workload; the one-hour mixed session remains
required. The protocol checks in `1530-six-peer-matrix.md` are separate evidence.

## Available machine provenance

Read from Windows CIM on 2026-09-27:

| Field | Observed value |
| --- | --- |
| OS | Windows 11 Home, 10.0.26200, build 26200 |
| CPU | Intel Core Ultra 9 275HX, 24 cores / 24 logical processors |
| Physical RAM | 68,112,736,256 bytes |
| GPU | NVIDIA GeForce RTX 5090 Laptop GPU, driver 32.0.16.1714 |
| Integrated GPU | Intel Graphics, driver 32.0.101.8628 |

This describes one development machine. It is not evidence for six independent
machines or for a minimum supported specification. Record the active renderer,
power mode, thermal state, display resolution, browser version and native SDK
version for every actual run; those were not measured by this inventory.

## Measured loopback protocol probe

`node scripts/fleet-relay-probe.mjs` ran on this machine with Node v24.13.0
at 2026-09-27T00:13:00Z. It launched the shipped registry on loopback, admitted
four ship-role and two GM-role protocol peers over real WebSocket relay links,
froze their roster, and timed 100 synthetic frame round trips using one monotonic
clock. Raw samples, observed routes, base revision, dirty-state flag and SHA-256
hashes of the exercised sources, runner hash and tracked source patch are in
`1543-loopback-protocol-probe.json`.

| Round-trip statistic | Observed milliseconds |
| --- | --- |
| p50 | 0.7128 |
| p95 | 1.1100 |
| p99 | 1.3328 |
| Maximum | 1.3481 |

This is transport-only evidence: zero simulations, zero Station client
documents, no rendering and no authoritative command application. The source
was the dirty scale worktree based on `ea9acfb316c80a52ccbf586d07c3a5ecdfa853e4`.
It is not a comparable supported-workload baseline, a network acceptance cell,
or evidence that the proposed command/stall/recovery limits pass.

## Proposed shorter runs and repeatable profiles

Run each applicable browser/native/mixed cell from the matrix for 2 minutes of
warm-up, then 10 measured minutes, three repetitions. Use the same composed
content, seed and command schedule for baseline and candidate. The mixed
one-hour endurance run follows the shorter runs; it is not replaced by them.
Record the authored CommandDelay and hold it fixed throughout each run. These
proposals do not authorize adaptive delay or tuning a failing run into a pass.

| Profile | Controlled added delay per direction | Random packet loss |
| --- | --- | --- |
| Local baseline | 0 ms | 0% |
| Delayed | 50 ms | 0% |
| Impaired | 100 ms, uniform jitter ±20 ms | 1% |

These are proposed test inputs, not descriptions of the user's LAN or mobile
networks. Record the shaping tool/version, seed, interface and observed counters.
WebSocket relay has TCP retransmission and head-of-line delay: packet loss must
be applied below the WebSocket, not simulated by discarding reliable game frames.
Keep ordinary internet, hotspot and separate mobile-network observations in
separate ledger rows. Loopback does not establish any of those rows.

## Proposed numerical limits for discussion

| Measurement | Local baseline | Delayed | Impaired |
| --- | --- | --- | --- |
| Command input to authoritative application, p95 | ≤250 ms | ≤450 ms | ≤750 ms |
| Command input to authoritative application, p99 | ≤500 ms | ≤900 ms | ≤1,500 ms |
| Unplanned stall time / measured duration | ≤0.5% | ≤1% | ≤3% |
| Longest unplanned stall outside fault injection | ≤1 s | ≤2 s | ≤5 s |
| One departed peer to survivor progress, p95 | ≤5 s | ≤8 s | ≤15 s |
| Deliberate divergence to verified recovery, p95 | ≤10 s | ≤15 s | ≤30 s |

All rows additionally require zero unexplained digest mismatches, zero duplicated
canonical actions and exactly one winner for each replacement-slot race. Numeric
values are starting proposals; measured distributions and identified causes must
be shown to the user before any acceptance decision. Do not hard-code these
values into product behavior or unrelated CI timing gates.

Measure input-to-application with a correlation ID and the originating clock;
do not subtract unsynchronized clocks on different devices. Track the application
tick and a correlated return observation, documenting whether the reported value
includes the return trip. Exclude deliberate GM pause from unplanned stalls,
but record both. For recovery, retain the loss/divergence detection, restore
commit and resumed-progress timestamps and digest/history evidence.

## Required evidence for a ratification request

For both comparable baseline and candidate, retain commit SHA plus dirty patch,
composed content ID/epoch, bundle and binary hashes, seed, runtime versions,
per-peer hardware and role, actual link routes, twelve active client counts and
their command totals, both GM consoles' actions, shaping provenance, raw samples,
p50/p95/p99/max summaries and first-divergence artifacts. Report changes against
baseline beside absolute values. A baseline from a different build profile,
content set or network profile is not a comparable baseline.

Current measurement ledger: **no supported-workload latency, stall, recovery or
one-hour runtime measurements captured**. Owner transport handover (#1534) and
the actual runtime matrix remain outstanding. User ratification remains pending.
