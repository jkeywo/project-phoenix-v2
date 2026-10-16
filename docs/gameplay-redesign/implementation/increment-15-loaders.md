# Increment 15: optional shared loader groups

Prepared against `32ba7bb6`; adopted by the integrator after review of
`e97960e7`, before runtime implementation. No playable exit is claimed.

## Selected scope and actual owners

S004 selects finite shared handling where fitted, useful reload priorities and
ordinary automatic readiness, while retaining independent loaders. The review
leaves exact grouping, throughput and damage rules open. This is a bounded first
consumer for existing guided tubes; kinetic magazine loading is not silently
migrated, and general personnel/cargo logistics do not become weapon loaders.

| Existing owner | Extension boundary |
|---|---|
| Gameplay `weapons/torpedo.rs` | Sole magazine `Stock`, per-tube `target_count`, load/unload timer, loaded rounds and bursts. Add immutable group membership and retained per-tube priority here. No second inventory, work queue or controller. |
| Simulation `console/weapons/torpedo.rs` | Current admitted tube controls and magazine claim arbiter. Grouped competing claims must use the same priority order as progression, rather than arrival/vector order. Keep the existing magazine AI grant policy and paid transfer transaction. |
| Simulation `console/weapons/guided_advance.rs` | Existing physical `TorpedoWork::Progress` spends actual tube reserve; `Transfer` spends magazine reserve; `Launch` pays the actual launcher. Add a derived shared handling limit before requesting existing work. |
| Simulation `console_ai/server.rs::ai_torpedo_load` | Existing ordinary per-System load policy requests desired readiness through admission. Continue that controller; priority is retained operator intent, not a new loader AI. |
| Stores/Logistics/Production/ports | Existing real magazine occupancy, loading claims, finite supply, timed deliveries and receiving-space constraints remain authoritative. No group stock or refill shortcut. |
| Existing Model/GUI/Workshop/snapshot/digest | Add closed priority intent, permitted queue/status, exact accepted-source group editing and continuation alongside the existing tube owner. |

The current loading claim reserves one actual magazine round before work and
consumes it only at load completion. Cancellation releases that same claim.
An unloaded round returns only at the existing paid completion. Loaded and
pending-burst rounds already occupy magazine capacity. Preserve these rules.

## Proposed authored data

Add optional `[[torpedoes.loader_groups]]` rows with stable `id`, unique member
`tubes` and finite positive `work_rate` (load-work seconds per simulation second).
All members resolve to actual fitted guided-tube Systems sharing this ship's
existing actual `torpedo-magazine` owner. A tube belongs to at most one group.
Require at least two distinct members, known references and `work_rate` no greater
than member count. An absent group leaves every old independent path unchanged.
Groups do not require another seat, Console, System or resource service.

Add optional `load_priority` to tube authoring: unsigned bounded integer, default
100; lower numbers go first. It is the initial retained value, not a new resource
or automatic efficiency bonus. The existing per-tube `load_time`, volley target,
stock and real work costs retain their meaning. Do not change weapon flight,
firing patterns, launch authority, magazine capacity or projectile profile.

The group's physical handling mechanism belongs to its existing magazine
System. Its current Operation availability multiplies the shared rate; zero or
unavailable machinery pauses grouped work. Actual paid tube work still supplies
loading motion and magazine transfers pay their existing cost exactly once.
No extra continuous power charge, battery, timer or group HP is invented. A
future separately equipped loader would require its own real composition
consumer; it is outside this bounded implementation.

## Proposed allocation and hysteresis

At the ordinary fixed loading boundary, each group receives a fresh derived
budget `work_rate * dt * magazine_operation_fraction`. Unused work is discarded;
it cannot accumulate while idle or recharge a hidden handling reserve. Each
member can advance by at most its own clipped `dt` and remaining stage work.
Thus rate one shared by two two-second tubes needs four work seconds to prepare
two rounds, while independent loaders still need two elapsed seconds.

For each group sort eligible actual load/unload operations by:

1. ascending retained tube priority;
2. already committed operation before a new operation at equal priority;
3. canonical member SystemId in lexical order.

Authoring/vector order, packet arrival, player identity and ECS iteration do not
break ties. Keep independent paths in their original order. Apply this same group
order to competing new magazine claims, including explicit LoadTube requests and
automatic readiness requests, before any actual stock/payment transaction. Grouped
claims use one derived arbitration pass after that tick's admitted readiness and
manual load intents settle; they cannot claim earlier merely because their old
message handler ran first. Do not persist another request queue: standing targets
remain tube state, and an unserved one-shot manual request receives its ordinary
correlated refusal/wait outcome without reserving imaginary stock. A positively
accepted one-shot must either commit its actual loading claim or receive an
explicit failure outcome; never silently drop it. Standing readiness remains
automatic and needs no repeated per-round clicks.

Allocate only up to remaining group work and the member's physical request, then
request ordinary tube work. The exact paid tube delta includes the existing
Operation/power grant once; the group gate must not multiply that same fraction
again. Debit group budget by actual progressed work, not
requested work. An unfunded/unavailable high-priority tube must leave capacity
for another feasible member without paying, consuming a round or advancing a
cooldown. Unloading competes for the same handling capacity as loading.

Existing load/unload timers and their magazine claims retain partially completed
work. A changed priority changes future budget allocation; it does not cancel,
refund, reset or duplicate an already committed stage. Existing claims are not
revoked to satisfy a later higher-priority request. A queued committed Loading
phase may hold its one existing claim while receiving zero group work; report
that distinction rather than claiming progress. Reducing requested readiness or
explicit cancellation uses the current tube lifecycle and releases only the
existing unconsumed claim. Do not introduce an unbounded same-step chain: retain
the current one-stage-per-tube advancement/start boundary.

A priority may intentionally starve repeated lower-priority work. Expose that
choice and the actual waiting reason; do not invent age boosts or a second hidden
policy. The default reference has finite desired readiness, allowing lower
members to proceed after higher members reach their targets.

## Authority, knowledge and ordinary AI

Add `SetTorpedoLoadPriority { priority }` to the actual tube's existing admitted
System boundary. Only its current authorised operator can change that member's
priority. It cannot set another tube, alter group capacity or command the
magazine. Displaying a group summary or sharing a Console does not grant control.
Stale-host and GM operation use ordinary admission, with correlated outcome.

Human readiness targets and Load/Unload remain standing normal machinery
requests. Group scheduling is a deterministic physical service constraint, like
finite supply, rather than another decision controller. The existing individual
load AI continues requesting its authored readiness through existing admission
and permitted facts; it respects retained priorities and actual stock. AI needs
no hidden cross-Console enemy, inventory or component knowledge. No temporary
backfill-specific scheduling behavior is added.

Each tube's own publication reports priority, its group identity/rate, current
phase/progress and a closed local waiting reason (shared handling, tube work,
magazine unavailable, or stock). Do not expose foreign member priorities,
readiness or conditions just because hardware is shared. The magazine Console
can receive an aggregate own handling summary; named member details require the
existing explicit typed tube feeds. Missing information remains unknown and
does not alter physical service allocation. Controls must explain when current
priority is delaying requested readiness, without requiring each round approval.

## Workshop, content and continuity

Extend actual guided weapon and group authoring through existing document
transactions. Create/edit/remove group membership/rate and tube priority, inspect
inherited definitions, explicitly materialise edited leaves, and validate before
Preview/disposable Test. Removing or renaming a tube/group must guard references
atomically; no dead membership or unsupported non-guided binding may be accepted.
Preserve unrelated dependency bytes and existing history/export/reload behavior.

Add an optional shared-loader Cruiser, inheriting unchanged finite stocks, load
times, energy, reactor, damage and crew roster. Its two tubes share rate one,
initial equal priorities, and expose both priority inversion and low-stock
contention. Shipped default independent hulls remain unchanged. This is a quiet
mechanism probe, not a scenario pacing/deadline redesign.

Persist priority on each existing tube snapshot row and fold it with that tube's
existing state. Immutable groups reconstruct from the exact accepted recipe.
No budget or queue view persists between intervals. Validate exact tube roster,
bounded priorities, group references, existing timer/claim conservation and
complete candidate equipment before either row-local or whole restore writes.
Missing required priority state across the new compatibility boundary must not
silently reset a saved decision. Grouping cannot be altered by a later launcher
config on restore; accepted candidate recipes remain the source. Active flights
and bursts retain their existing frozen payload and custody semantics.

Advance affected protocol/content/snapshot/rules versions at root integration;
retain the existing transport, delivery classes, save mechanism and deterministic
fixed-step owner. Host reassignment, reconnect and save/resume retain work and
priority, while current control sources remain derived normally.

## Implementation and evidence sequence

1. Review/adopt this contract and PASM; then extend accepted definitions and pure
   allocation through the actual existing guided tube consumer.
2. Join admitted priority, shared claim order and paid physical progression;
   preserve independent behavior and exact finite stock accounting.
3. Add permitted controls/status, Workshop native transactions and optional fit;
   integrate capture/guards/digest immediately with authoritative state.
4. Independently source-review the composed packet. Root alone runs Cargo,
   full gates and native acceptance; workers may run targeted JS/static checks.

Required targeted proofs:

- Independent default remains equivalent; rate-one/rate-two groups respect
  aggregate capacity, per-tube limits, stable ties and concurrent unload/load.
- Manual priority inversion affects next work without erasing partial progress;
  low stock claims follow priority, duplicate requests do not duplicate claims,
  an unfunded high-priority member does not waste capacity or payment.
- Real finite tube/magazine energy, damage, flow cuts, thermal pause, receiving
  capacity and existing incoming/logistics claims remain conserved. No free
  transfer, ammunition, heat reset or hidden refund.
- Ordinary vacant/delegated AI prepares a salvo; Human priority/target work,
  Console isolation, stale-host denial and ordinary handoff remain meaningful.
- Actual joined-host group loading, interruption, priority change and firing;
  fresh JSON restore with complete capture/digest equality for 360 ticks and
  forged roster/priority/claim/recipe no-write refusals. Include actual supply
  shortage/replenishment and more than one group plus an independent tube.
- Workshop create/edit/materialise/undo/export/reload/validation, real Preview
  and disposable Test; targeted JS transactions are not native acceptance.
- Compare salvo preparation and station decisions with independent hardware.
  Human workload, matching browser/native devices and broader combat balance
  remain separate acceptance; this bounded slice alone does not close15.

## Implementation checkpoint and evidence boundary

The isolated source packet extends the existing guided consumer, including
accepted immutable groups, per-tube priority, shared work limits and ordered
magazine claims. The original magazine grant policy/transactions and ordinary
load AI remain. Existing LoadTube one-shot requests now receive correlated
terminal outcomes after the actual magazine claim frontier. Standing readiness
continues without further clicks. Derived own-tube waiting receipts are
presentation only and clear on restore. Independent tubes retain their existing
progression/auto-load path and do not gain priority state in capture or digest.

The common guided authoring form carries initial priority; the group editor
uses native `put` of only `torpedoes.loader_groups`, preserving inherited
machinery and rejecting missing/duplicate membership before acceptance. New
ordinary joined-host and Backfill proof source is
`tests/shared_loader_continuation.rs`; actual native execution is pending. Pure
owner tests cover rate conservation, actual granted fractions, priority changes,
retained claims, magazine unavailability and unload contention. Native document
source preservation has its separate owner test. No Rust test has run in this
lane. Compatibility versions remain the integrator's responsibility.

Eight focused JavaScript files pass 599 tests after adding the optional semantic
action to the exact inventory and correcting new panel labels. This includes
current-owner projection/action refusal, actual mounted tube controls and
retirement, accepted Workshop transaction/history and existing weapon controls.
Layer rules, strict strings (6499 entries), pure client build and owned Rust
formatting pass. These receipts do not establish a full integrated gate, native
continuation, actual Workshop Preview/Test, device/workload acceptance or an increment 15 exit.

Independent read-only source review passes for the physical owner/continuation
core (Craft) and the GUI/Workshop subset (Personnel), against the frozen
`b7c81e29` to `4bc4a91e` packet. Their reviews do not claim compilation or test
execution. The subsequent `6a4875d8` and `baa0bdbe` changes correct the host
proof's snapshot-readiness API and assert ordinary running bootstrap; they do
not change runtime behavior. Integration Review passes the bounded actual-host
proof through `e1ff4759`, after correcting its initial settled-empty premise:
ordinary target-zero handling must finish prior loads/unloads and release actual
claims before the new priority witness. Finite ammunition, paid progress,
aggregate rate and complete JSON360/no-write assertions remain. No reviewer ran
Cargo or tests. This source-reviewed packet remains uncompiled; native owner
and host results, integrated compatibility and acceptance must be established
separately.
