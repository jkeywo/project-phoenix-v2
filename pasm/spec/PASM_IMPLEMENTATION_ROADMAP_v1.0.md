# PASM Implementation Roadmap

**Version:** 1.0

## Objective

Deliver a usable PASM vertical slice that can:

- load and validate intended architecture;
- map entities to Project Phoenix code;
- produce an observed repository model;
- identify architectural drift;
- model one migration;
- encode relevant game-design intent;
- detect one cross-domain design violation;
- generate focused LLM context.

The provisional vertical slice is **engineering damage diagnosis**.

## Phase 0 — Define the Vertical Slice

### Tasks

- identify engineering damage files, messages, UI, tests, and legacy paths;
- write ten to fifteen audit questions;
- create known-good and known-bad fixtures;
- establish the Python package and test runner.

### Exit Criteria

- subsystem boundaries identified;
- audit questions agreed;
- fixtures exist;
- package and CLI skeleton run.

## Phase 1 — Core Model

### Tasks

- `EntityId`;
- `SourceLocation`;
- lifecycle status;
- confidence;
- references;
- exceptions;
- evidence;
- findings;
- base entity;
- unit tests.

### Exit Criteria

- entities are typed;
- references resolve;
- source locations persist;
- findings are structured;
- tests pass.

## Phase 2 — Restricted YAML and Validation

### Tasks

- choose YAML library;
- define restricted document shape;
- parse files;
- collect errors;
- detect unknown fields;
- load multiple files;
- resolve cross-file references;
- implement `pasm validate`;
- add JSON output.

### Exit Criteria

- valid models load;
- malformed models fail with source locations;
- CLI works;
- deterministic exit codes work.

## Phase 3 — Architecture Model

### Tasks

- components;
- states;
- dependencies;
- ownership;
- runtime;
- authority;
- trust;
- messages;
- platform constraints;
- architecture queries;
- vertical-slice model.

### Exit Criteria

- engineering architecture encoded;
- duplicate ownership detected;
- forbidden dependencies detected;
- authority violations detected.

## Phase 4 — Implementation Model

### Tasks

- implementation artefact types;
- declared mappings;
- path validation;
- coverage checks;
- implementation queries.

### Exit Criteria

- engineering entities map to real code;
- stale paths detected;
- unmapped model entities reported.

## Phase 5 — Observed Implementation Model

### Tasks

- Cargo scanner;
- Rust module/import scanner;
- basic JS/TS scanner;
- basic HTML scanner;
- inventory JSON;
- declared/observed comparison.

### Exit Criteria

- repository revision recorded;
- observed dependencies generated;
- at least one real or fixture conformance violation detected.

## Phase 6 — Migration Semantics

### Tasks

- migration model;
- approved legacy callers;
- temporary adapters;
- removal conditions;
- duplicate authority checks;
- partial-conversion heuristics;
- migration reports.

### Exit Criteria

- one real or representative migration encoded;
- undeclared legacy caller detected;
- removal conditions evaluated.

## Phase 7 — Game Design Model

### Tasks

- roles;
- verbs;
- protected decisions;
- mechanics;
- information visibility;
- coordination;
- resources;
- failure and recovery;
- tuning;
- playtest claims;
- engineering design encoding.

### Exit Criteria

- engineering diagnosis intent encoded;
- hidden information has reveal conditions;
- protected decisions have owners.

## Phase 8 — Cross-Domain Validation

### Tasks

- design-to-architecture links;
- architecture-to-implementation links;
- role/action conformance;
- information enforcement;
- protected decision checks;
- traceability report.

### Exit Criteria

- one design-to-code violation detected;
- traceability report generated.

## Phase 9 — Simplified Scenario Reasoning

### Tasks

- scenario format;
- minimal state transitions;
- reachability;
- failure and recovery checks;
- role access checks;
- information reveal checks.

### Exit Criteria

- diagnosis scenario runs;
- premature reveal detected;
- wrong-role repair detected;
- model remains simpler than production code.

## Phase 10 — LLM-Assisted Audits

### Tasks

- context bundle schema;
- source-slice selection;
- structured findings schema;
- architecture audit;
- migration audit;
- design alignment audit;
- finding deduplication.

### Exit Criteria

- targeted LLM audit uses PASM and selected code;
- output is structured and source-linked;
- deterministic and semantic findings remain distinct.

## Phase 11 — Task Context Generation

### Tasks

- relevance traversal;
- dependency-depth controls;
- migration inclusion;
- evidence inclusion;
- omission reporting;
- `pasm context`.

### Exit Criteria

- a coding agent can use the bundle for a real refactor;
- relevant context is smaller than broad repository ingestion.

## Phase 12 — Workflow and CI Integration

### Tasks

- project configuration;
- CI validation;
- scan caching;
- pre-refactor procedure;
- post-refactor audits;
- audit history.

### Exit Criteria

- deterministic failures can block CI;
- reports are revision-linked;
- PASM participates in normal Phoenix work.

## Milestones

### Milestone 1 — Parsable Model

- YAML;
- IDs;
- references;
- findings;
- CLI.

### Milestone 2 — Architecture Validation

- ownership;
- dependency;
- authority;
- runtime.

### Milestone 3 — Repository Conformance

- mappings;
- scanner;
- observed model.

### Milestone 4 — Migration Audit

- legacy callers;
- adapters;
- removal conditions.

### Milestone 5 — Game Design

- roles;
- protected decisions;
- visibility.

### Milestone 6 — Cross-Domain Audit

- traceability;
- design implementation checks.

### Milestone 7 — LLM Audit

- targeted context;
- structured semantic findings.

### Milestone 8 — Phoenix Integration

- CI;
- refactor workflow;
- audit history.

## Immediate Backlog

### Foundation

- [ ] Create package.
- [ ] Configure `pytest`.
- [ ] Add CLI skeleton.
- [ ] Implement `EntityId`.
- [ ] Implement lifecycle and confidence enums.
- [ ] Implement `SourceLocation`.
- [ ] Implement `SpecEntity`.
- [ ] Implement `Reference`.
- [ ] Implement `Exception`.
- [ ] Implement `Evidence`.
- [ ] Implement `Finding`.

### Parsing

- [ ] Select YAML library.
- [ ] Define restricted schema.
- [ ] Implement file discovery.
- [ ] Implement parsing.
- [ ] Preserve source locations.
- [ ] Resolve references.
- [ ] Reject unknown fields.
- [ ] Add JSON findings output.

### First Fixtures

- [ ] valid minimal model;
- [ ] duplicate entity;
- [ ] broken reference;
- [ ] invalid status;
- [ ] unknown field;
- [ ] malformed YAML;
- [ ] temporary exception without removal condition.

## First Complete Version Exit Criteria

The first complete PASM version requires:

1. intended engineering architecture encoded;
2. relevant game-design intent encoded;
3. deterministic validation;
4. real implementation mappings;
5. observed repository model;
6. known fixture violations detected;
7. one migration evaluated;
8. one protected decision checked;
9. one information-visibility rule checked;
10. traceability report;
11. focused LLM context;
12. structured LLM audit;
13. findings distinguish code defects, stale PASM, exceptions, and decisions;
14. CLI and CI operation;
15. evidence that expansion is justified.

## Scope-Control Questions

Before adding a feature:

- Which current Phoenix audit needs it?
- Does it add distinct semantics?
- Can it be deterministic?
- Does it duplicate production code?
- Does it remove real duplication?
- Is the output consumed?
- Can a simpler declaration solve the problem?
- Is it needed before the vertical slice is complete?

If the answer is unclear, defer it.
