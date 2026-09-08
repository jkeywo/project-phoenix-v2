# Four admitted-command producers: six-pair fixture (#1400)

## Declared-order stability source revision

The current test source replaces the historical opposed-order roles below with
ordinary registration, `RegistrationOrder::Shuffled(17)` and `Shuffled(991)`;
the publisher proof also retains a separate physics-last role. Each role runs
in two fresh default-pool processes and one pinned process. These are existing
headless test seams, with no shipping flag or test-created ordering edge.
Execution of this revision is pending; the results below remain historical
receipts for their original source, not validation of this revision.

The expected directions come from the frozen actual execution inventory in
`tests/fixtures/determinism/declared-owner-order.json`. Every selected conflicting
pair must follow that direction in every role while retaining its complete raw
access vector. Full gameplay, state, wire, RNG/mint and positive coverage
assertions remain. This now proves declared-order stability, not commutativity.

Raw system instances, containing-set nodes/hierarchy and effective edges remain
in every report. Concrete IDs
may change with registration: comparison requires a unique complete metadata
correspondence including the actual ancestor-set semantic membership multiset,
rejects any still-ambiguous repeated concrete identities, and retains
all prior concrete dependency paths and deferred writer-to-consumer visibility
obligations. Automatic ApplyDeferred instances are retained in raw output but
are not assigned a cross-registration identity. Anonymous condition-set allocation
numbers also remain raw; correspondence uses their actual descendant metadata
multiset and ordered condition names, preserving distinct set multiplicity. The producer's exact eight
probes retain their individual admission/before/producer/after assertions;
incidental probe/barrier paths remain diagnostic output. Physical external
conflict vectors and multiplicities remain compared even when declared order
removes those rows from the reported ambiguity census.

## Historical proof and receipts

This fixture passed targeted native execution on the integrated worktree at
`a1619a9389b6eaf5aafacd06dfcd260619c006fa`. It adds no production ordering,
commutativity annotation, allowance change, or shipped authored value.

The exact producer identities and all six complete historical ADC-only vectors
are retained in `tests/fixtures/determinism/admitted-producer-six.json`. The live
initialized graph must independently resolve each unique production instance,
retain its real incompatibility and exact access vector, and retain its ordinary
producer-to-consumer path. A new edge or overlapping access fails the fixture;
the historical inventory is not a current census result.

| Producer | Actual typed consumer | Positive state exercised |
| --- | --- | --- |
| `console_ai::server::ai_power_allocation` | `ship::power::handle_power_messages` | Authored healthy-battery/red-alert allocation, complete commanded levels |
| `console_ai::server::ai_shield_focus` | `ship::shields::handle_shields_messages` | A different real arc bearing on each hull, actual focused facing |
| `console::navigation::server::operate_navigation_ai` | `console::navigation::server::handle_navigation_waypoint` | Published per-ship Reach doctrine, distinct waypoint and generation |
| `console::repair::server::operate_repair_ai` | `console::repair::dispatch::handle_dispatch_repair_team` | Prepared reported damage request, real team dispatch/travel, HP recovery and RepairApplied |

The full App boots `probe_fleet_duel.toml` and selects its second authored
GameStart cruiser and script-authored hostile cruiser. Both are ordinary NPC
hulls without FleetSlotOf; the LocalShip human-seeking resolver cannot overwrite
their prepared per-system authorities. LocalShip/crew-seeking policy is not
covered by this fixture. All ships' unrelated systems are placed under
Human control; the two selected cruisers retain their authored policies,
selectors, team counts, timings and normal consumers. Navigation anchors and
standing doctrine, damage and already-reported Repair requests are explicit
fixture inputs. Repair notification/coordination lag is not claimed by this setup.
Prepared damage lies strictly inside each selected subsystem's authored Damaged
band; exactly its damage threshold would still be Operational.

Before each observed decision, the fixture resets each NPC's red alert to false
and sends correlated `SetRedAlert` requests `[false, true]` through the ordinary
accepted-command continuation seam to `RED_ALERT_SYSTEM_ID`. The actual Captain
consumer handles every Ship, including these NPCs; the frequency consumer is
LocalShip-only and cannot prove this fixture's foreign-prefix effect. This does
not claim network authentication coverage. Read-only before/after probes around
the actual producers assert the **complete** raw prefix is retained and capture
ship-owned AI admissions. In positive, settled and Human-gated phases, each hull
must finish with red alert true and both prefix correlations must receive exactly
one targeted reliable Applied response, including the idempotent false request.
The full trace adds red alert and retains frequency and all existing state/wire
observations. Power therefore still sees its required true alert at its Physics
boundary. There are no synthetic AI emissions or replacement producers/consumers.

The predicates in `domains.rs` follow the four real target-and-payload filters;
Shields projects each actual authored arc's request sequence in the consumer's
outer arc order. Every stable interleaving of each pair's actual nonempty emitted
chunks is checked against the unchanged prefix and complete consumer subsequences,
including operands, response token and correlation. This finite check is tied to
actual production emissions, not a fabricated payload/name table.

One parent runs nine fresh processes: ordinary, forward and reversed orders,
each in two fresh default multi-thread pools and a pinned single-thread pool.
Test-only empty sets order the existing type-set instances; production systems
are not re-registered. The graph proof retains exact external raw vectors and
multiplicity. Eight probes are identified by their actual function identities,
with both Admission systems preceding each before-probe and the actual producer
between its before/after probes. Actual system IDs, names and access flags must
match across these same-source runs before production paths are compared. Every
ordinary production-to-production path must survive. For each deferred writer
and sink that had an actual ApplyDeferred between them, some actual barrier must
still provide that visibility. Compiler-generated barriers can be shared
differently under forced orders; their identity is not a preservation rule.
Full raw instances, edges and name-path multiplicities remain in the report,
including displaced incidental probe paths in a separate diagnostic.
The actual raw queue/probe order is retained separately; the comparison checks
consumer subsequences plus every observed tick's digest, RNG/mint, command log,
complete blackboards, consumer state, balance events and typed network messages.

Coverage must be nonvacuous: eight positive producer/ship combinations, all six
pairs on each ship, a nonempty foreign prefix, actual consumers, real Repair
travel plus positive work, and Power/Navigation standing-order suppression.
Human takeover checks absence of all selected AI emissions; Power and Shields
receive fresh tempting inputs. Navigation and Repair are already settled or
committed there, so that observation does not independently isolate their Human
gate from their no-op conditions.

## Validation

Independent source review and the targeted R5 native run passed. The exact
parent command is:

```text
cargo test --locked --offline --jobs 6 --features host,headless,perf,ultralight --test admitted_producer_ordering -- --exact four_producers_preserve_prefix_effects_and_consumer_subsequences --nocapture --test-threads=1
```

The recorded runner separates Cargo's no-run build from execution of its
reported test executable, verifying the source and dependency binding and
unchanged library artifact hashes throughout. It uses the verified pre-change
commit above as `PHOENIX_AMBIGUITY_BASE_REF`. R5 took 10.53 seconds to compile the
test and reused the library previously rebuilt from this same worktree. Its
local receipts are in `.t2-batch/producer-six-native-r5/`; the seven-file tested
source manifest is `.t2-batch/producer-six-fixture-r7/source.json`, SHA-256
`32655D6DE343CC5B06712D2CB76D177C715D351B6D2A6ABEA09C65BAD7B5177D`.
Only this document and the wiki were updated afterward.

All nine children and the parent passed. Each child observed 380 ticks, eight
positive producer/ship combinations, eight Human-controlled no-emission
observations and 24 actual-chunk interleavings. The default pools had 16 workers;
the pinned pools had one. All complete gameplay traces matched, including every
integer-valued RNG/mint observation. Their canonical gameplay SHA-256 is
`08e1c57dcb3cbdd9e8de002588df7e05010aec9efaec3b2ca335e29cf2ec5f00`.

An independent integer-safe checker reconstructed reachability from the raw
instance/edge reports. All 265 production instances, 30,150 ordinary production
paths and 4,799 deferred visibility obligations survived both forced orders.
The six actual pair directions matched the requested roles, and all external
raw conflict vectors and multiplicities remained. The full raw queues and
probe observations are retained separately from the equal gameplay trace.

Earlier failed attempts exposed fixture assumptions about the authored damage
threshold, probe ordering before Admission, and a LocalShip-only foreign-prefix
consumer. R4 then passed four complete gameplay children before its parent
rejected incidental probe-to-barrier reachability. The corrected graph oracle
preserves production obligations directly; R5 passed all nine children. The
failed stdout/stderr and full raw reports are retained. Output uses
`PHOENIX_PRODUCER_SIX=`, `PHOENIX_PRODUCER_SIX_PARENT=` and
`PHOENIX_PRODUCER_DISPLACED_PATHS=` frames. A whole-binary run additionally
reports the child driver as one ignored test. The full pre-push matrix remains
the integrator's separate gate.

This tranche does not cover the other producer families, thirteen shared-Helm
pairs, alternate emitted variants such as ClearNavigationWaypoint, all consumer
refusals, the upstream Repair notification route, or arbitrary ADC readers.
It provides no blanket 168-pair claim and licenses no annotations before the
actual behavior, production premises and final integrated graph are reviewed.
