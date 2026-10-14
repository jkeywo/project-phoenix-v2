# Increment 3: reusable AI state trees

Implementation reference: [contract](../gameplay-redesign/implementation/increment-3.md).

## Implemented scope

Dock is the first individual System controller to use a reusable, authored state
tree. The closed node vocabulary supports hierarchy, single-base inheritance,
boolean parameters/private memory, ordered tasks/transitions and reference calls.
Later writes win; no intermediate command enters admission. Each System owns its
continuation, including nested calls. Human/offline control suspends that state.

Dock reads its existing permitted observations and emits ordinary Dock/Undock
commands through existing admission. Its shared authored library preserves
Transfer behaviour and withdrawal reporting. Other System policies remain on
the existing controller interface, with no competing Dock policy.

Workshop supplies supported node forms, resolved inheritance inspection,
materialisation, ordering, typed bindings and atomic Apply with undo/redo.
Unsupported definitions cannot enter Preview or disposable Test. Active Dock
content requires an explicit tree; external content receives conversion guidance.
Snapshot 39 and rules 0.11 reject incompatible old state. Protocol 6 is unchanged.

## Evidence

Runtime implementation: `22e5a23e`, with final scheduling/Workshop corrections
through `e78fb194`. [Validation receipts](../gameplay-redesign/implementation/tree-validation.json)
record 9,079 passing native tests (102 ignored), 8,708 passing JavaScript tests,
Clippy, formatting, layers, generated surfaces, strict strings, LODs and PASM.
The renderer-independent Simulation suite passes 4,667 tests; the pure tree
suite passes 12. Independent evaluator, controller and Workshop reviews passed.

Four Dock/status/transfer browser cases pass against the actual WASM host and
pure-JS consoles. The rendered Workshop journey inspects inherited/reference
nodes, materialises and flattens a tree, changes a parameter, rejects incomplete
components, rebinds command channels, changes Console ownership, exercises
undo/redo and export/reload, and starts/stops disposable Test without changing
the source session. The new form fits within its narrow docked panel; screenshots
were inspected. These tests use the existing browser transport shim.

Integration corrected the controller's deferred publication boundary and the
Workshop scalar writer's handling of nested AI tables. Regression checks pass;
no scheduling checks or runtime validators were weakened. Phone-sized status
inspection remains separate from desktop Dock operation because of the existing
diagnostics overlap.

[Gameplay comparison](../gameplay-redesign/implementation/tree-baseline.md) passes:
both bounded scenario reports match Increment 2 byte for byte, seeded repeats
and replay/continuation agree, and the Cruiser wins 20/20 with all 16 unchanged
thresholds. Median tick times are similar; Combat Test p95 is modestly higher.
Automated evidence is complete. Human/device acceptance remains open, so the
ledger retains `in_progress`.

## Human/device acceptance

Physical phones, native multi-monitor operation and human workload judgement
remain pending, including prior increments' landscape-phone diagnostics overlap.
The user explicitly authorised continuing to Increment 3; these checks remain open.

Operate and delegate Cruiser Dock with an active Transfer objective, reclaim it
and verify continuation when delegated again. In Workshop inspect the inherited
Dock tree, materialise and edit it, undo/redo, export/reload, then enter and stop
disposable Test. Judge whether supported node labels and resolved order are clear.
Increment 4 has not started.
