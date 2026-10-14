# Increment 3: reusable AI state trees

The user instructed: “Continue to increment 3”. Physical/device acceptance in
increments 1 and 2 remains pending; this instruction authorises advancing without
rewriting that evidence. Increment 3 does not start later gameplay migrations.

## Implementation contract

Adopt `pasm/spec/design/ai-state-trees.yaml` before runtime changes. Extend the
existing per-System AI host gate with a typed, pure tree evaluator; migrate Dock
alone as the first consumer. Existing controllers migrate with their own future
capabilities. Never run a tree and legacy controller on the same System.

Author `config.ai = { root, tree = [...] }` under the existing System definition.
Reuse libraries through entity include fragments. Named definitions have single
base inheritance, stable state/task/transition IDs, boolean parameters and private
memory. Supported tasks write a bound command, remember a value, call another tree
or complete. Hierarchical transitions and task order are explicit authored data.

On an ordinary AI cadence tick, capture permitted facts, evaluate parent transitions
before child transitions, take at most one transition per frame, then execute parent
work before child work. Calls evaluate inline. Final writes win by authored order;
later nodes see earlier private/pending writes. No intermediate command is admitted.
Parent transitions interrupt calls immediately; existing machinery may take time.
No Workshop scripting or custom node implementations are introduced.

Dock uses its permitted Transfer order and own observed availability/engagement.
Preserve nearest-target matching, engaged-only start test, ownership adoption,
withdrawal hysteresis and OrderWithdrawn reporting before Undock. All human/AI
commands use the existing admission and physical applier. Human/offline control
suspends private tree state; no unique temporary-Backfill policy is added.

Validate inheritance, hierarchy, calls, supported bindings and bounded expansion.
Capture/restore/digest active state, memory and per-call continuation immediately;
advance affected compatibility versions. Definitions are immutable derived data,
not a second authoritative simulation store.

Workshop supplies visual supported-node editing, ordering, inheritance/reference
inspection, typed bindings, atomic accepted-source Apply and ordinary undo/redo,
export/reload, Preview and disposable Test. Shared jobs and Scenario trees remain
later consumers of the mechanism, not extra implementation in this increment.

## Verification

Use pure evaluator and owner tests for ordered writes, internal visibility,
inheritance/references, parent interruption, isolation, continuation and validation.
Prove Dock authority, ordinary AI and unchanged lifecycle behavior. Demonstrate
human Dock operation/delegation and real Workshop tree editing/Test. Run required
integrated gates once on the combined result; record physical/native limitations
separately. Compare bounded Cruiser gameplay and performance plus unchanged balance
thresholds with increment 2. Update PASM, runtime wiki, ledger and PRD #1562.

## Content conversion and compatibility

Active Dock definitions require an explicit `system.config.ai`. The five shipped
active hulls reuse `assets/entities/fragments/ai/dock_state_tree.toml`; passive
berths have no controller and retain their endpoint declarations. The fragment
uses System ID `dock` and channel `enabled`. Custom IDs/channels must bind the
library to their own System and actual mating component's enabled channel.
Workshop performs channel rebinding atomically with component edits.

To opt out intentionally, author a root with one idle state and no tasks. Missing
AI is a load error on the production strict path, with conversion/original-version
guidance. Trees on other System kinds remain unsupported until those capabilities
migrate. Save format 39 and simulation rules 0.11 reject older private-state gaps;
network protocol 6 is unchanged because ordinary game messages are unchanged.

AI memory remains suspended on human/offline takeover, including nested call state.
Definitions resolve once at spawn and are reconstructed from pinned content on
restore; a reused asset never shares runtime state between Systems.

Successful Dock tree evaluation publishes its private runtime at the existing
deferred controller flush, preserving the declared FixedUpdate scheduling
boundary. No failed or suspended evaluation changes private state.
