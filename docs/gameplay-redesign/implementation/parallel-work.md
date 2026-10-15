# Parallel implementation through increment 24

Approved 15 October 2026. This changes the implementation schedule for increments
16–24, including the mission planner. It preserves their scope and individual
playable exits. Increments 0–15 retain their historical dependency sequence.
Initial implementation, targeted validation, integrated validation and human
acceptance remain distinct; this plan marks none of them passed.

## Entry and ownership

Finish increment 15's initial integrated implementation and targeted checks before
dispatching the first runtime wave. Earlier outstanding acceptance remains recorded.
For every slice adopt its PASM/implementation contract before runtime edits. Pin
the prerequisite source revision and the exact contract consumed by the worker.
A slice may start before its parent increment's full exit only under the entry
conditions below. Its parent cannot close before all completion dependencies pass.

Use three implementation lanes and one integrator, in isolated worktrees. Give
workers leaf modules and real consumers; never create unused frameworks or fake
physical operations to claim a dependency is satisfied. Workers own targeted checks
and independent read-only review. The integrator owns shared schema changes,
schedule registration, version advances and dependency-order integration. Keep
Cargo builds and performance captures sequential. Run the required full gates on
the integrated result before a push, not per worker commit.

For PASM repository observation during parallel work, use a clean committed
validation worktree with explicit matching spec and workspace roots, and record
its exact revision. The current pinned tool scans nested worktrees, so observing
the integration checkout can include unrelated worker packages or race a worker
rename. This changes the observation root, not the model or findings. The final
gate must still cover the integrated revision; do not substitute an older pin.

Keep per-increment commits, with slice/fix commits where useful. Later validation
can run on pinned worktrees while independent work proceeds. No initial commit
claims a verified exit. Do not push or close PRD #1562 merely because a lane ends.

## Lanes and work slices

| Lane | Slice | Entry requirements and actual deliverable |
|---|---|---|
| Communications | 16 core | After 15's initial integrated implementation: shared human/AI channels, generic requests, receipts and equipment-backed signal links. Preserve existing transport; no message capacity accounting. |
| Communications | 17 electronic operations | After 16's shared channel/link contract and its actual consumer are integrated: jamming, spoofing, protection and concealment using the existing signals, mounts, energy and conditions. |
| Movement and craft | 18 movement/transit | After 15's initial integration: direct/assisted controls, Match Speed, docking and configured transit. No general dependency on 17. Formation order delivery joins 16 when available. |
| Movement and craft | 19 static deployables | After 15: ordinary finite battery-backed entities and actual deployment/recovery jobs. This is not the emitted-ordnance flight carrier. Pin the shared deployment/custody contract with lane 3 first. |
| Movement and craft | 19 carried/remote craft | After 18 motion and 16 command-link consumers: sorties, return/recovery and ordinary link-loss AI. Deception equipment joins 17; passenger craft join 20's actual custody owner. |
| Personnel and facilities | 20 local personnel | After 15: Duty Officers and internal medical/security/team work using existing jobs, routes and conditions. Use the same actual boundary/access state for movement and containment. |
| Personnel and facilities | 21 facility foundations | After 15: migrate facilities onto existing equipment, damage, stocks and servicing. This does not require all of 20. Workforce joins 20; outside work uses real services and deliveries. |
| Personnel and facilities | 20/21 external operations | Join 19's deployment/custody and 20's local-team contracts. Boarding, rescue, passenger transfer, workforce and outside-actor deliveries use one actual personnel/custody state. New transit uses 18; carried craft use 19 only where needed. |
| Personnel and facilities, after foundations | 22 planner core | Can overlap late 16–21 once activity/event contracts are adopted. Implement 22A using an existing repair/delivery consumer, then 22B HTN and 22C scheduling against real capabilities. Forecasts do not claim resources or control Systems. |
| Same lane | 22 integrated planner | Monitoring/repair, authoring and full authored demonstration join 20/21 outcomes. Full increment exit still depends on 21. |
| Same lane | 23 continuing modes | After 22's initial integrated implementation, establish persistent commitments. Patrol, War and Sandbox retain separate ordered playable exits. Full increment exit depends on 22. |
| Available communications lane | 24 opportunity search | After 22's operator/precondition/outcome contract and actual HTN consumer settle, bounded hypothetical search can overlap 23. It never mutates physical state. |
| All lanes, one integrator | 24 admission/pacing/final audit | Join 23's authored outcomes and 22's scheduler. Preserve announced commitments, complete planetary/catalogue/Workshop/crew-size audits and all remaining integration evidence. |

These are assignments of responsibility, not a promise of simultaneous use of all
three lanes at every point. Use a free lane for review/validation or an independent
ready slice; do not start a dependent slice to keep an agent busy.

On resuming the 15 October pause, split independent local Personnel/facility work
from the same lane's planner work so Security/team routing and treatment can
progress while temporal planning is reviewed. Their physical-custody join still
uses 19's single lifecycle. A separate validation worker owns the heavier actual
PDC saturation probe; it cannot change weapon limits or the gameplay owners.
These subtasks use isolated worktrees and the same single Cargo/integration owner.
The completion dependencies and separate authored-mode exits below are unchanged.

## Completion dependencies versus early work

The ledger's `depends_on` fields describe full increment completion prerequisites.
`parallel_work.slices` describes permitted early implementation and its contract
conditions. Neither is a substitute for acceptance evidence.
From increment 16, `current_increment` identifies the integration focus rather
than claiming every lower-numbered increment is complete. Historical entry through
15 still requires its existing completion or recorded advance authorisation.

| Increment | Completion prerequisites |
|---|---|
| 16 | 14, 15 |
| 17 | 14, 15, 16 |
| 18 | 14, 15; remote formation requests additionally integrate 16 |
| 19 | 16, 17, 18; shared personnel/custody contract with 20 |
| 20 | 14, 19; local work can start earlier |
| 21 | 13, 18, 20; facility foundation can start earlier |
| 22 | 21; existing-capability planner core can start earlier |
| 23 | 22; Patrol → War → Sandbox remains ordered |
| 24 | 23; search can start after 22's stable operators |

The shared contract between 19 and 20 is not mutual completion dependency: 19 owns
craft/deployment and 20 owns personnel/custody. Integrate the contract once and
prove their join before either claims passenger operations. No separate passenger
inventory, replicated door state, alternative job owner or replacement AI is allowed.

## Shared integration surfaces

Coordinate Model messages/configuration, System registry and command admission,
snapshot schema/capture/preflight/restore/digests, central schedules, World events
and scripts, Workshop shell registration, strings and shared Cruiser data through
the integrator. Workers propose narrow additions; they do not independently bump
versions or rewrite shared dispatch tables. Existing Gameplay/World/Session
ownership and knowledge-limited individual System AI remain intact.

Before integration, test the actual crossing between lanes: channel loss during a
sortie; interrupted custody transfer; facility failure during servicing; planner
refusal/capability loss; save/resume and deterministic ordering of simultaneous
work. Source fixtures alone do not establish playable, performance or human-workload
acceptance. Update PASM, the ledger/report and PRD #1562 with precise evidence.
