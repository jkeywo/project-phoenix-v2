# Captain / Sensors one-pair proof (#1400)

Source preparation only. No native execution or commutativity annotation is
claimed. The exact fixture base is `5ba6ad0805c700142d4ce53451687d09952a2368`.
Execution requires the separately reviewed production declared-order policy;
that policy is not copied into this test-only worker.

The pair is `console::captain::server::operate_captain_ai` and
`ship::sensors::operate_sensors_ai`. The initialized graph must resolve exactly
one nonexclusive, nondeferred instance of each, with the full shared access
vector containing only `AdmittedCommands`. Every actual graph must contain the
production Captain -> Sensors path and no reverse path. The test adds no
direction edge or ambiguity annotation. No production system is re-registered.

The small new test world references shipped Alliance destroyers and transfer
depots. Destroyers retain their composed Captain policy, Sensors selector,
nominal equipment and authored 120/260-unit scan bands. Cruisers cannot prove a
positive scan because they author no scan suite. Two NPC destroyers are used,
without LocalShip or FleetSlotOf control overrides. The local host and unrelated
fine systems remain Human-controlled. No shipped template is edited.

Seven phases exercise both ships: simultaneous alert activation, ScanTarget and
SetScienceTarget; standing-order no-op; alert stand-down and ClearScienceTarget;
a real OutOfRange scan refusal; its next-cadence retry; Human gating with live
tempting inputs; and successful resumption without replacing those inputs.
Captain's initial recent-combat memory and each phase's standing Scan/Destroy
objectives are explicit fixture state. This does not claim the upstream damage
or mission authoring route. The ordinary aggregate publishes the objectives;
there is no one-tick fabricated blackboard. Scan success reads the live authored
depot condition and mass, raises the subject scan flag and emits a real
per-ship TargetDesignation. Refusal comes from the unchanged Science consumer.

Staging waits for the real snapshot latch, consumes that publication tick with
both producers still Human-gated, and checks the published Scan/Destroy IDs and
targets before waiting for the next decision boundary. `AiSnapshotReady` is
rearmed in `FixedLast` for the next tick; stopping when it becomes true alone
would still read the previous aggregate. All settling ticks remain in the full
gameplay trace, and staging must emit neither producer's commands. This fixes
R4's first executed failure at the no-Scan phase; R5 still requires native
execution of every existing phase and comparison.

Every observed decision starts with two correlated, ordinary accepted
SetPowerGroupAllocation requests (weapons 0 then 1). Probes assert the complete
raw prefix is preserved, and the real per-ship Power consumer must apply the
last request and settle both correlations once. This tests accepted-command
continuation, not network authentication. Captain and Sensors admissions must
carry their ship's own AI token. Sensors can emit multiple payloads to the same
SystemId: selection and scan use separate typed consumer projections. Every
stable interleaving of actual nonempty Captain/Sensors chunks must retain all
three consumer subsequences and the complete foreign prefix.

One parent uses the existing `SimRegistrationOverrides::registration_order`
seam: canonical registration and shuffled seeds 17 and 991, in two fresh default
processes and one pinned process each: nine children. The actual pre-initialized
registration sequences must be stable within each role and all three distinct;
different seeds alone do not establish that fact. Every child asserts its real pool,
executor and seed. Complete per-tick comparisons retain the three consumer
subsequences, foreign commands, full selected-ship blackboards and scan records,
alert/stance/attacker/combat state, frequency, hull/physics/power, WorldData,
all flag entries, typed coordination, full outbound messages, balance events,
command log, digest, RNG and mint. Raw global append order and probe snapshots
are retained separately. Unordered maps are represented by all sorted key/value
entries; emitted vectors and wire order are not sorted or dropped.

Graph capture reuses `headless::determinism_audit::graph::fixed_update_graph`.
A small read-only build pass captures the complete initialized physical access
vectors before Bevy extracts systems into its executable. It changes no edges
and is removed after capture. Reported conflicts cannot substitute for that
raw-access evidence: the selected pair remains physically incompatible even
when its declared dependency removes the reported ambiguity.
The report retains its complete nodes, hierarchy, authored and effective edges,
and reported conflicts. Four probes are identified by actual function identity
and must satisfy Admission -> before -> producer -> after. All remaining
production instances retain their own IDs and complete metadata. Correspondence
across registration shuffles uses each actual system's name, type, access flags,
condition sequence, and full ancestor-set descriptors. The two legitimate
`prune_removed_station_puppets` instances are distinguished by their real
unphased versus Publish ancestry; neither instance is collapsed.

Typed ancestors retain concrete type identity, label and condition sequence.
Anonymous condition sets retain their raw ID and label in the report, but their
correspondence descriptor uses the actual descendant-system metadata multiset
and condition sequence: an anonymous allocation number is not stable under
plugin shuffle. Every resulting physical-instance descriptor must be unique;
any remaining ambiguous correspondence fails. Complete descriptor inventories
must agree before comparing all production paths and deferred-writer/sink
visibility obligations. Their endpoint indexes refer to that exact sorted
inventory, rather than a hash or a collapsed system name. The full raw graph,
physical instance-to-descriptor mapping and ancestor metadata remain available.

Roles with identical actual registration additionally require identical raw
physical instance IDs, metadata and ancestry. Only actual ApplyDeferred nodes
and the four exact probes are excluded from the production comparison. External
raw conflicts and multiplicity stay identical. Type IDs and instance IDs are
only evidence within these fresh processes of the same executable; they are
not a cross-build identity contract.

Setup and gameplay are separate from `graph.rs`'s declared-direction contract.
The seven-phase behavioral checks and actual-chunk interleaving checks are
retained independently of the policy. This source licenses no broad producer
exemption or allowance change. R1 remains preserved as unexecuted source; its
post-initialization graph access lookup was corrected before any native run.
R2 is also preserved: its overly strict unique-name assumption was corrected
using real ancestry rather than deleting the duplicated production instance.

After integrator source/dependency review and a coordinated native build, run:

```text
cargo test --locked --features headless --test captain_sensors_ordering -- captain_and_sensors_preserve_typed_effects_across_registrations --exact --nocapture --test-threads=1
```

Require one passing parent and nine distinct successful child processes, seven
phases per child, both ships' concrete effects/refusals/no-ops, nonempty raw
prefixes and interleaving checks, and equal full gameplay traces. A whole-binary
run also reports the child driver as one intentionally ignored test. No timing,
network/mesh, browser, or full #1400 completion claim follows from this fixture.
