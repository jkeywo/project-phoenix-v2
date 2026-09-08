# FixedUpdate ambiguity audit

Issue #1400's first slice measures unordered conflicting access in the ordinary
headless FixedUpdate graph. It does not change execution, RNG streams, minting,
or declare that conflicting systems commute.

`src/headless/determinism_audit.rs` initializes the schedule without running it.
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
introduction. That branch also checks the reviewed first count and fingerprint,
because a PR base predating the entire batch cannot ratchet against an earlier
commit inside the same PR. `tests/fixed_update_ambiguities/bootstrap.rs` encodes
the parsed rows with the production `Ambiguity` serializer through
`serde_json::to_vec`: compact UTF-8, no BOM or trailing newline, retaining every
row and access entry. Git's read-only `hash-object --stdin` must report blob
`a3a15b158431852c9b3c3bf890dbe8929e9e02ec` for those 437,412 bytes. It writes no
object, applies no clean filter and performs no network operation. Pretty-print
whitespace and checkout line endings therefore do not affect the check.

Keep the first 1,968 allowance unchanged through its initial introduction while
live conflicts shrink. Once the trusted base contains that ledger, the ordinary
access-subset/multiplicity comparison allows the checked-in allowance to shrink.
No repository variable or external approval setting is needed. Neither an empty
placeholder, a hand-authored list nor the 1,973 capture substitutes for the first
allowance, and a missing current ledger fails. Complete-history and no-delete
checks remain mandatory even when the fingerprint matches.

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

The initialized post-split graph reports 1,929 rows. Full name/access/multiplicity comparison finds zero added rows and 39 removed rows against the original 1,968 capture. The real Rust bootstrap serializer produces the pinned fingerprint, and the ordinary live-subset/history gate passes. Preserve the original allowance through first introduction; do not replace it with this smaller capture inside the same introduction. Typed stream/namespace ownership and the baseline-preserving Projectile edge carry their own restore and default/pinned behavior tests. Final integration remains the integrator's responsibility.
