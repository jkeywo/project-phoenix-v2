---
title: Individual System AI State Trees
type: concept
tags: [ai, systems, workshop, determinism]
sources: [crates/phoenix-sim-gameplay/src/ai/state_tree/mod.rs, crates/phoenix-sim-gameplay/src/ai/state_tree/schema.rs, crates/phoenix-sim-gameplay/src/ai/state_tree/compile.rs, crates/phoenix-sim-gameplay/src/ai/state_tree/evaluate.rs, crates/phoenix-sim-gameplay/src/ai/host.rs, crates/phoenix-sim-gameplay/src/entities/config.rs, crates/phoenix-simulation/src/ai/state_tree_runtime.rs, crates/phoenix-simulation/src/dock/server.rs, crates/phoenix-simulation/src/entities/spawner.rs, crates/phoenix-simulation/src/snapshot.rs, crates/phoenix-simulation/src/sim_digest.rs, assets/entities/fragments/ai/dock_state_tree.toml, gui/workshop-state-trees.js, gui/workshop.css, editor/workshop-state-trees.js, pasm/spec/design/ai-state-trees.yaml, pasm/spec/architecture/ai-state-trees.yaml]
updated: 2026-10-14
---

# Individual System AI State Trees

Dock is the first System running the shared state-tree evaluator. Other
capabilities retain their existing policy/selector controllers. Each System
has one controller and its own execution state; reusing a definition shares
neither private memory nor another Console's observations.

Gameplay owns the closed schema, inheritance compiler and pure evaluator under
`ai/state_tree/`. `ai::host::decide_tree` shares the existing Control Source gate.
The Simulation Dock adapter captures its permitted Transfer order and supported
own-equipment facts, evaluates final enabled intent and sends ordinary Dock/Undock
commands through admission. Successful evaluation queues its private runtime at
the controller's existing deferred flush. Human/offline operation leaves tree
memory suspended.

## Definitions and order

`system.config.ai` contains a root and named tree library. Entity includes merge
named tree/state/task/transition arrays by stable identity. Within the resolved
library, `extends` merges parameters, memory defaults and named structure, keeping
replacement positions and appending new nodes. Reference tasks call another tree
with supported boolean bindings and private call frames.

Each frame checks transitions parent first, takes the first eligible transition,
then executes active tasks parent before child in authored order. Calls run at
their task position. Later nodes see earlier private memory and pending writes;
last writes win before commands leave the evaluator. Parent transitions interrupt
affected calls. External facts remain fixed during evaluation.

The compiler rejects unknown facts/channels, invalid identities/hierarchy,
inheritance/call cycles and oversized expansion. The evaluator refuses missing
inputs, non-advancing ticks or invalid restored state without changing continuation.
No authored expressions or custom code nodes are supported.

## Continuation

`ShipAiTreePrograms` holds immutable compiled content rebuilt at spawn.
`ShipAiTreeRuntime` holds a deterministic map keyed by SystemId: active states,
private memory, nested calls/completion and last evaluation tick. Snapshot capture,
restore and the `system-ai-trees` digest stage include that state. Snapshot format
39 and simulation rules 0.11 establish the compatibility boundary.

The shared `dock_state_tree.toml` library provides a derived standard controller
and reusable operation call. Adoption and withdrawal ownership are authored memory,
replacing the old transient DockAiEngaged marker.

## Workshop

`gui/workshop-state-trees.js` presents typed tree/state/task/transition forms,
reference bindings and an inherited resolved-order view. `editor/workshop-state-trees.js`
materialises or replaces only the selected System's AI definition through the
existing accepted-source history. A derived tree can be explicitly flattened for
structural changes. Renaming a mating enabled channel also rebinds its AI writes
and pending reads in the same source operation. Inherited fragments remain intact.
The editor uses the shared Workshop grid/wrapping conventions inside narrow docks.

## Related

- [System](../entities/system.md)
- [Console](../entities/console.md)
- [Shared Behaviour Modules](./shared-behaviour.md)
