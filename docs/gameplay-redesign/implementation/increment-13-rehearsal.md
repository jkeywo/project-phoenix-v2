# Increment 13 — Measured fitting rehearsal

**Adopted implementation contract; first consumer source is implemented, native and playable verification pending.** Prepared from the actual
fitting and disposable Test owners on 16 October 2026. The approved direction
closes the missing whole-fit comparison and direct favourite rehearsal selected
by [increment 13](increment-13.md). Root adopted this contract after independent
source review of its actual boot, fitting and sampling owners. This is not
implementation or a playable exit. Cargo, epochs and integration remain root-owned.

## First actual consumer

Offer **Rehearse favourite** in the private host Workshop planner before launch,
using the existing disposable Test launcher. Begin with the accepted six-seat
Alliance Cruiser and its existing Beam/Reactor definitions. Compare authored
sustained-engagement and burst/recovery reference workloads in independent fresh
baseline and selected-fit children. The equipment graph, per-System AI, admitted
commands, real energy/thermal/Stocks, ordinary observations and weapon effects
remain the current simulation. There is no separate forecast simulator.

Availability requires the current Workshop source-capture/Test capability and a
validated plan. A phone console or Live mission does not gain a WASM host, private
Test command route or Live-state cloning facility. Existing personal save/share
and Queue requests remain available as before. Unsupported surfaces can explain
how to open the favourite in the host Workshop; the first direct rehearsal does
not require manually editing equipment TOML. Model Preview remains the existing
geometry preview, not a thermal or endurance prediction.

The first reference workloads are ordinary authored Test scenarios with real
subjects and controller policies, admitted through the captured source validator.
They are introduced only with the approved consumer and actual accepted-loader
proof; this draft adds no global scenario, hull, hardware family or asset.

## Closed request and immutable context

Extend the existing private Test `Launch` with an optional closed rehearsal
request, not a network System command. Its only inputs are version 1, the current
version-1 favourite plan and one supported reference workload identifier:
`sustained-engagement` or `burst-recovery`. Existing TestSelection supplies the
World, selected slot, hull and seed; existing capture supplies the immutable
source bytes and revision. Duplicate seed/world/actor selectors are not allowed.

Initial technical bounds are a 32 KiB encoded request within the existing 64 KiB
native record limit, at most eight Station sections and 64 slot choices, and
1–128 ASCII lowercase/digit/hyphen slot/definition identifiers. These are closed
request-envelope limits, not fitting points. Reject unsupported versions, extra
fields, duplicate keys, non-offered slots/definitions, bad source identities and
incompatible families. First supported fitting deltas are Beam and Reactor only.

Resolve the plan against the same original accepted hull and full-definition
content_id used by the current planner. Recompute that identity from the real
include-resolved EntityConfig, not a caller-supplied hash or JS merge. Apply the
existing whole-ship/Station/default and explicit-section composition rules;
unspecified sections use accepted defaults. Rehearsal choices grant no live
claimant override, stock, assembly or installation authority.

The selected reference workload must be present in the same immutable captured
source closure and offer the selected hull/slot. Freeze its actual source revision
and assumptions with the result. Ordinary World player overrides are resolved by
the existing player-hull path; if they change the bound fitting context, refuse
rather than silently relabel the favourite. The first reference has no conflicting
fitting/physical-hardware overrides, scenario refits or hidden resource grants.
Missing dependency bytes cannot fall back to mutable Live state, filesystem roots
or a new fetch route. Browser and native use the same validation and payload.

## Existing owners and the initialization seam

| Responsibility | Existing owner / bounded extension |
|---|---|
| Capture and admission | Workshop `test_source.rs`, provider/test_snapshot.rs and editor/workshop-test-snapshot.js validate exact captured World/slot/hull/dependencies. Extend these checks with the closed rehearsal request. |
| Child lifecycle | Existing fresh browser iframe or native private-pipe Test child; TestClockPlugin marks DisposableTest before app.run. No Live sender token, save path or transport is inherited. |
| Legal equipment | Original CompiledFitting plus FittingState validation. Never serialize effective_config as a newly authored hull: its defaults/initial assembly contract still describes the original source. |
| Hypothetical initial fit | One private Test-only constructor stages the existing ShipFitting, PowerSystem and actual capability adapters together before this hull can work. It does not perform a field refit or run beside Repair. |
| Physical operation | Same ordinary AI/admission, ShipPowerSystem/energy_runtime, damage, shields, stocks and weapon machinery. No direct work/grant/damage injection to make a forecast look successful. |
| Measurements | A bounded read-only Test report samples those actual owners after their ordinary settled interval. It owns presentation aggregates, not machinery state. |

Browser `workshop_host/test_browser.rs::install` and native
`workshop/test_process.rs` already install TestClockPlugin before app.run. Add the
prepared rehearsal input only there, after capture validation and before running
the shared schedules. It is absent from ordinary Live boot.

**Exact initialization order:** retain the existing
OnEnter(GamePhase::InProgress) chain in server_app/registration.rs through
seed_ship_power_counter and spawn_game_start_entities. Insert an explicit deferred
application boundary and the Test-only exclusive constructor after that spawn,
before dump_tracked_entities and before the selected hull's first FixedUpdate
Input/Physics work. Test selection targets the actual frozen authored slot, never
an arbitrary EntityUuid or whichever entity happens to be LocalShip in a peer.
Beam/Reactor fitting does not change power_rating or the existing ship-power
scenario seed. The first reference permits no spawn-time hook that consumes this
hull's machinery before initialization; an unsupported hook/order is a refusal.

Resolve exactly one actual target with all required installed owners. A missing,
duplicate, already-worked or differently bound target refuses the rehearsal.
Never fix a running child by reapplying a favourite after an initial operating
frame. Initialization itself spends no time, makes no observations and executes
no tree/weapon work.

Build a candidate from this child's original compiled profile and actual initial
FittingState. Change only the hypothetical installed definitions and matching
desired selections for the chosen compatible slots; retain existing child-local
assembly identities/locations and unrelated initial stocks/conditions. No assembly
IDs or physical work stages come from the request. This is explicitly declared
initial hypothetical equipment, not a claim of owning spares in the real game.

Validate the complete candidate, clone the actual initial PowerSystem and use
fitting_server::stage_capability / PowerSystem::reconfigure_energy and the same
apply_capability adapters. Preserve buffer capacities, charge, heat, coolant
credit, rooms, Consoles, sources and the ordinary physical damage owner. Stage
all fallible checks before committing actual ShipFitting, power and capability
sections once. Keep the original CompiledShipFitting/source identity for ordinary
continuation. No new request/refit job is queued, and no initial reserve is read
from the selected equipment to refill a buffer.

On failure, leave the child's entire already-created baseline machinery, equipment,
Stocks, observations, RNG and mint unchanged. Only the private Test error/result
status may change; retire the failed child before any machinery step. Do not
silently fall back to a default fit and label that as the requested rehearsal.
The parent/source document and any Live host remain unchanged on every path.

## Actual trials and displayed assumptions

A supported reference profile binds its accepted World/controller setup and a
closed finite phase schedule, not user code. Maximum eight phases, at most 600
warm-up and 3,600 measured fixed intervals, positive measured duration and a total
maximum of 4,200 intervals. Use the World-authored fixed rate and existing Test
clock; no second timebase or wall-clock duration drives machinery. Operators
select a named profile, not arbitrary duty numbers or scripts.

Display the requested active Systems and phase duty, authored initial reserve
charge/heat/coolant/ammunition, reactor/allocation/cooling modes, seed, source
revision and interval counts. These assumptions are real accepted initial data
and ordinary supported control intent. Subsequent changes go through the existing
System admission and ordinary individual AI. Report actual achieved activity
separately: an unfunded actuator or unacquired target cannot be counted as active
merely because a phase requested it.

The sustained profile requires actual ordinary acquisition and firing against
its real accepted reference subject. The burst/recovery profile requires real
work and subsequent ordinary recharge/cooling. Authored policies or closed normal
admitted inputs may set those supported intents; no test adapter calls
work_continuous, advances beam timers, fills a reserve, inserts a target contact
or applies synthetic damage to manufacture output. Each System still has only
its existing Human/AI controller. Reference target durability and duration are
authored finite tuning; they cannot grant invulnerability or hide destruction.

Compare default and chosen fit in two fresh children from the same immutable
capture, profile and seed. Never clone the final baseline into the selected run,
reuse its warmed reserves/knowledge or rewind one running child. Record complete
assumptions and result identities for both. A legally fitted but unsustainable
combination remains selectable and is reported honestly.

## Exact measurement boundary and result identity

Install the read-only sampler in **FixedLast before sim_tick::advance_sim_tick**.
FixedUpdate and its existing SimDispatch/capacity settlement/deferred application
have completed. Read the actual owners there, while current work receipts still
belong to this interval and before the next begin_work_interval clears them.
Do not sample a rendered frame, an earlier Publish board or a queued coolant
adjustment as if it had already settled.

StateTransition may create the hull between FixedUpdate and FixedLast. Capture
its initial baseline at construction, but do not count that same initialization
tick as completed hull work. Count only subsequent eligible fixed intervals after
initialization; paused/render/lobby frames add no samples or elapsed work. Pin
same-tick idempotence in the private derived report. If required capacity work is
unexpectedly still pending at this boundary, refuse that result rather than
inventing a second stock application or speculative coolant accounting.

Freeze exactly the requested warm-up/measured window, with last measured SimTick,
step count and original fixed period. Subsequent ordinary child steps never
extend or refresh a completed report. Request pause through the existing private
TestClock control owner; do not introduce another machinery clock or promise the
render frame stopped at the sampled tick. Early actual termination/loss or lack
of required activity is labelled incomplete/insufficient activity, not zero-cost
success or a completed endurance forecast.

The private TestStatus result binds capture revision, accepted hull content_id,
normalized plan choices, workload revision, seed, selected slot and run generation.
Retire previous results immediately on plan/profile/source/slot/seed replacement
or disposal. An old child reply cannot paint into a replacement comparison.
Bound report data to 128 chart samples plus finite aggregate counters/minima/maxima
and 32 KiB encoded rehearsal result. The complete private status record must also
fit the existing 64 KiB framing limit. Choose deterministic aggregation/downsampling
before encoding; if either byte bound still fails, emit a small explicit
result-too-large refusal, never silently truncate a successful result or oversize
the pipe. This is disposable TestInfra and read-only presentation, not a canonical ship
history, save payload or second resource account.

Report actual interval paid energy using PowerSystem::emitted_work, actual reactor
output and net reserve change separately. Include reserve minima/depletion,
observed heat maxima/trip/recovery, real coolant Stock plus fractional-credit
change, observed flow cuts, achieved active duty and supported actual capability
output. last_work_fraction is a last per-System receipt, not an aggregate requested
demand accumulator. Do not derive exact unmet whole-hull demand or remaining
lifetime by pretending otherwise. Label nominal authored damage/range/cycle values
separately from measured real effects. No steady-state or universal DPS claim is
made from a bounded trial or a target that was never acquired.

## Proof required before completion

- Execute the actual accepted-loader/compiler path for the complete base and
  alternative before adding a host tracer. Assert exact six-seat ownership,
  original physical/energy completeness and compatible Beam/Reactor choices.
- Prove initial default parity, a real selected sidegrade and real reactor change
  through the shared child constructor, with unchanged charge/heat/Stocks and no
  first-tick default operation. Reject stale hashes, bad choices, late target,
  partial owner graph and invalid staging with full baseline no-write evidence.
- Run actual paid reference firing/recovery, hot or constrained cases and finite
  coolant depletion. Show requested versus achieved work and no invented output;
  source unit formulas or manually supplied receipts are not trial evidence.
- Verify fixed-tick sampling at initialization, paused/no-step, same interval,
  multi-step frames, exact final window and pending-resource refusal. Compare
  deterministic fresh child runs and reject stale replies on disposal/replacement.
- Exercise the existing native/browser captured-source Test launcher and actual
  planner action. Show document/history, parent preferences, live machinery,
  authority and saves unchanged after success, cancellation and failed launch.
  Device/native acceptance remains distinct from source/mocked UI tests.

The contract itself adopted no runtime or compatibility epoch. The source
checkpoint below implements its first consumer; it does not establish full
increment13 completion. The complete native frame, including status prefix and
line terminator, satisfies the same explicit bound in the shared encoder.

## First consumer source checkpoint

The private Workshop Test panel now mounts the existing favourite planner with a
Rehearse action. The ordinary phone and lobby widgets receive no private Test
callback. One comparison freezes the document before yielding, then uses the
existing serial Test session to create a fresh default child and a fresh selected
child. Its existing immutable dependency provider supplies captured bytes;
different actual source/workload identities refuse the comparison. Positive
private run generations and exact report bindings prevent late results painting
over source, plan, profile, seed or slot replacement. Cleanup failure releases
the capture and controls and retains the error.

`workshop/rehearsal_request.rs` decodes closed favourite/section/choice records,
including duplicate-map refusal. `rehearsal.rs` re-admits the actual captured
World and include-resolved Cruiser, verifies the original planner content_id and
frozen slot, and stages actual FittingState and Power plus the read-only initial
report before its single commit. Browser capture and the native private staged
reader call the same admission. Refused initialization pauses and retires through
the existing TestClock owner; Resume cannot operate a default fit under the
requested favourite's label.

The two actual reference Worlds contain a Test-only `test_rehearsal` declaration
with strict workload/warmup/phase fields. Their first supported setup is one
controlled Cruiser slot at 60 Hz, 120 warmup intervals and 1,800 measured intervals.
The sustained phase requests engagement throughout. Burst/recovery requests 600
engagement and 1,200 recovery intervals; at twelve ordinary scenario seconds the
World retires its subject through the existing destruction effect. This is an
explicit workload boundary, never damage or a kill credited to the fitting.
The subject uses the complete accepted finite unarmed utility hull, its ordinary
empty doctrine/idle policies and a stationary collider. It retains all nine real
Systems, their Operation hardware, reserves and complete physical placements;
only its declared identity/faction, collider mobility and finite compartment HP
differ from that hull. It uses the ordinary behaviour/ship
substrate and the existing Pirate relationship; its one room has 10,000 finite
structural HP. It has no invulnerability or replenishment grant.

The TestInfra sampler runs in FixedLast before the existing SimTick advance. It
reads real current-interval work receipts, reactor output, reserve/thermal state,
flow cuts and settled coolant Stocks plus fractional credit. Initial assumptions
and nominal selected definitions are retained separately; measured start/minima
begin after warmup. Observed subject HP loss is separate from retirement/loss.
No measured fore-beam work is reported as insufficient activity. Frozen charts
use a fixed interval stride and at most 121 points, with an explicit 32 KiB result
and complete 64 KiB native prefix/JSON/newline refusal.

Source review: Comms reviewed the Rust constructor, sampler and both adapter
joins; Personnel reviewed the comparison, private UI and rejecting-stop cleanup.
Targeted JavaScript checks and static import/string checks pass. Rust accepted
World/compiler, atomic constructor, settled sampler and full-frame owner tests
are authored but unexecuted. Root owns Cargo and must execute the accepted-loader
gate before a whole-host tracer. Actual paid firing/recovery, hot/depleted cases,
full child continuation, independent fresh-run results and browser/native/device
acceptance are still required. These are remaining verification, not implied by
the source reviews or mocked private-provider tests.
