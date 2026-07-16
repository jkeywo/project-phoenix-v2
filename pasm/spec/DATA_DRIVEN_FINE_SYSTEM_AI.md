# Data-Driven Fine-System AI Policy Specification

Status: accepted design; implementation proposed.

This document is the normative authoring and runtime contract for replacing
Project Phoenix's hard-coded and top-level `[behaviour.doctrine]` AI with
inline, data-driven policies owned by individual fine systems.

The architecture declarations for this contract live in
`architecture/data-driven-fine-system-ai.yaml`. Existing runtime truth remains
described by the current subsystem slices until the migration is complete.

## 1. Goals

The policy system must:

- express all existing NPC and Backfill AI through one format;
- keep each fine system's policy and state independent;
- emit the same typed system inputs available to human operators;
- support simple reactive rules without forcing every system to have states;
- support persistent movement phases where state is necessary;
- make target selection and ranking data-driven per fine system;
- reuse and extend the world-trigger predicate language;
- keep every designer-tunable threshold, duration, margin, and weight in TOML;
- evaluate deterministically on the authoritative host; and
- validate authored policies before a world activates.

The policy system must not:

- introduce a ship-wide combat state machine;
- allow one fine system to inspect another system's private policy state;
- allow policies to mutate world flags or counters directly;
- allow arbitrary ECS component writes, arbitrary messages, or scripts; or
- preserve `[behaviour.doctrine]` as a permanent second AI authoring model.

## 2. Ownership and placement

AI policy is authored inline on the `[[system]]` entry that owns the operated
capability. Separate policy assets are not part of this design.

Executable movement policy belongs only to actuator fine systems such as
engines, steering, lateral thrust, vertical thrust, and boost. There is no
coarse `helm` system to own an AI policy — issue #801 deleted it, and `helm`
is a station id only. The shared Helm motion and hazard layer is a
stateless compositor and safety service, not a combat behaviour controller.

Every system that can receive `ControlSource::Ai` must declare one of:

- an inline `[system.ai]` policy; or
- an explicit idle policy.

Missing AI configuration on an AI-capable system is a content error. An idle
declaration distinguishes intentional inactivity from an omitted policy.

Illustrative shape:

```toml
[[system]]
id = "helm-steering"
kind = "steering"
ai_policy = "inline"

[system.ai]
initial_state = "attack"
evaluate_every_ticks = 1

[system.ai.parameters]
reengage_shield_fraction = 0.75
safe_range_margin = 20.0
pressed_window_secs = 2.0
minimum_escape_progress = 5.0
```

Exact TOML deserialisation may use equivalent nesting required by repeated
`[[system]]` tables, but the ownership and semantics above are normative.

## 3. Policy forms

All policies use the same evaluator. States are optional.

### 3.1 Stateless reactive policy

A stateless policy continuously evaluates prioritised rules. Torpedo loading
and firing is the canonical example: the tubes do not need artificial
`loading` and `firing` states when readiness, arc, target shield, and in-flight
projectiles already exist as authoritative facts.

```toml
[[system.ai.rule]]
channel = "launch"
priority = 100
when = "tubes_full(self) and shield_fraction(target:facing) <= 0 and target_in_arc(self, target)"
verb = "launch_salvo"
```

### 3.2 Stateful policy

A stateful policy declares one initial state and zero or more named states.
States contain continuous rules and prioritised outgoing transitions. A state
with no outgoing transition is a valid terminal state and produces no lint
finding.

At most one transition may fire for a fine-system policy in one AI tick. The
new state becomes active immediately, but its outgoing transitions are not
evaluated until the next eligible tick.

Continuous intents belong to states. One-shot typed commands and local-memory
changes belong to transitions. The format does not provide general entry or
exit scripts.

```toml
[[system.ai.state]]
id = "recovery_orbit"

  [[system.ai.state.rule]]
  channel = "facing"
  priority = 10
  when = "target_valid(target)"
  verb = "orbit_target"

  [[system.ai.state.transition]]
  to = "reentry_pivot"
  priority = 100
  when = "shield_fraction(self) >= parameter(reengage_shield_fraction)"

    [[system.ai.state.transition.set_memory]]
    name = "orbit_direction"
    value = "random(clockwise, anticlockwise)"
```

Transition priorities are explicit. If two outgoing transitions from the same
state have equal priority and can compete, content validation fails rather than
using TOML order. Only the highest-priority matching transition fires.

### 3.3 Output channels

Rules declare a typed output channel. The highest-priority matching rule for
each channel wins. Equal-priority rules competing for the same channel are a
content error. Rules on different channels may produce outputs in the same
tick, such as one facing intent and one travel intent.

Transition outputs take precedence over continuous state outputs on the same
channel for the transition tick.

## 4. Local state and memory

Policy state and memory are private to the owning fine system. Another policy
cannot query them. Cross-system coordination uses public authoritative facts
such as selected targets, waypoints, shield state, boost state, weapon
readiness, projectiles in flight, motion, damage history, or hazard assessment.

Policies may declare bounded typed memory:

- boolean;
- integer;
- float;
- entity reference; and
- enum with an authored closed value set.

Transitions may mutate only declared memory. Policies cannot create variables
at runtime. If a new cross-system fact is required, it must be added as a typed
public fact owned by the relevant system or blackboard; a policy may not publish
an arbitrary shared AI fact.

Random memory choices are deterministic. Their seed derives from the world
seed, ship UUID, fine-system ID, transition identity, and transition occurrence
count. This permits different choices between ships and manoeuvres while
keeping tests and replays reproducible.

## 5. Expressions and supported inputs

Policy `when` expressions extend the existing world-trigger predicate grammar.
They retain parentheses, `and`, `or`, `not`, and comparison operators. The
parser and typed AST are shared; runtime contexts register the functions they
support.

World policies retain `flag()` and `counter()`. AI policy additionally exposes
typed AI facts. Unknown functions, unknown arguments, invalid context
references, and incompatible comparisons are content errors.

### 5.1 Context references

- `self` is the ship or fine system currently evaluating, as required by the
  called function.
- `candidate` is bound only while a target candidate is being filtered or
  scored.
- `target` is the fine system's currently selected target.

Context references are explicit. Their meaning must not change implicitly
between selector and state expressions.

### 5.2 Required fact families

The initial runtime must expose registered functions covering these existing
authoritative inputs:

- **Scenario:** read-only `flag(name)` and `counter(name)`, including existing
  parent-layer lookup semantics.
- **Entity:** identity, tags, faction, hostility, threat, authored
  `power_rating(entity)`, alive/valid state, and objective membership.
- **Objectives:** active scored objectives, directive affinity, resolved
  objective targets, anchors, and mandatory/priority information.
- **Contacts and targets:** detectable contacts, effective system horizon,
  Sensors target, Tactical target, recent attackers, and per-system selected
  target.
- **Navigation:** the authoritative shared waypoint and its live entity anchor
  where present.
- **Motion:** positions, velocity, relative bearing, distance, range bands,
  closest approach, separation progress, and current desired-motion/hazard
  facts.
- **Ship capability:** supported actuators, movement mode, authored limits,
  boost/impulse state, and system availability.
- **Shields and damage:** facing shield arc, shield fraction, collapsed state,
  regeneration state, recent incoming damage, hull state, and recent combat
  activity.
- **Weapons:** target arc/range, bank or tube readiness, magazine availability,
  loaded fraction, cooldown, active beam/volley state, and matching projectiles
  in flight.
- **Policy runtime:** current state time, policy tick, deterministic local
  memory, and registered events.

The API should expose domain facts rather than hard-coded hull categories. A
policy asks for facts such as firing arc, size rating, threat, or shield state;
it does not branch on a Harrow-specific Rust enum.

### 5.3 History operators

The expression runtime provides bounded history over facts explicitly marked
history-capable:

- `time_in_state()`;
- `time_since(event)`;
- `delta(fact, duration)`;
- `sum(fact, duration)`; and
- `average(fact, duration)`.

Durations may come from named parameters. The loader calculates the largest
required window so runtime history remains bounded. These operators support
existing needs such as failed separation progress, recent shield damage, and
recent combat activity.

### 5.4 Ship power rating

`power_rating(self)`, `power_rating(candidate)`, and `power_rating(target)`
expose the authored ship-level rating used by scenario scaling. This is not
allocated or effective subsystem power. World scripts may continue to use
`counter(ship_power)` where a scenario intentionally captures the selected
player ship's rating.

## 6. Parameters

Designer-tunable thresholds, durations, margins, ranges, fractions, and score
weights are named TOML parameters. Expressions reference them through
`parameter(name)`. Structural comparisons with values such as zero may use a
literal.

Policies are inline, so parameters are local to the owning system policy. The
runtime must not supply hidden gameplay defaults that override authored values.
Parsing defaults are permitted only where the ship schema explicitly defines
them.

## 7. Data-driven target selection

Any fine system may declare a target selector. Sensors is expected to rank a
good combat contact using objectives and threat. Other systems may copy the
Sensors selection by strongly favouring it, but each retains its own authority
and eligibility checks. In particular, Sensors designation remains advisory;
Tactical independently writes its authoritative firing target.

### 7.1 Candidate sources

A selector declares one or more registered typed sources. The initial set must
cover:

- sensor contacts;
- Tactical contacts;
- resolved objective targets;
- recent attackers;
- current Sensors and Tactical selections; and
- selectable Navigation entities.

Sources are unioned and deduplicated by entity UUID before eligibility and
scoring. A source does not bypass system constraints: Sensors must still reject
a contact outside its damage-scaled horizon even when an objective names it.

### 7.2 Eligibility and scoring

Each candidate is first evaluated against `eligible_when`. Eligible candidates
receive additive utility from prioritised or independent score rules:

```toml
[system.ai.targeting]
candidates = ["sensor_contacts", "objective_targets", "recent_attackers"]
eligible_when = "hostile(candidate) and detectable(candidate)"
switch_margin = "parameter(target_switch_margin)"

[[system.ai.targeting.score]]
when = "objective_target(candidate)"
add = "parameter(objective_target_bonus)"

[[system.ai.targeting.score]]
when = "candidate_is(sensors_target())"
add = "parameter(sensors_designation_bonus)"
```

Target utility is system-specific. Sensors, Tactical, Helm actuators, and other
systems may use different filters and weights over the same candidates.

### 7.3 Retention and invalidation

Selectors continuously rescore on eligible policy ticks. The current valid
target remains selected unless a challenger exceeds its score by the authored
`switch_margin`. Equal scores use stable entity UUID ordering as the final
tie-breaker, never query iteration order or unseeded randomness.

An invalid target is cleared immediately. Destruction, changed faction
validity, loss of required visibility/horizon, or other eligibility failure
bypasses the switch margin. The best eligible replacement may be selected in
the same tick.

## 8. Supported outputs

Every fine-system kind registers a closed catalogue of typed AI verbs and
output channels. Configuration validation rejects a verb that the owning
system kind cannot perform.

The catalogue must be capable of representing all existing AI-controlled
systems, including:

- engines, steering, lateral thrust, vertical thrust, boost, and impulse;
- phaser banks, blaster banks, torpedo tubes, Tactical targeting, Tactical
  radar, and torpedo magazines;
- Shields arc focus;
- Sensors target designation and frequency advice;
- data-defined Power group allocations;
- Captain Red Alert and objective priority;
- Navigation waypoint selection;
- Comms actions; and
- Repair priorities and team dispatch.

Accepted verbs produce the same typed system inputs used by a human operator.
The policy runtime cannot mutate simulation components directly. Human and AI
inputs converge before downstream simulation logic, availability checks, and
damage gating.

## 9. Scheduling and evaluation

AI uses an authoritative deterministic fixed-rate base tick aligned with
Helm's control rhythm. The shipped precedent is the shared AI-helm sim tick
(issue #803): one TOML-authored rate — `[global] ai_helm_tick_hz`, default
30 Hz — gates every per-axis AI helm system, decoupling AI decisions from the
host frame rate. Each fine-system policy declares an integer
`evaluate_every_ticks`; omitted values may use the schema's documented parsing
default. Systems such as Sensors, Power, Repair, or Comms may therefore run less
often without introducing frame-dependent timers.

For every eligible fine-system tick, evaluation order is:

1. capture one immutable authoritative fact snapshot;
2. validate or select the fine system's target;
3. evaluate at most one state transition;
4. apply transition-local memory changes;
5. evaluate continuous rules from the resulting state;
6. resolve transition and continuous outputs per channel, with transition
   outputs winning their channels for that tick; and
7. emit typed system inputs.

All fine-system policies evaluate from the same tick snapshot. Their outputs
are applied as a batch. A Sensors target change made this tick becomes visible
to Tactical, Helm, or other policies on the next AI tick. Explicit Coordination
messages retain their authored channel latency.

## 10. Lifecycle

A policy evaluates only while its fine system's Control Source is `Ai` and the
system is available.

- When a fine system gains AI control, its policy resets to the authored initial
  state and initial memory.
- While Human or Offline controls it, the policy does not evaluate or mutate
  memory.
- If an AI-controlled system becomes unavailable through damage, evaluation
  stops.
- When that system recovers, its policy resets before resuming.

This prevents Backfill or repaired systems from resuming stale manoeuvres or
timed sequences.

## 11. Validation

Deterministic structural and type failures block entity/world loading before
the root world activates. Required checks include:

- missing policy or explicit idle declaration on an AI-capable system;
- unknown system kind, state, transition target, function, parameter, memory
  slot, candidate source, output channel, or verb;
- invalid argument or comparison types;
- missing or invalid initial state;
- competing equal-priority transitions;
- competing equal-priority rules on one output channel;
- writes to undeclared memory;
- direct writes to scenario flags/counters or arbitrary public facts;
- use of a verb not owned by the fine-system kind; and
- invalid evaluation interval or history window.

A state unreachable from the initial state produces a warning. A state with no
outgoing transition is valid and produces no warning. PASM and content lint
must not manufacture behavioural certainty beyond these deterministic checks.

## 12. Combat Test movement policies

The following behaviours are the accepted initial content that exercises this
policy model. Exact values are authored in the relevant entity TOML.

### 12.1 Harrow destroyer

- Uses forward-facing blasters; remove its torpedoes.
- Makes a normal-speed attack pass while Steering continuously turns toward the
  moving target.
- Closest approach ends target tracking for that pass. Engines preserve the
  current outward heading, modified only by shared hazard avoidance, and Boost
  drives the escape.
- Safe distance is the selected player ship's longest usable direct-fire range
  plus an authored margin.
- At safe distance, it enters a randomly chosen clockwise or anticlockwise
  recovery orbit and spirals to maintain that dynamic distance.
- NPC shields may recover from zero after an authored no-damage delay. The
  destroyer remains in recovery until both safe distance is maintained and
  shields reach the authored re-entry fraction, initially `0.75`.
- Normal re-entry cuts thrust, pivots without boost, then begins another
  normal-speed attack pass.
- If recovery fails to increase separation by the authored minimum over the
  authored history window while inside player threat range, the relevant
  actuator policies independently enter pressed behaviour.
- Pressed behaviour abandons shield recovery, performs shorter normal-speed
  passes, and uses boost while stationary to increase pivot yaw. It does not
  boost the inbound attack leg.

### 12.2 Harrow cruiser

- Chooses a random clockwise or anticlockwise orbit when combat begins.
- Maintains a continuous broadside orbit near authored range, spiralling inward
  or outward for range correction. It does not stop or face directly inward for
  ordinary range correction.
- Carries fore and aft phaser banks with 270-degree arcs; their overlap lies on
  the port and starboard broadsides.
- Torpedo launch is stateless: all tubes must be full, the target's facing
  shield must be down, and the target must be in arc.
- Independently, Steering cuts thrust and pivots bow-on when the target's facing
  shield is down. It tracks the moving player while torpedoes remain in flight
  or the shield remains down.
- The attack aborts before launch if the shield recovers. Once a salvo launches,
  Torpedoes remains committed until its projectiles hit, miss, or expire.
- While bow-on, the fore phaser continues shield pressure.
- After the salvo resolves and the shield has recovered, the cruiser resumes
  its orbit and rerandomises orbit direction.

### 12.3 Harrow battleship

- Acts as a stationary long-range artillery platform after entering its
  authored range.
- Its powerful, slow, bow-mounted blaster uses predictive non-homing fire. The
  firing solution predicts current motion; the projectile does not update after
  launch, so changing speed or direction evades it.
- It holds position if the player closes, relying on strong short-range phaser
  coverage and fore/aft torpedo launchers rather than retreating.
- Torpedoes fire opportunistically when a loaded launcher already bears on a
  target whose facing shield is down. Torpedoes never override bow-artillery
  facing.
- Repositioning begins when the target moves outside maximum artillery range
  and stops at 90 percent of that range, providing range hysteresis.

### 12.4 Shared movement rules

- Enemy ships run their policies independently; there is no formation or squad
  movement state.
- Hazard avoidance has limited priority. It may bend travel to avoid collision
  without changing the current combat state. Only imminent collision may
  temporarily override facing.
- Combat doctrine remains planar. Bounded vertical motion is used only for
  avoidance, after which ships gradually return to their authored cruise plane.
- Destroyer, cruiser, and battleship tuning belongs to their TOML system
  policies and weapon/shield configuration, not Rust hull branches.

## 13. Migration

Implementation must migrate every existing AI operator to inline fine-system
policy, not only the Combat Test enemies. The migration sequence is:

1. implement parsing, validation, expression contexts, scheduling, targeting,
   state/memory, and typed verb registries;
2. provide typed public facts currently read directly by hard-coded AI;
3. convert existing AI systems one fine-system kind at a time;
4. convert NPC entity `[behaviour.doctrine]` blocks into inline system policy;
5. remove direct-write AI paths such as the monolithic Helm doctrine executor;
6. remove the legacy `[behaviour.doctrine]` parser and data once no entity uses
   it; and
7. retain no permanent compatibility mode or second policy evaluator.

During migration, temporary adapters must still respect Control Source,
system availability, authoritative typed inputs, and the immutable tick
snapshot. They must have explicit removal conditions in PASM.
