# Writing PASM

**Version:** 1.0

## 1. Purpose of This Guide

This guide explains how to author PASM model files.

It covers:

- file organisation;
- entity declarations;
- architecture sections;
- game-design sections;
- implementation sections;
- evidence;
- migrations;
- exceptions;
- uncertainty;
- writing for useful validation and LLM reasoning.

The first source format should be a restricted YAML subset.

## 2. Authoring Principles

### 2.1 Write intent, not a transcription of code

Bad:

```yaml
component: damage-rs-file
```

Better:

```yaml
component: damage-resolution
```

The implementation path belongs in `implementation`, not in the identity.

### 2.2 Prefer testable declarations

Weak:

```yaml
goals:
  - handle networking well
```

Better:

```yaml
architecture:
  must_not_own:
    - authoritative-world-state
  accepts:
    - validated-client-command
  sends:
    - replicated-world-update
```

### 2.3 Separate current, target, and temporary state

Do not describe a half-finished migration as if it were already complete.

Use a migration entity and explicit temporary exceptions.

### 2.4 Record why a boundary exists

A rule without rationale is easier to remove accidentally.

### 2.5 Do not formalise uncertainty as certainty

Use `provisional`, `inferred`, `disputed`, and `open_questions`.

### 2.6 Avoid duplicate truth

If an entity already exists, extend it rather than creating a parallel version for another domain.

## 3. File Organisation

Recommended structure:

```text
spec/
    core/
    architecture/
        runtime/
        networking/
        simulation/
        presentation/
        platform/
    design/
        roles/
        mechanics/
        information/
        failures/
    implementation/
    migrations/
    scenarios/
    evidence/
```

Organise primarily by subsystem where that improves maintainability.

A file may contain one entity or several closely related entities.

## 4. Basic Entity Form

```yaml
entities:
  - component: engineering-station

    core:
      title: Engineering Station
      status: accepted
      confidence: confirmed
      summary: Player-facing engineering interface and interaction boundary.
      goals:
        - let engineering diagnose and manage ship systems
      rationale:
        - engineering should own repair prioritisation
```

The declaration key identifies the kind and ID:

```yaml
component: engineering-station
state: authoritative-damage-state
message: diagnose-fault-command
migration: damage-model-v2
player_role: engineering
```

## 5. Core Section

Suggested fields:

```yaml
core:
  title: Engineering Station
  status: accepted
  confidence: confirmed
  summary: ...
  goals:
    - ...
  rationale:
    - ...
  tags:
    - bridge-station
  references:
    - host-simulation
  assumptions:
    - one player occupies the station
  open_questions:
    - should diagnosis consume time or power?
```

### 5.1 Status

Use:

```text
proposed
provisional
accepted
partially-implemented
implemented
deprecated
removed
rejected
```

### 5.2 Confidence

Use:

```text
confirmed
inferred
provisional
disputed
unknown
```

### 5.3 Rationale

Rationale should explain the design or architectural reason, not restate the rule.

Bad:

```yaml
rationale:
  - clients do not own authoritative state because they do not own authoritative state
```

Better:

```yaml
rationale:
  - prevents divergent simulation and client-side cheating
```

## 6. Architecture Section

Example:

```yaml
architecture:
  kind: client-interface

  owns:
    - engineering-ui-state

  reads:
    - known-damage-state
    - power-state

  writes:
    - engineering-ui-state

  sends:
    - diagnose-fault-command
    - reroute-power-command

  receives:
    - engineering-state-update

  depends_on:
    - client-network-interface

  must_not_depend_on:
    - host-damage-resolution

  runs_in:
    - browser-client
    - native-client

  authority: non-authoritative
```

### 6.1 Ownership

Use `owns` only where the component is responsible for the lifecycle and truth of the state.

Use `reads`, `writes`, `replicates`, or `derives` for weaker relationships.

### 6.2 Forbidden Dependencies

Declare only boundaries that matter.

Example:

```yaml
must_not_depend_on:
  - browser-dom
```

A forbidden dependency should have a rationale in the same entity or a referenced invariant.

### 6.3 State Declarations

```yaml
- state: authoritative-damage-state

  core:
    status: accepted

  architecture:
    classification: authoritative
    owner: host-simulation
    writers:
      - damage-resolution
    replicas:
      - client-damage-summary
```

### 6.4 Derived and Player-Knowledge State

```yaml
- state: known-damage-state

  architecture:
    classification: player-knowledge
    owner: engineering-knowledge-model
    derived_from:
      - authoritative-damage-state
    reveal_conditions:
      - successful-diagnosis
```

### 6.5 Messages

```yaml
- message: diagnose-fault-command

  architecture:
    producer:
      - engineering-station
    consumer:
      - host-command-router
    authority: request
    trust_boundary: client-to-host
    validator:
      - engineering-command-validator
    version: 1
```

### 6.6 Platform Constraints

```yaml
platforms:
  allowed:
    - browser
    - native
  forbidden:
    - host-only
```

## 7. Game Design Section

Example:

```yaml
game_design:
  player_role: engineering

  responsibilities:
    - diagnose ship faults
    - prioritise repairs
    - manage power distribution

  player_verbs:
    - diagnose
    - reroute
    - repair
    - prioritise

  protected_decisions:
    - repair-priority
    - power-allocation

  visible_information:
    - known-damage-state
    - current-power-state

  hidden_information:
    - undiagnosed-fault-detail

  coordination_with:
    - command
    - tactical

  experience_goals:
    - sustained operational pressure
    - consequential prioritisation
```

### 7.1 Protected Decisions

Protected decisions should identify the decision, owner, and prohibited bypasses.

```yaml
- action: choose-repair-priority

  game_design:
    owner_role: engineering
    protected: true
    must_not_be:
      - fully-automated
      - committed-by-command
```

### 7.2 Information Visibility

```yaml
- information_set: undiagnosed-fault-detail

  game_design:
    visibility: hidden
    permitted_viewers: []
    reveal_condition:
      - successful-diagnosis
    indirect_signals:
      - subsystem-performance-drop
```

### 7.3 Mechanics

```yaml
- mechanic: diagnose-fault

  game_design:
    participating_roles:
      - engineering

    inputs:
      - target-subsystem

    reads:
      - authoritative-damage-state
      - engineering-tool-state

    changes:
      - known-damage-state

    costs:
      - diagnosis-time

    resolution:
      - host-authoritative

    information_revealed:
      - fault-type
      - repair-requirement
```

### 7.4 Resources

```yaml
- resource: ship-power

  game_design:
    sources:
      - reactor-output
    sinks:
      - weapons
      - engines
      - shields
    capacity: bounded
    owner_role: engineering
    pressure_intent:
      - force competing priorities
```

### 7.5 Failure and Recovery

```yaml
- failure_state: coolant-loop-failure

  game_design:
    causes:
      - accumulated-reactor-damage
    consequences:
      - rising-reactor-heat
    visible_to:
      - engineering
    terminal: false
    recovery_paths:
      - repair-coolant-loop
```

## 8. Implementation Section

The Implementation Model describes where the entity is intended to be implemented.

```yaml
implementation:
  paths:
    - crates/client/src/engineering
  symbols:
    - EngineeringPanel
    - EngineeringCommand
  messages:
    - DiagnoseFaultCommand
  tests:
    - diagnosis_hides_unknown_faults
  status: declared
```

Mappings may refer to:

```text
repository
crate
package
module
file
directory
type
trait
function
method
message
event
route
entry-point
worker
thread
test
configuration
asset
```

Do not use implementation paths as entity IDs.

## 9. Evidence Section

```yaml
evidence:
  - kind: test
    reference: diagnosis_hides_unknown_faults

  - kind: manual-review
    summary: Confirmed host owns actual damage state.

  - kind: playtest
    reference: playtest-2026-06-engineering-pressure
```

Evidence should support a specific claim.

Avoid generic evidence links with no stated relationship.

## 10. Migrations

Example:

```yaml
- migration: damage-model-v2

  core:
    status: partially-implemented
    confidence: confirmed

  architecture:
    replaces:
      - legacy-damage-model

    target:
      - authoritative-damage-state
      - known-damage-state

    temporary_adapters:
      - legacy-damage-adapter

    permitted_legacy_callers:
      - legacy-debug-panel

    removal_conditions:
      - no-observed-imports: legacy-damage-model
      - symbol-does-not-exist: LegacyDamageState

  implementation:
    legacy_paths:
      - crates/simulation/src/legacy_damage.rs
    target_paths:
      - crates/simulation/src/damage
```

A migration should not be marked complete merely because the new path exists.

## 11. Exceptions

Example:

```yaml
exceptions:
  - rule: shared-code-must-not-access-browser-api
    scope:
      - clipboard-adapter
    rationale: Clipboard support has no platform-neutral equivalent.
    temporary: false
    approval_status: accepted
```

Temporary example:

```yaml
exceptions:
  - rule: new-networking-must-not-call-v1
    scope:
      - legacy-lobby-client
    rationale: Lobby migration is not complete.
    temporary: true
    removal_condition:
      - lobby-client-uses-v2
```

## 12. Invariants

A declarative invariant may look like:

```yaml
- invariant: clients-do-not-own-authoritative-world-state

  core:
    status: accepted
    rationale:
      - maintain deterministic host authority

  architecture:
    subject:
      - client-runtime
    prohibition:
      - owns: authoritative-state
```

Complex invariants may be implemented in Python and referenced by ID.

## 13. Scenarios

Scenarios support simplified reasoning and reachability checks.

```yaml
- scenario: undiagnosed-reactor-fault

  game_design:
    initial_state:
      - reactor-fault: present
      - reactor-fault-known: false

    available_roles:
      - engineering
      - command

    expected:
      - engineering-has-meaningful-choice
      - command-cannot-repair-directly

    forbidden_states:
      - reactor-fault-known-without-diagnosis
```

The scenario model should not reproduce the full game implementation.

## 14. Writing for LLM Reasoning

Good PASM declarations are:

- specific;
- scoped;
- linked;
- explicit about ownership;
- explicit about exceptions;
- explicit about uncertainty;
- supported by evidence;
- compact enough to load selectively.

Avoid:

- vague prose;
- generic statements;
- duplicated entities;
- hidden assumptions;
- comments that describe temporary behaviour without a migration;
- hard-coded implementation details in conceptual IDs.

## 15. Authoring Checklist

Before accepting an entity, check:

- Does it have a stable semantic ID?
- Is its status accurate?
- Does it state why it exists?
- Are ownership and authority explicit?
- Are forbidden dependencies meaningful?
- Are game-design claims linked to mechanics or evidence?
- Are temporary deviations represented as migrations or exceptions?
- Are implementation mappings current?
- Are unresolved questions marked?
- Can a validator or audit use the declaration?

If the declaration cannot support reasoning, validation, migration control, or context generation, it may not belong in PASM.
