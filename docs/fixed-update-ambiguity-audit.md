# FixedUpdate ambiguity audit

Issue #1400's first slice measures unordered conflicting access in the ordinary
headless FixedUpdate graph. It does not change execution, RNG streams, minting,
or declare that conflicting systems commute.

`src/headless/determinism_audit.rs` finishes and cleans up ready plugins before
initializing the schedule, without running Startup or a simulation frame.
This includes systems registered by ordinary plugin finish hooks. Unready
plugins are refused without a readiness wait; already cleaned Apps retain their
current lifecycle state. Graph capture still refuses an initialized schedule
before a finish hook could dirty it. The `inspection_lifecycle` regressions
exercise actual late system/resource access and once-only hooks.
It resolves Bevy's conflict records to full system and component/resource names,
sorts each pair and the output, and retains repeated same-named instances.
Exclusive World access has an explicit marker. Existing Bevy ambiguity
exemptions remain visible through the source that declares them; this helper
adds none.

`tests/fixed_update_ambiguities.rs` is its own native test binary. It checks the
live graph against `tests/fixtures/determinism/fixed-update-ambiguities.json`
with one-to-one access-subset matching, then checks that ledger against the
trusted pre-change Git version. Removing a resource from a conflict is allowed;
adding a pair, access or repeated instance is not. Each previous row covers at
most one current instance. Augmenting paths handle overlapping subsets without
a greedy false failure. Exclusive World access can shrink to named access,
but named access cannot authorize exclusive World access. Once the ledger is empty it also rebuilds with Bevy's strict ambiguity
error setting. Small real schedules test conflict detection, explicit order,
exclusive access and multiplicity. An inert registration must leave the actual
production census and pre-run authoritative digest unchanged; the existing
`registration_order_determinism` binary retains the long-run digest guard.

## Running the gate

Set `PHOENIX_AMBIGUITY_BASE_REF` to the actual pre-change commit, such as the PR
base or local integration base, before running:

```text
cargo test --features headless --test fixed_update_ambiguities
```

The commit must already exist locally. The test performs no network operation,
does not fall back to comparing HEAD with itself, and fails if the trusted
ledger cannot be read. CI fetches history during checkout and uses the PR base
SHA or push-before SHA. Scheduled and manual runs use the checked-out commit's
first parent. A missing or invalid parent/base is a failure, not permission to
replace the ledger.

## First introduction and capture

The first ledger is the reviewed 1,968-row capture from the original census
boundary. The independently repeated pre-declaration capture is byte-identical
(SHA256 `49312c588121cb38f9a1b203a34b01e2e9936dae36380af5f55b2c0b052236fe`).
The later honest access declarations expose 1,973 rows: five new pairs and 17
broader access vectors. That later output is diagnostic debt, not a replacement
allowance. The final live graph must fit the original allowance after resolution.
The explicit capture test prints real records and never writes or approves a file:

```text
cargo test --features headless --test fixed_update_ambiguities print_fixed_update_census -- --ignored --nocapture
```

Record each capture's exact graph revision, features, command and result with
the implementation evidence. The ordinary independent review approved the
original capture. When the trusted base lacks
the ledger, the gate requires complete local history and checks every reachable
history branch for that path. Any prior occurrence fails as deletion followed
by reintroduction; only a path never present in trusted history is a first
introduction. That branch checks the independent original capture in
`tests/fixtures/determinism/fixed-update-ambiguities.initial.json` against its
reviewed first count and fingerprint, then bounds the current allowance with
the same access-subset/multiplicity law. A PR base predating the entire batch
cannot ratchet against an earlier commit inside the same PR.
`tests/fixed_update_ambiguities/bootstrap.rs` encodes the initial capture's
parsed rows with the production `Ambiguity` serializer through
`serde_json::to_vec`: compact UTF-8, no BOM or trailing newline, retaining every
row and access entry. Git's read-only `hash-object --stdin` must report blob
`a3a15b158431852c9b3c3bf890dbe8929e9e02ec` for those 437,412 bytes. It writes no
object, applies no clean filter and performs no network operation. Pretty-print
whitespace and checkout line endings therefore do not affect the check.

Keep the independent 1,968-row initial capture unchanged. The current allowance
may shrink within the same introduction: its rows, access and instance quotas
must remain covered by that capture, and every live conflict must still fit the
current allowance. Replacing both JSON files cannot authorize growth because
the original fingerprint is checked first. An empty current allowance requires
zero live debt and the strict Bevy check; an empty initial capture fails its pin.
Once the trusted base contains a ledger, that trusted version remains the bound.
No repository variable or external approval setting is needed. A missing current
ledger fails, and complete-history and no-delete checks remain mandatory.

The initial ledger retains the original duplicate record as well as complete
vectors. The immutable 17-expansion regression fixture tests every newly exposed
RNG/mint access against its prior vector; fixing only the five new pairs cannot
satisfy this gate. Bootstrap tests also cover changed access, extra instances,
same-count duplicate substitution and equivalent JSON whitespace. The original
native capture and bootstrap tests passed. The post-resolution live gate passed
with 1,929 rows, no added rows and 39 removals from the original. This slice alone does
not complete #1400's schedule, RNG, mint or default-pool equivalence work.

## Instance graph diagnostics (#1400)

The separate `print_fixed_update_instance_graph` ignored test prints marked JSON
from a fresh inspection App. `src/headless/determinism_audit/graph.rs` retains
capture-local SystemKey/SetKey identities and full names, so repeated registrations
remain distinct. It includes authored dependencies, nested membership, full
instance-specific conflicts, and flattened dependencies after the existing build
passes, including real ApplyDeferred nodes. An appended observer reads only; it
adds no edges or systems. Bevy subsequently removes redundant edges, preserving
reachability. Executable list order is not reported as a dependency.

An initialized schedule is refused rather than changed to force a rebuild. Capture
initializes systems but never executes them. Instance IDs are not portable ledger
keys. This diagnostic neither blesses debt nor replaces the name/multiplicity gate.

Focused commands (both structural tests and the actual capture have passed;
coordinate the Cargo slot for future executions):

```powershell
cargo test --locked --features headless --test fixed_update_ambiguities -- instance_graph_ --nocapture
cargo test --locked --features headless --test fixed_update_ambiguities -- print_fixed_update_instance_graph --ignored --exact --nocapture
```

Require respectively two and one passing tests. Use the integrator's guarded
runner with jobs 6, exact source manifests, fresh own-library/dep-info provenance,
frozen executable/PDB, unique logs and true exits. Preserve the original 1,968-row
and declared 1,973-row captures. Follow actual effective paths to explain torpedo
fire/lifecycle and inspect both directions for each newly visible pair; shared
successors and observed execution order are not proof of precedence. No ledger,
exemption or production order follows automatically from this output.

## Verified resolution boundary

The initialized post-split graph reports 1,929 rows. Full name/access/multiplicity comparison finds zero added rows and 39 removed rows against the original 1,968 capture. The real Rust bootstrap serializer produces the pinned fingerprint, and the ordinary live-subset/history gate passes. The independent initial capture retains the original fingerprint while the current allowance may shrink under the checked subset law. The later bootstrap correction passed six focused tests and the actual live/history gate against the unchanged pre-introduction local main in the SDK-enabled configuration; final combined validation remains separate. Typed stream/namespace ownership and the baseline-preserving Projectile edge carry their own restore and default/pinned behavior tests. Final integration remains the integrator's responsibility.

## Scoped Tactical commutativity

The ordinary source-bound Tactical baseline ran retarget, lock-clear and real
target-death/reacquisition in two fresh 16-worker default processes and one
single-worker pinned process per case. All nine runs agreed on 92 gameplay
observations and digests, ticks 4–95. Retarget and death runs actually differed
in raw cross-target command append order; every per-target subsequence agreed.
Victim A died at tick 24 and the real AI selector reacquired B at tick 25. This
is bounded gameplay evidence, not a claim that every weapon conflict commutes.

`WeaponsPlugin` marks only the following five Input pairs with Bevy's
`ambiguous_with`. No component/resource access or production order is changed.

| Pair | Complete shared access |
|---|---|
| Target selector / phaser decider | AdmittedCommands, ShipSystemBlackboards |
| Target selector / blaster decider | AdmittedCommands, ShipSystemBlackboards |
| Target applier / phaser decider | AdmittedCommands |
| Target applier / blaster decider | AdmittedCommands |
| Phaser decider / blaster decider | AdmittedCommands |

The selector writes Weapons intent and emits SetTarget. The sole target applier
remains after selection. Fire deciders read the previous PublishAggregate
Viewscreen combat lock; their authored policies receive seeded readiness,
posture/frequency and read-only scenario flags, not the new Weapons intent.
AI admission appends without allocating a logged sequence. Radar, phaser and
blaster consumers retain their own command subsequences and act on distinct
typed payloads/fine-system identities. None of this excuses other control,
blackboard, damage, recoil or projectile writers.

Bevy 0.18.1 suppresses only the selected ambiguity diagnostics; its executor
still computes access incompatibility and serializes the shared Vec/map access.
`tests/tactical_target_ordering/order_proof.rs` checks unique actual instances,
the complete five raw access vectors, unchanged incompatibility and preservation
of all external incident conflict vectors and multiplicities. New overlapping
access fails this proof even though the pair annotation would otherwise hide it.
Two opposed full-App orders reverse all five relations while retaining
selection-before-applier. The test resolves existing SystemTypeSets as dependency
targets through three empty test-only sets, adds no runnable systems, and checks
the actual flattened paths. Its 18 fresh children compare every gameplay tick
and every per-target command subsequence. The unforced nine-run regression remains.

The annotated candidate passed both parent tests: nine ordinary children and
eighteen opposed-order children, each case in fresh default/default/pinned
processes. All 27 runs retain the five real access incompatibilities and the
complete 143-row external incident-conflict multiset. Their 92-tick gameplay,
digests and per-target subsequences agree across ordinary and both forced orders
and with the original unannotated baseline. Final digests are
`445a02464d996220` (retarget), `edb813cd82fa5d16` (clear) and
`f1bd96f4faeb1661` (death/reacquisition).

The successful SDK-enabled source-bound build, two frozen test processes,
27 full child reports and before/after source/dependency/artifact checks all
exited zero. An earlier test-only trait-object API compile failure remains
preserved separately. These targeted checks validate this five-pair slice;
the combined integration census and ordinary final gates remain the integrator's
responsibility. This slice does not edit either an allowance or a capture.
