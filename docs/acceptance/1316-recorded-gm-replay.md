# Recorded GM continuation for the M2 evidence runner

This native verifier takes two ordinary browser `StoredRun` RON exports from
one ship host. It restores the initial world and replays the recorded GM
requests to the final tick and applied-action frontier. It compares the actual
terminal results and authoritative digest with the final export. The final
world state and final outcomes are never installed into the simulation.

This is the bounded replay leg of #1316. It does not complete the two-GM browser
exercise or the human acceptance session in #1320.

## Recording contract

1. Start the chosen scenario and seed with the final fleet roster, hulls,
   immutable crew Stations/ratings, and GM identities. Select all crew seats
   and ratings before mission start.
2. During `InProgress`, create and export an ordinary named manual save from
   one ship host. Retain the complete downloaded RON file.
3. Continue using only typed GM actions. Keep the same participants, owner,
   crew assignments and ratings. Do not issue ordinary crew commands, reconnect
   peers, change recovery generations, or return to the lobby during this leg.
4. Export that host's existing rolling GameOver autosave after the scenario
   ending. A later named manual capture in `InProgress` is also supported for
   a shorter probe. Use the two complete files as the verifier inputs.
5. Retain browser evidence witnessing the input and crew restrictions. Ordinary
   save exports omit the ordinary command log and seat/rating history. Empty
   exported commands therefore cannot independently establish that nobody
   operated a crew console or briefly changed a seat. The verifier states this
   limitation in every successful report.

The verifier requires current snapshot format and rules, the same scenario,
seed and content version in both records, and an exact match with the loaded
native content ledger. Boot identity must retain the same fleet topology,
hulls, local ship, GM identities and authored GameStart UUID mapping. The final
journal must extend every initial grant and actual applied outcome in canonical
order; additional grants cannot precede the origin. Recovery history and the
initial pause baseline must remain unchanged. The initial phase must be
`InProgress`; the final phase may be `InProgress` or `GameOver`, no more than
1,000,000 logical ticks later. Unsupported or incompatible inputs are refused.

Simulation rules `0.5` require shared collision history on every host. Its
fixed-tick collector commits attribution before peer digest exchange and save
capture, independently of optional headless report telemetry. Restoring a save
replaces the recorded history and forgets unread bootstrap inputs for each
reader; it preserves later entry events and ordinary continuation. Collision
rows retain their existing snapshot format `32` fields and fold order. Rules
`0.4` recordings remain unmodified evidence and are refused; this correction
requires fresh ordinary browser exports after the new native and WASM builds.

Content identity includes the uncurated world's declared playable hulls even
when this crew selects another hull, and the root's compiled script record.
Local catalogue Start and portable import check full compatibility before
`wasm_init`. Both first declare the loaded root's `#scripts` ledger record
from the lifted source set, using the same sorted-source hash as compilation.
This does not compile or activate scripts, or freeze the ledger early.

The browser seals its preload ledger after root-script compilation in Startup,
before either spawn pass. Native boot records the declared hulls and compiled
scripts before its own freeze. Recordings made before this boundary correction
remain content-incompatible: retain the refusal and make fresh ordinary browser
exports. Never replace the recorded version or relax the verifier's content gate.

The headless boot uses the complete frozen fleet. It seeds the local Sessions
from that ship's recorded crew, and the ordinary fleet boot uses the same crew
rows for the other ships. It preserves the real selected ratings and human
control ownership; it does not apply the fresh-session save-load operation
that deliberately returns old crew seats to Backfill. No network mesh is
started. Saved authored UUIDs, layer reconciliation, roster readiness and
snapshot restore use their ordinary production paths. The exact initial digest
must match before any new GM request is appended.

## Native test driver

Build and run the focused integration binary from the repository root with the
normal `headless` feature and coordinated Cargo slot:

```text
cargo test --features headless --test recorded_gm_exports
```

Its native regressions make a real first recording with a human-held Station,
manual capture, typed requests, and the ordinary GameOver autosave. A real
Captain red-alert takeover/command/release checks the actual consumer state,
settled feedback and preserved human ownership. The tests cover exact origin
and continuation, independently derived outcomes, and refusal of
changed crew/recovery history or a false origin digest. The file-driven browser
test is intentionally ignored in this command because its inputs come from the
separate real browser exercise.

The Node collector calls the already-built `recorded_gm_exports` test executable
with these arguments, from the repository root:

```text
--ignored --exact verify_browser_gm_exports --nocapture
```

Supply these environment variables through the process-spawn API:

| Variable | Value |
| --- | --- |
| `PHOENIX_GM_RECORDING_INITIAL` | Path to the initial manual-save RON file |
| `PHOENIX_GM_RECORDING_FINAL` | Path to the final save RON file |
| `PHOENIX_GM_RECORDING_REPORT` | Writable path for the JSON result |

Use a new output path for each invocation. Retain stdout/stderr and the true
process exit; a stale file or a partially launched test is not passing evidence.
Missing files/environment or output I/O errors fail the process. A verification
refusal writes a structured failure report and then exits nonzero.

Success has this shape (the values below only illustrate the schema):

```json
{
  "pass": true,
  "report": {
    "scope": "GM-only requests; immutable recorded crew; absence of omitted human input requires collector evidence",
    "initial_tick": 100,
    "final_tick": 200,
    "initial_digest": "123",
    "expected_final_digest": "456",
    "actual_final_digest": "456",
    "initial_applied_actions": 1,
    "final_applied_actions": 4,
    "results_match": true,
    "frozen_crew": 1
  }
}
```

Digests are decimal strings so JavaScript retains all 64 bits. A refusal is
`{"pass":false,"error":{"stage":"...","detail":"..."}}`.

The public native API is
[`headless::replay::recorded_gm::replay_exports`](../../src/headless/replay/recorded_gm.rs).
It returns the report and the actual continued `PhoenixSim` on success. The
file adapter lives in [`tests/recorded_gm_exports.rs`](../../tests/recorded_gm_exports.rs).
There is no new shipped host flag or browser mutation/export hook.
