# PASM Core Concepts

**Version:** 1.0  
**Project:** Project Phoenix  
**Name:** Phoenix Architecture & System Model

## 1. Purpose

PASM is an executable system model for Project Phoenix.

It records:

- how the system is intended to be structured;
- why architectural boundaries exist;
- how runtime authority and state ownership are assigned;
- how game-design intent should be expressed by the software;
- where implementation is expected to live;
- which migrations are in progress;
- what evidence supports the current claims;
- how the observed repository differs from the intended system.

PASM exists because software can compile, pass tests, and appear functional while still violating its intended architecture or design.

Typical failures include:

- old and new implementations operating in parallel;
- compatibility layers becoming permanent;
- presentation code acquiring simulation responsibilities;
- clients mutating authoritative state;
- duplicated sources of truth;
- browser and native paths drifting apart;
- functions partially converted to a new data model;
- automated behaviour removing an intended player decision;
- restricted information being sent to clients that should not possess it;
- design rules existing only in conversation and not in code or tests.

PASM should identify these failures more reliably than general code review prompts.

## 2. Nature of PASM

PASM is not merely documentation.

It consists of:

1. **Model files**  
   Human-readable declarations of intended architecture, game design, implementation, migrations, and evidence.

2. **Python runtime**  
   Typed models, parsing, validation, graph analysis, inference, repository scanning, audits, report generation, and LLM context generation.

3. **Observed implementation model**  
   Machine-derived facts about the repository.

4. **Audit procedures**  
   Named workflows combining deterministic checks and targeted semantic review.

The Python code defines the semantics of the model. It is part of PASM itself.

## 3. Guiding Principles

### 3.1 Architecture first

PASM begins as a tool for Project Phoenix architecture.

Other domains may extend it only when they solve a real Phoenix problem or remove proven duplication.

### 3.2 Shared entities

A bridge station, state object, network message, mechanic, or subsystem should have one stable identity.

Different domains attach different semantics to the same entity.

Example:

```yaml
component: engineering-station

core:
  status: accepted

architecture:
  reads:
    - known-damage-state
  sends:
    - diagnose-fault-command

game_design:
  player_role: engineering
  protected_decisions:
    - repair-priority

implementation:
  paths:
    - crates/client/src/engineering
```

### 3.3 Declarative model, executable semantics

Use model files for ordinary declarations.

Use Python for:

- validation;
- graph traversal;
- repository extraction;
- migration analysis;
- state reachability;
- simulations;
- schema migration;
- report generation;
- LLM context selection.

### 3.4 Intent and observation remain separate

PASM must distinguish:

- **Intended model:** what the system should be.
- **Observed model:** what the repository currently contains.
- **Migration allowance:** temporary deviations explicitly permitted during transition.

The scanner must never silently rewrite intent to match the code.

### 3.5 Uncertainty is explicit

A model may be:

- incomplete;
- inferred;
- provisional;
- disputed;
- stale.

PASM should represent this rather than manufacturing certainty.

### 3.6 Local intent overrides generic heuristics

A specific declared design intention takes precedence over generic balance, architecture, or UX advice.

A deliberate exception should not repeatedly appear as a defect.

### 3.7 Deterministic checks before LLM reasoning

Python should handle facts that can be established mechanically.

LLMs should be reserved for:

- semantic interpretation;
- responsibility leakage;
- half-converted logic;
- qualitative design drift;
- stale-specification assessment;
- ambiguous code behaviour.

### 3.8 Findings require evidence

A finding should identify:

- the rule or declaration involved;
- the observed behaviour;
- source locations;
- severity;
- confidence;
- evidence;
- whether a design decision is required.

## 4. The PASM Model Stack

```text
PASM
├── Core Model
├── Architecture Model
├── Game Design Model
├── Implementation Model
└── Evidence Model
```

### 4.1 Core Model

Provides shared semantics:

- stable IDs;
- entity kinds;
- lifecycle status;
- confidence;
- references;
- assumptions;
- open questions;
- exceptions;
- provenance;
- evidence links;
- findings.

### 4.2 Architecture Model

Describes:

- systems and components;
- responsibilities;
- dependency direction;
- state ownership;
- authority;
- runtimes;
- platform constraints;
- interfaces and messages;
- trust boundaries;
- migrations;
- architectural invariants.

### 4.3 Game Design Model

Describes:

- design pillars;
- player roles;
- player verbs;
- protected decisions;
- mechanics;
- resources;
- information visibility;
- coordination requirements;
- failure and recovery;
- tuning intent;
- playtest claims.

### 4.4 Implementation Model

Describes the intended relationship between conceptual entities and implementation artefacts:

- crates;
- packages;
- modules;
- files;
- symbols;
- messages;
- entry points;
- tests;
- configuration;
- deployment targets.

### 4.5 Evidence Model

Records support for claims:

- automated tests;
- source locations;
- manual review;
- runtime observations;
- telemetry;
- playtest reports;
- decision records;
- benchmark results.

### 4.6 Observed Implementation Model

Generated from the repository.

It may contain:

- workspace members;
- dependencies;
- imports;
- modules;
- public symbols;
- messages;
- event producers and consumers;
- worker entry points;
- tests;
- configuration;
- feature flags.

The observed model is not part of intent. It is compared against intent.

## 5. Core Entity Semantics

Every entity has a stable semantic ID.

Required:

```text
id
kind
status
```

Common optional fields:

```text
title
summary
goals
rationale
confidence
tags
references
assumptions
open_questions
exceptions
evidence
supersedes
conflicts_with
source_location
```

### 5.1 IDs

IDs should be:

- lowercase;
- human-readable;
- stable across file moves;
- globally unique within the loaded model;
- independent of implementation paths.

Examples:

```text
engineering-station
authoritative-damage-state
diagnose-fault-command
network-message-v2-migration
```

### 5.2 Lifecycle Status

Initial values:

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

Status describes the relationship between intent and implementation, not general confidence.

### 5.3 Confidence

Initial values:

```text
confirmed
inferred
provisional
disputed
unknown
```

### 5.4 Exceptions

An exception records a deliberate deviation from a rule.

It should contain:

```text
rule
scope
rationale
temporary
removal_condition
approval_status
```

Temporary exceptions without removal conditions should produce findings.

### 5.5 Open Questions

Open questions record unresolved decisions.

They should not be silently filled by an LLM.

A finding that depends on one should use:

```text
requires_decision: true
```

## 6. Architecture Concepts

### 6.1 Components

Initial component-like kinds include:

```text
system
subsystem
component
service
process
runtime
thread
worker
module
adapter
external-system
deployment-target
```

### 6.2 Responsibilities

Common relationships:

```text
owns
reads
writes
produces
consumes
exposes
receives
transforms
coordinates
validates
persists
renders
```

Ownership is stronger than access.

### 6.3 State Classification

State may be:

```text
authoritative
replicated
derived
cached
ephemeral
persistent
local-view
player-knowledge
```

Authoritative state should normally have one owner.

### 6.4 Dependencies

Relationships include:

```text
depends_on
may_depend_on
must_not_depend_on
runtime_depends_on
build_depends_on
optional_dependency
temporary_dependency
```

Dependencies may be checked directly and transitively.

### 6.5 Runtime and Platform

PASM should model:

- authoritative host;
- browser client;
- native client;
- WASM runtime;
- workers;
- threads;
- channels;
- network connections;
- startup and shutdown;
- trust boundaries.

Platform categories:

```text
shared
native-only
browser-only
host-only
client-only
test-only
development-only
```

### 6.6 Interfaces and Messages

Messages should describe:

- producer;
- consumer;
- payload;
- authority;
- validator;
- version;
- replacement;
- trust crossing.

### 6.7 Architecture Invariants

Examples:

- simulation code must not depend on presentation;
- clients must not mutate authoritative world state;
- browser-only APIs must not enter shared simulation;
- every authoritative state has one owner;
- untrusted commands require validation;
- rendering must not resolve game rules;
- compatibility adapters may only be called by approved legacy paths.

## 7. Game Design Concepts

### 7.1 Roles and Verbs

A player role may define:

```text
responsibilities
player_verbs
exclusive_verbs
protected_decisions
visible_information
hidden_information
coordination_with
expected_decision_frequency
```

### 7.2 Protected Decisions

A protected decision must not be:

- fully automated;
- silently resolved by another role;
- bypassed through an unintended UI route;
- removed without an explicit design amendment.

### 7.3 Mechanics

Mechanics may define:

```text
inputs
reads
changes
eligibility
costs
resolution
outputs
failure
side_effects
information_revealed
participating_roles
```

The mechanic model should be simpler than production code.

### 7.4 Information Visibility

Information may be:

```text
public
role-visible
team-visible
hidden
partially-known
derived
delayed
uncertain
```

Restricted information should identify:

- permitted viewers;
- reveal conditions;
- indirect signals;
- architectural enforcement.

### 7.5 Coordination

A coordination requirement identifies:

- participating roles;
- information exchanged;
- actions required;
- intended player effect;
- implementation path.

### 7.6 Resources

A resource may define:

- sources;
- sinks;
- capacity;
- ownership;
- visibility;
- transfer;
- pressure intent.

### 7.7 Failure and Recovery

Failure states should define:

- cause;
- consequence;
- affected roles;
- visibility;
- recovery;
- terminal status.

### 7.8 Tuning

Tuning parameters are not fixed invariants.

They should record:

- affected mechanics;
- intended directional effect;
- bounds;
- current maturity;
- supporting evidence.

## 8. Migration Concepts

Migration is a first-class concern.

A migration should define:

```text
source
target
reason
status
legacy_implementation
target_implementation
temporary_adapters
permitted_legacy_callers
completed_steps
remaining_steps
removal_conditions
verification
```

PASM should detect:

- undeclared legacy callers;
- legacy paths referenced from new code;
- old and new systems both mutating authoritative state;
- adapters without removal conditions;
- completed migrations retaining old code;
- mixed old and new data models in one function;
- conversion in both directions;
- fallback branches preserving obsolete behaviour.

## 9. Findings

Each finding should contain:

```text
id
category
severity
confidence
summary
details
rule
spec_entities
implementation_locations
evidence
suggested_resolution
requires_decision
status
```

Severity:

```text
error
warning
concern
information
```

Categories:

```text
violation
probable-violation
incomplete-migration
stale-specification
unmapped-implementation
unimplemented-specification
intentional-exception
conflicting-intent
unverified
design-risk
architecture-risk
```

Severity is not confidence.

## 10. Scope Boundaries

PASM 1.0 should not attempt to provide:

- public third-party plugins;
- a universal ontology;
- a full formal programming language;
- exhaustive language parsing;
- a graphical editor;
- automatic code changes;
- full gameplay reimplementation;
- automatic proof of design quality;
- automatic proof of architectural correctness.

A feature belongs in PASM only when it supports a current Phoenix audit, workflow, or reasoning need.

## 11. Success Criterion

PASM is successful when it can identify a meaningful defect or design deviation that compilation, tests, and ordinary code review would otherwise miss.

If it merely restates the repository, it is documentation.

If it detects architectural drift, incomplete migrations, design bypasses, or missing evidence, it is part of the engineering system.
