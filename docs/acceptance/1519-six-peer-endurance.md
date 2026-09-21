# Six-peer deterministic endurance gate

Issue #1519's automated workload runs four ship hosts, two stationless GM peers,
three active Station clients per ship and ordinary Backfill on every other
Station. Every peer simulates the same hostile NPC. The harness compares both
the complete authoritative digest and the NPC's admitted output every tick; it
does not assign NPC ownership or transport NPC decisions.

The test-only world is
`assets/worlds/probe_fleet_six_peer.toml`. It has no ending condition, so the
hour-long exercise does not weaken or replace any reference scenario's ending.
Its workload alternates Helm, Captain and Engineering input on all four ships
and canonical operations from both GM identities.

## Short deterministic gate

Run the ordinary-CI form with:

```powershell
$env:PHOENIX_AMBIGUITY_BASE_REF = '<verified pre-change SHA>'
cargo test --features headless --test lockstep_six_peer six_peers_keep_digest_and_npc_output_equal_under_twelve_clients_and_two_gms -- --exact
```

This is tick-bounded and runs as fast as the headless simulation permits. It is
the correctness gate, not evidence of one hour of real operation.

## One-hour acceptance workload

Run the ignored form on the exact release candidate:

```powershell
$env:PHOENIX_T5_ARTIFACT_DIR = 'C:\absolute\path\to\retained-evidence'
cargo test --features headless --test lockstep_six_peer six_peer_one_hour_endurance_workload -- --exact --ignored --nocapture
```

It defaults to 3,600 seconds and paces at 60 logical ticks per wall-clock
second. For a rehearsal only, set `PHOENIX_T5_ENDURANCE_SECONDS` to a smaller
positive integer. Do not report a rehearsal as the one-hour result.

Record the full Git SHA and dirty state, OS/hardware, Rust version, start/end
time, world and hull hashes, seed, peer/client composition, command exit code
and artifact directory. This native deterministic workload is not evidence for
the parent PRD's browser/native/device/network matrix; keep unrun cells open.

On the first digest or NPC-output mismatch the workload stops and writes
`six-peer-divergence.json`. The versioned JSON pins the first divergent tick,
Git revision, world/hull paths, seed, six peer roles, twelve client seats, every
peer's digest and NPC output, FNV-1a content hashes for the world and hull, plus
the human command log and canonical GM grants needed to replay the input window.
Retain the file with the run record. A failed artifact write is printed together
with the complete JSON so loss of the output path cannot disguise the divergence.
