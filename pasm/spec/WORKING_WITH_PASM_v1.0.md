# Working with PASM

**Version:** 1.0

## 1. Purpose

This guide defines how PASM should be used during Project Phoenix development.

It covers:

- creating and updating the model;
- ingesting an existing subsystem;
- running audits;
- handling migrations;
- planning and reviewing refactors;
- recording design changes;
- collecting evidence;
- generating LLM context;
- resolving discrepancies.

PASM should be part of the development workflow, not a document updated only after work is complete.

## 2. Standard PASM Workflow

```text
Intent
  ↓
PASM declaration
  ↓
Implementation work
  ↓
Repository scan
  ↓
Deterministic validation
  ↓
Targeted LLM audit
  ↓
Findings and decisions
  ↓
Fixes, evidence, or PASM amendment
```

## 3. Ingesting an Existing Subsystem

Use this procedure when PASM does not yet describe a subsystem.

### Step 1: Define the audit purpose

Examples:

- detect legacy networking paths;
- preserve host authority;
- check design information visibility;
- prepare a worker refactor;
- identify duplicate state.

Do not ingest a subsystem merely for completeness.

### Step 2: Identify conceptual entities

List:

- responsibilities;
- authoritative state;
- derived state;
- runtimes;
- messages;
- player roles;
- mechanics;
- migrations;
- relevant evidence.

### Step 3: Map implementation

Identify:

- crates;
- modules;
- symbols;
- messages;
- entry points;
- tests;
- browser/native variants;
- legacy paths.

### Step 4: Distinguish current and target state

Record:

- what exists;
- what should exist;
- what is temporary;
- what remains unresolved.

### Step 5: Ask focused questions

Examples:

- Which module is the source of truth?
- Is this compatibility layer still required?
- Is this state replicated or re-derived?
- Which role should make this decision?
- Should this information be visible before diagnosis?
- Is browser/native divergence intentional?

### Step 6: Encode provisional PASM

Use `provisional` or `inferred` where necessary.

### Step 7: Validate and audit

Run deterministic validation before asking an LLM to inspect code.

### Step 8: Confirm with evidence

Use tests, code review, runtime observation, or design decisions.

## 4. Running Validation

Basic command:

```text
pasm validate
```

Validation should include:

- schema;
- references;
- lifecycle;
- ownership;
- dependency rules;
- message completeness;
- implementation path existence;
- migration consistency;
- cross-domain consistency.

Validation findings should be classified by severity and confidence.

Only deterministic errors should initially fail CI.

## 5. Repository Scanning

Command:

```text
pasm scan
```

The scanner should produce an Observed Implementation Model.

The scan should include a repository revision.

A scan should not modify PASM source files.

After scanning:

```text
pasm audit architecture-conformance
```

## 6. Audit Procedures

Initial named audits:

```text
architecture-conformance
dependency-boundaries
state-ownership
sources-of-truth
runtime-authority
platform-divergence
incomplete-migrations
deprecated-code
implementation-coverage
specification-staleness
design-implementation-alignment
player-agency
protected-decisions
information-visibility
role-responsibility
coordination-requirements
resource-flow
failure-and-recovery
mechanic-reachability
design-pillar-support
```

### 6.1 Standard Audit Sequence

1. Load PASM.
2. Validate PASM.
3. Load or generate observed model.
4. Run deterministic checks.
5. Select relevant entities and source files.
6. Run semantic review only where required.
7. Consolidate findings.
8. Distinguish code defects from stale PASM.
9. Mark unresolved decisions.
10. Produce a report.

### 6.2 Audit Outcomes

A discrepancy may be:

- implementation violation;
- probable implementation violation;
- stale specification;
- incomplete migration;
- intentional exception;
- missing implementation;
- missing specification;
- unresolved design decision;
- unverified claim.

Do not treat all discrepancies as code defects.

## 7. Refactor Workflow

### 7.1 Before Refactoring

1. Generate task context:

```text
pasm context --task "Refactor engineering diagnosis into shared worker-safe code"
```

2. Review:
   - affected entities;
   - invariants;
   - dependency boundaries;
   - active migrations;
   - protected decisions;
   - platform constraints;
   - linked tests.

3. Record any intended architecture change.

4. Create or update a migration if the old and new paths will coexist.

### 7.2 During Refactoring

- preserve stable conceptual IDs;
- update implementation mappings as files move;
- do not mark migration complete early;
- record temporary adapters;
- keep target code from depending on legacy code;
- add evidence as tests are created.

### 7.3 After Refactoring

Run:

```text
pasm scan
pasm validate
pasm audit architecture-conformance
pasm audit incomplete-migrations
pasm audit design-implementation-alignment
```

Then:

- resolve findings;
- remove completed exceptions;
- update PASM only where intent changed;
- attach evidence;
- close migration when removal conditions are satisfied.

## 8. Migration Workflow

### 8.1 Start a Migration

Record:

- source system;
- target system;
- reason;
- target architecture;
- temporary adapters;
- approved legacy callers;
- steps;
- removal conditions;
- verification plan.

### 8.2 Audit During Migration

Check:

- undeclared callers;
- old and new authority overlap;
- backward conversions;
- target code importing legacy code;
- obsolete parameters;
- fallback branches;
- adapter spread;
- platform divergence.

### 8.3 Complete a Migration

A migration is complete only when:

- target implementation exists;
- old callers are removed or explicitly retained;
- removal conditions pass;
- legacy authority is gone;
- tests cover target behaviour;
- PASM mappings are updated;
- temporary exceptions are removed.

### 8.4 Rejected or Reversed Migration

Record the decision explicitly.

Do not leave both target and source marked active without explanation.

## 9. Architecture Change Workflow

An architecture change should record:

```text
current state
target state
reason
affected entities
added invariants
removed invariants
migration
implementation consequences
tests
evidence
```

The change may require:

- PASM amendment;
- implementation plan;
- migration entity;
- new validators;
- updated context bundles.

## 10. Game Design Change Workflow

A design change should record:

```text
existing intent
new intent
affected roles
affected mechanics
information changes
protected decision changes
architecture consequences
implementation consequences
playtest evidence
status
```

Do not update game-design declarations merely to match an accidental implementation.

### 10.1 Evidence Priority

For playtest work, record separately:

1. player sentiment;
2. player diagnosis;
3. suggested solution.

Player sentiment is strongest evidence of experience.

Player diagnosis may be wrong but useful.

Suggested fixes should not be treated as requirements without design review.

## 11. Handling Findings

### 11.1 Confirmed Violation

Fix implementation or amend intent through an explicit decision.

### 11.2 Probable Violation

Inspect relevant code and gather evidence.

### 11.3 Stale Specification

Update PASM only after confirming the implementation reflects accepted intent.

### 11.4 Intentional Exception

Record it with scope, rationale, and removal condition if temporary.

### 11.5 Requires Decision

Ask focused questions.

Do not invent answers.

### 11.6 Missing Evidence

Add tests, review notes, playtest reports, or runtime observations.

## 12. LLM Context Generation

PASM context should be generated for a task, not dumped wholesale.

Example:

```text
pasm context \
  --task "Move tactical target selection out of presentation code" \
  --include architecture,game-design,migrations,implementation,evidence
```

A bundle should include:

- relevant entities;
- goals and rationale;
- invariants;
- ownership;
- authority;
- design intent;
- protected decisions;
- active migrations;
- implementation paths;
- relevant tests;
- known findings;
- open questions.

It should also state:

- omitted domains;
- dependency depth;
- observation revision;
- stale or missing mappings.

## 13. LLM Audit Workflow

The LLM should receive:

- audit purpose;
- relevant PASM slice;
- deterministic findings;
- selected source files;
- exceptions;
- open questions.

The LLM should return structured findings with:

```text
summary
category
severity
confidence
PASM references
code references
evidence
suggested resolution
requires_decision
```

The LLM should not:

- rewrite PASM automatically;
- invent missing design decisions;
- treat generic advice as superior to declared local intent;
- repeat deterministic checks without adding semantic value.

## 14. CI Workflow

Initial CI:

```text
pasm validate
pasm scan
pasm audit architecture-conformance
```

Recommended policy:

- deterministic errors fail;
- warnings and concerns report;
- heuristic and LLM findings do not initially fail CI;
- migration removal conditions may become blocking once stable.

## 15. Audit History

Audit outputs should record:

- PASM version;
- PASM revision;
- repository revision;
- scanner version;
- audit version;
- timestamp;
- enabled domains;
- findings.

Old audit reports are historical evidence, not current truth.

## 16. Adding a New Domain

Do not add a domain because it might be useful.

Add one only when:

- at least two real use cases need distinct semantics;
- existing architecture or game-design fields are insufficient;
- dedicated validators or audits will use it;
- the domain reduces actual duplication.

Potential future domains might include:

- security;
- performance;
- deployment;
- accessibility;
- AI behaviour.

None should be implemented speculatively.

## 17. Review Checklist

Before merging a PASM change:

- Is intent clear?
- Is status accurate?
- Are references valid?
- Is ownership explicit?
- Are temporary deviations represented?
- Does the model duplicate implementation?
- Are design claims linked to mechanics or evidence?
- Are local exceptions recorded?
- Are open questions preserved?
- Does the change improve an audit, workflow, or context bundle?

## 18. Initial Vertical Slice

Use engineering damage diagnosis.

Required model coverage:

- host damage authority;
- actual damage state;
- known damage state;
- diagnosis command;
- engineering UI;
- client/host messages;
- reveal conditions;
- engineering protected decisions;
- at least one migration;
- implementation mappings;
- tests or missing evidence.

Required audits:

- source of truth;
- client authority;
- information visibility;
- protected decision bypass;
- incomplete migration;
- implementation coverage.
