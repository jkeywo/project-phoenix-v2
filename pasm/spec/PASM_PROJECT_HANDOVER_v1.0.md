# PASM Project Handover

**Project:** Phoenix Architecture & System Model  
**Version:** 1.0

## 1. Objective

Implement PASM as an executable, architecture-first system model for Project Phoenix.

PASM should help deterministic Python tooling and LLMs:

- preserve intended architecture;
- detect architectural drift;
- detect old code and incomplete migrations;
- identify duplicate sources of truth;
- compare intended and observed repository structure;
- preserve game-design intent;
- detect code that bypasses protected player decisions;
- detect information visibility violations;
- generate focused task context.

## 2. Current Design Decisions

PASM is:

1. specific to Project Phoenix initially;
2. architecture-first;
3. based on human-readable model files and Python semantics;
4. composed of Core, Architecture, Game Design, Implementation, and Evidence models;
5. supported by a separate Observed Implementation Model;
6. extended with one built-in `game_design` domain;
7. deliberately limited to abstractions with immediate audit value.

PASM is not:

- a public plugin platform;
- a general-purpose modelling ecosystem;
- a second complete implementation of Phoenix;
- a replacement for tests, ADRs, static analysis, or playtests.

## 3. Core Principle

> Shared entities, architecture-first modelling, domain-specific semantics, and cross-domain validation without building a general-purpose modelling platform prematurely.

## 4. First Vertical Slice

Use **engineering damage diagnosis** provisionally.

Required questions:

- Who owns actual damage state?
- Who owns known damage state?
- Can a client mutate authoritative damage?
- Are old and new damage systems both active?
- Does UI code resolve damage rules?
- Can engineering see faults before diagnosis?
- Can another role bypass repair prioritisation?
- Does network replication expose hidden fault data?
- Does a migration leave dual authority or undeclared callers?

## 5. Immediate Implementation Scope

Implement Phase 0, Phase 1, and Phase 2 only.

Deliver:

- Python package;
- restricted YAML loading;
- stable IDs;
- lifecycle and confidence;
- source locations;
- references;
- exceptions;
- evidence references;
- structured findings;
- deterministic validation;
- `pasm validate`;
- unit tests;
- JSON output.

Do not implement yet:

- repository scanning;
- graph visualisation;
- scenario simulation;
- LLM integration;
- public extensions;
- custom grammar.

## 6. Suggested Initial Layout

```text
tools/pasm/
    pyproject.toml
    README.md

    src/pasm/
        __init__.py

        core/
            __init__.py
            model.py
            source.py
            references.py
            findings.py
            evidence.py
            parser.py
            validation.py

        cli/
            __init__.py
            main.py

    tests/
        unit/
            test_model.py
            test_parser.py
            test_references.py
            test_validation.py

        fixtures/
            valid_minimal/
            duplicate_entity/
            broken_reference/
            invalid_status/
            unknown_field/
            temporary_exception_without_removal/
```

## 7. Initial Dependencies

Recommended:

```text
Python 3.12+
ruamel.yaml
pytest
typer
rich
```

Use dataclasses or Pydantic.

Prefer the smallest adequate dependency set.

## 8. First Model Types

Implement:

```text
EntityId
SourceLocation
Status
Confidence
Reference
ExceptionRule
EvidenceReference
SpecEntity
Finding
Severity
FindingCategory
```

## 9. First CLI

```text
pasm validate --spec <path> --format terminal
pasm validate --spec <path> --format json
pasm query entity <id>
```

Only `validate` is required for the first milestone.

## 10. First Acceptance Tests

The implementation must:

- load a valid minimal model;
- reject duplicate IDs;
- reject unknown fields;
- report broken references;
- preserve source locations;
- reject invalid lifecycle values;
- report temporary exceptions lacking removal conditions;
- output findings as JSON;
- return a non-zero exit code on deterministic errors.

## 11. Documentation to Use

Read in this order:

1. `PASM_CORE_CONCEPTS_v1.0.md`
2. `WRITING_PASM_v1.0.md`
3. `PASM_RUNTIME_v1.0.md`
4. `PASM_IMPLEMENTATION_ROADMAP_v1.0.md`
5. `WORKING_WITH_PASM_v1.0.md`

## 12. Suggested First Prompt for a Coding Agent

```text
Implement PASM Phase 0 through Phase 2 only.

Use:
- PASM_CORE_CONCEPTS_v1.0.md
- WRITING_PASM_v1.0.md
- PASM_RUNTIME_v1.0.md
- PASM_IMPLEMENTATION_ROADMAP_v1.0.md

Start by creating the package structure, typed core model, restricted YAML parser, reference resolver, structured findings, unit fixtures, and the `pasm validate` CLI.

Do not implement repository scanning, architecture-domain semantics, game-design semantics, simulations, LLM integration, or a custom grammar yet.

Where the documentation leaves a decision open, choose the simplest implementation that preserves source locations, rejects unknown fields, and remains easy to test. Record the decision in README.md.
```

## 13. Scope Guard

Before adding anything, ask:

- Does Phase 0–2 require this?
- Does a test need it?
- Does it preserve source information?
- Does it simplify the model?
- Is it speculative?

Defer speculative work.
