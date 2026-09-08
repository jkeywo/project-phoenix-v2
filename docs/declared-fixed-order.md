# Declared FixedUpdate execution order (#1400)

`src/sim_sets/order.rs` records the execution order of conflicting owners within
the existing simulation phases. Each owner joins a concrete `FixedStep` set at
its original registration. Phase membership, conditions, system parameters and
function bodies stay with that registration. The policy also names the early
identity, lobby, reconnect, output and asteroid-streaming boundaries, and the
late scenario-publication/narrative boundary.

The sequences preserve the built SingleThreaded baseline at `5ba6ad08`.
Registration order, system names, graph indices and observed timing never decide
production order. The checked-in owner fixture records the historical execution
positions used by the registration-perturbation proofs; production does not
load that fixture. Physics keeps its existing serial configuration.

## Deferred visibility

Execution order and command publication are separate contracts. An extra
`before_ignore_deferred` edge can overlap an existing normal edge during set
flattening and suppress its automatic flush. It can also propagate pending
flush work to another boundary. Preserving the old source declarations is
therefore insufficient to preserve their resulting schedule.

`order::pass::DeclaredOrder` appends to Bevy's schedule-build passes after the
default automatic deferred-insertion pass. It adds only the explicitly listed
execution edges to the flattened graph. Bevy then performs its normal cycle
analysis, transitive reduction and ambiguity detection. The pass neither
inserts nor removes systems, barriers, conditions, access declarations or
ambiguity exceptions. Typed owner membership must contain exactly one system;
typed phase boundaries include their actual nested members. The headless
auto-start owner is explicitly absent on other host profiles.

Ship LOD promotion and demotion also write through deferred Commands. Their
buffer remains after region membership's buffer at the existing shared flush,
even though those writes do not produce an ordinary ECS ambiguity.

## Callbacks inside an owner

`SimDispatch` calls multiple broadcast producers through one exclusive system.
Their registry also needs declared order: shuffling plugins previously swapped
RepairState and PowerState messages without changing the ECS graph or digest.
The seven production factories now name typed `SimProducer` owners in the existing
canonical sequence: Repair, Power, Shields, Weapons, SimState, Modifier, Outbox.
Each producer retains its audience, cadence, callback and message sequence.
Generic unnamed registrations retain their own insertion order after these owners.

Rhai scheduling sinks are local to synchronous script calls. The live trigger,
callback and delayed-action paths take mutable `WorldScriptRuntime` access;
their explicit owner order also serializes its shared operation budget. The
same-tick callback restore fixture checks an order-sensitive victory, complete
saved callback identities and every subsequent digest and queue boundary. See
`docs/script-callback-order-proof.md`.

## Evidence and standing guards

The comparison uses physical system identities, preserving repeated instances,
all prior ordering paths, producer-to-barrier-to-consumer obligations and the
actual ordered command-buffer frontier at each consumer. Barrier allocation IDs
are local to a capture. Executable-order analysis describes the built schedule;
behavioral tests cover the conditions that actually run.

The pool-equivalence mission now compares every completed frame/tick observation
between two default-pool processes and one pinned process. The publisher,
admitted-producer, foreign-consumer and Captain/Sensors fixtures preserve their
full state, wire, refusal, control-tenure and nonvacuity assertions while varying
the actual registration order. Historical opposed-order commutation receipts
remain historical evidence; the current fixtures check declared ordering under
those registration changes.

The ambiguity gate retains full names, access vectors and instance multiplicity.
Its current allowance is empty, so the standing test activates Bevy's strict
ambiguity error check. The immutable original capture and verified pre-change
history remain the bound against growth. The focused native comparison preserves
all 265 ordinary instances, 21 barriers and 37 deferred producers, including every
ordered publication frontier, and all 240 mission observations in each pool mode.
Complete publisher and admitted-command traces match their preserved baseline.
Final batch validation also includes the existing perturbation, restore and
cross-peer guards; a zero census alone does not establish gameplay equivalence.
