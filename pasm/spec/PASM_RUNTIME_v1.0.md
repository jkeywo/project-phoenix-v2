# PASM Runtime

**Version:** 1.0

## 1. Purpose

The PASM Runtime is the Python implementation that gives PASM executable semantics.

It is responsible for:

- loading model files;
- parsing and schema validation;
- building typed semantic models;
- resolving references;
- running deterministic validators;
- scanning the repository;
- building the Observed Implementation Model;
- performing graph analysis and inference;
- executing audit procedures;
- generating findings and reports;
- selecting focused LLM context;
- validating structured LLM findings;
- migrating PASM schemas.

## 2. Runtime Architecture

Recommended package structure:

```text
pasm/
    core/
        model.py
        parser.py
        references.py
        source.py
        validation.py
        findings.py
        evidence.py
        queries.py
        reporting.py
        schema_migrations.py

    architecture/
        model.py
        validation.py
        dependencies.py
        ownership.py
        runtime.py
        messages.py
        migrations.py
        audits.py

    domains/
        game_design/
            model.py
            validation.py
            mechanics.py
            information.py
            scenarios.py
            simulation.py
            playtest.py
            audits.py

    implementation/
        model.py
        mappings.py
        observation.py
        conformance.py

    scanners/
        cargo.py
        rust.py
        javascript.py
        html.py
        config.py

    integration/
        traceability.py
        cross_domain_validation.py
        llm_context.py
        llm_findings.py

    cli/
        main.py
```

## 3. Initial Technology Choices

Recommended:

- Python 3.12 or later;
- dataclasses or Pydantic;
- `ruamel.yaml` if source locations are required early;
- `networkx` for graph analysis;
- `pytest`;
- `typer` or `click`;
- `rich` for terminal output.

Possible later additions:

- tree-sitter;
- Hypothesis;
- Jinja2;
- JSON Schema.

Avoid unnecessary dependencies.

## 4. Core Types

### 4.1 Entity ID

```python
@dataclass(frozen=True, order=True)
class EntityId:
    value: str
```

Validation:

- lowercase;
- permitted characters;
- non-empty;
- globally unique after loading.

### 4.2 Source Location

```python
@dataclass(frozen=True)
class SourceLocation:
    path: Path
    line: int | None
    column: int | None
    section: tuple[str, ...]
```

### 4.3 Lifecycle and Confidence

Use enums.

### 4.4 Base Entity

```python
@dataclass(frozen=True)
class SpecEntity:
    id: EntityId
    kind: str
    status: Status
    confidence: Confidence
    title: str | None
    summary: str | None
    references: tuple[Reference, ...]
    source_location: SourceLocation
```

Domain data may attach through typed sections.

### 4.5 Findings

```python
@dataclass(frozen=True)
class Finding:
    id: str
    category: FindingCategory
    severity: Severity
    confidence: FindingConfidence
    summary: str
    details: str
    rule: str
    spec_entities: tuple[EntityId, ...]
    implementation_locations: tuple[SourceLocation, ...]
    evidence: tuple[EvidenceRef, ...]
    suggested_resolution: str | None
    requires_decision: bool
```

## 5. Parser

### 5.1 Initial Format

Use restricted YAML.

Reject:

- unknown fields;
- arbitrary object tags;
- executable constructors;
- unsupported implicit conversions;
- semantically significant anchors.

### 5.2 Loading Stages

1. Discover files.
2. Parse YAML.
3. Capture source locations.
4. Identify entity declarations.
5. Parse core fields.
6. Parse enabled domain sections.
7. Build unresolved references.
8. Resolve references after all files are loaded.
9. Run schema validation.
10. Build the semantic model.

### 5.3 Error Handling

Collect multiple parse and validation errors where possible.

Every error should include:

- source file;
- line;
- field path;
- rule;
- message.

Do not silently ignore unknown fields.

## 6. Validation Pipeline

Recommended stages:

```text
parse
schema
reference
structural
semantic
cross-domain
repository-conformance
migration
scenario
heuristic
```

Each validator returns findings.

Validators should not print directly.

### 6.1 Validator Interface

```python
class Validator(Protocol):
    id: str

    def validate(self, model: PasmModel, context: ValidationContext) -> list[Finding]:
        ...
```

### 6.2 Domain Registration

Version 1.0 only requires a built-in registry.

```python
DOMAIN_VALIDATORS = {
    "architecture": [...],
    "game_design": [...],
    "implementation": [...],
    "evidence": [...],
}
```

Do not build dynamic plugins.

## 7. Graphs and Inference

Graphs may represent:

- dependencies;
- ownership;
- message flow;
- state derivation;
- runtime deployment;
- implementation mappings;
- design-to-architecture traceability.

Inference may derive:

- transitive dependencies;
- effective ownership;
- information paths;
- impacted entities;
- migration residue;
- unsupported design pillars;
- possible decision bypasses.

Derived facts must retain provenance.

## 8. Implementation Model

The intended Implementation Model is authored in PASM.

It includes:

- paths;
- symbols;
- messages;
- entry points;
- tests;
- configuration;
- status.

Mappings may be many-to-many.

Mapping status:

```text
declared
observed
confirmed
suspected
stale
removed
```

## 9. Observed Implementation Model

The repository scanner builds separate typed objects:

```text
ObservedFile
ObservedCrate
ObservedModule
ObservedDependency
ObservedSymbol
ObservedMessage
ObservedEntryPoint
ObservedTest
ObservedConfiguration
```

Each observation should include:

- repository revision;
- extractor;
- source location;
- confidence.

## 10. Repository Scanners

### 10.1 Cargo Scanner

Extract:

- workspace members;
- package names;
- dependencies;
- optional dependencies;
- features;
- target-specific dependencies.

### 10.2 Rust Scanner

Initial extraction:

- modules;
- `use` relationships;
- public types;
- public traits;
- public functions;
- tests;
- feature gates.

Prefer syntax-tree parsing when regex becomes unreliable.

### 10.3 JavaScript or TypeScript Scanner

Extract:

- imports;
- exports;
- entry points;
- WASM binding references.

### 10.4 HTML Scanner

Extract:

- script references;
- module entry points;
- relevant application roots.

Do not construct a complete DOM model.

### 10.5 Scanner Output

Generate reproducible JSON containing:

- scanner versions;
- repository revision;
- extraction timestamp;
- observations.

## 11. Conformance Checks

Initial deterministic checks:

- mapped path does not exist;
- forbidden observed dependency;
- duplicate state ownership;
- client ownership of authoritative state;
- unvalidated trust-boundary message;
- deprecated entity with active implementation;
- missing message producer or consumer;
- implementation entity with no PASM mapping;
- PASM entity with no implementation;
- temporary adapter without removal condition;
- undeclared legacy caller;
- old and new systems both writing authoritative state.

## 12. Migration Runtime

Migration conditions should initially use a fixed set of predicates:

```text
path-does-not-exist
symbol-does-not-exist
no-observed-imports
all-callers-are
test-passes
```

Do not create a general expression language initially.

Heuristic migration checks may look for:

- legacy and target types in one function;
- bidirectional conversion;
- obsolete parameters;
- fallback branches;
- temporary comments;
- wrappers preserving old behaviour.

Heuristic findings should have lower confidence.

## 13. Game Design Runtime

### 13.1 Validators

Check:

- player verb has an owner;
- protected decision has no declared bypass;
- hidden information has a reveal condition;
- resource has source and sink where appropriate;
- failure has consequence;
- non-terminal failure has recovery;
- design pillar has supporting mechanic;
- tuning is not confused with invariant.

### 13.2 Scenarios

Provide a lightweight state-transition model.

Do not duplicate production gameplay.

Use it for:

- reachability;
- recovery;
- information reveal;
- action eligibility;
- role access;
- deadlocks;
- limited resource-flow checks.

## 14. Cross-Domain Validation

Initial checks:

- design state has no architecture owner;
- player-visible state has no presentation path;
- hidden state is replicated too broadly;
- role action has no implementation;
- implementation command has no permitted role;
- protected decision is automated;
- coordination requirement has no communication path;
- one component can bypass intended coordination;
- failure or recovery path is missing;
- tuning value is duplicated across platforms.

## 15. Audit Pipeline

```text
load PASM
validate PASM
load/generate observed model
run deterministic checks
select relevant entities
select relevant source
run optional LLM audit
validate LLM findings
deduplicate
classify
report
```

Named audit procedures should be Python objects or functions.

## 16. LLM Context Generation

Inputs:

```text
task
target entities
target paths
domains
graph depth
include migrations
include evidence
include findings
```

Relevance scoring may consider:

- direct task match;
- references;
- dependency distance;
- shared state;
- shared messages;
- active migrations;
- linked design intent;
- linked findings.

Output should include omissions and repository revision.

## 17. LLM Findings

Require structured output.

Validate:

- referenced entities exist;
- referenced paths are in the supplied source bundle;
- category and severity are valid;
- evidence exists;
- confidence is explicit;
- unresolved intent is marked as requiring a decision.

The runtime should reject malformed findings.

## 18. Queries

Initial CLI queries:

```text
pasm query entity <id>
pasm query owns <state-id>
pasm query dependencies <component-id>
pasm query dependents <component-id>
pasm query implementation <entity-id>
pasm query entities-for-path <path>
pasm query migration <id>
pasm query unmapped-spec
```

## 19. CLI

Initial commands:

```text
pasm validate
pasm query entity <id>
```

Later:

```text
pasm scan
pasm audit <audit-name>
pasm context --task "..."
pasm report
pasm migrate-spec
```

CLI requirements:

- human-readable output;
- JSON output;
- non-zero exit codes for deterministic failures;
- configurable repository and spec roots.

## 20. Generated Outputs

```text
generated/
    repository_inventory/
    reports/
    context/
    diagrams/
```

Initial outputs:

- validation JSON;
- terminal report;
- repository inventory;
- architecture conformance report;
- migration report;
- traceability report;
- LLM context bundle.

Diagrams should come later.

## 21. Tests

Each validator must have:

- passing fixture;
- failing fixture;
- source-location assertion where relevant.

Required fixtures:

- duplicate source of truth;
- forbidden dependency;
- invalid client authority;
- hidden information leak;
- incomplete migration;
- undeclared legacy caller;
- missing implementation;
- stale mapping;
- protected decision automation.

## 22. Schema Migration

PASM source schema changes should be versioned.

The runtime should support:

```text
pasm migrate-spec
```

Schema migration should:

- preserve comments where practical;
- report lossy transformations;
- create backups or write to a new directory;
- never silently discard unknown data.

## 23. Performance and Scale

Initial optimisation priorities:

- deterministic load order;
- cached repository observations;
- selective scans;
- selective context generation;
- incremental validation where practical.

Do not optimise before real repository use demonstrates a need.

## 24. Security

Treat PASM files and repository content as untrusted input.

Do not:

- execute YAML tags;
- execute arbitrary Python from model files;
- run shell commands from removal conditions;
- permit unrestricted path traversal;
- automatically send repository content to external LLMs without explicit configuration.

## 25. Runtime Scope Control

Do not implement:

- dynamic plugins;
- generic rule expression language;
- full compiler integration for every language;
- automatic source rewriting;
- full graph UI;
- gameplay-equivalent simulation;
- arbitrary executable PASM declarations.

The runtime should remain small enough to understand and test.
