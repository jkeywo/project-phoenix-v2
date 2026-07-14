# PASM v1.0 Documentation Set

**PASM** stands for **Phoenix Architecture & System Model**.

PASM is an executable, architecture-first specification system for Project Phoenix. It combines human-readable model files with Python semantics, repository observation, deterministic validation, targeted LLM audits, and task-specific context generation.

## Documentation Map

1. [PASM Core Concepts](PASM_CORE_CONCEPTS_v1.0.md)  
   Defines the model, its principles, terminology, domains, lifecycle, and scope.

2. [Writing PASM](WRITING_PASM_v1.0.md)  
   Explains how to author PASM model files, structure entities, record intent, declare architecture, and encode game-design semantics.

3. [Working with PASM](WORKING_WITH_PASM_v1.0.md)  
   Defines development workflows: ingestion, audits, migrations, refactors, design changes, evidence handling, and LLM context generation.

4. [PASM Runtime](PASM_RUNTIME_v1.0.md)  
   Specifies the Python implementation: parser, semantic model, validators, repository scanners, inference, audit pipeline, CLI, and generated outputs.

5. [PASM Implementation Roadmap](PASM_IMPLEMENTATION_ROADMAP_v1.0.md)  
   Gives the phased implementation plan and milestone exit criteria.

6. [PASM Project Handover](PASM_PROJECT_HANDOVER_v1.0.md)  
   Provides a concise implementation handoff for a new ChatGPT Work, Codex, or coding-agent session.

## Core Model Structure

```text
PASM
├── Core Model
├── Architecture Model
├── Game Design Model
├── Implementation Model
└── Evidence Model
```

The repository scanner produces a separate **Observed Implementation Model**. Audits compare this observed model with the intended models.

## Current Scope

PASM is initially:

- specific to Project Phoenix;
- architecture-first;
- implemented with human-readable model files and Python semantics;
- extended with one built-in `game_design` domain;
- designed to detect architectural drift, incomplete migrations, design-implementation divergence, and missing evidence.

PASM is not yet:

- a public plugin platform;
- a universal modelling language;
- a full second implementation of Phoenix;
- a replacement for tests, static analysis, design documents, ADRs, or playtesting.

## Recommended Starting Point

Implementation should begin with:

1. core typed entities;
2. restricted YAML loading;
3. references and source locations;
4. structured findings;
5. `pasm validate`;
6. one vertical slice: **engineering damage diagnosis**.

Do not begin with repository scanning, graph visualisation, simulations, or LLM integration.
