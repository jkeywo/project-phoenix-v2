# PASM Foundation

The **Phoenix Architecture & System Model** — this game's authored model — lives
in [`spec/`](./spec/). The tool that reads it is the fleet's, kept in
[vellum](https://github.com/jkeywo/vellum) and consumed as a git subdirectory
dependency pinned to a rev in `pyproject.toml`.

Phoenix used to vendor a copy of that tool here. It was byte-identical to
vellum's apart from one scanner regex, which has since been upstreamed
(jkeywo/vellum#10), so the copy was only somewhere for the two to drift apart.
The tool's own documentation — command reference, finding categories, parser
internals — lives with the tool.

## Running it

```powershell
uv run pasm validate
uv run pasm scan --json > target/pasm/scan.json
uv run pasm traceability --json > target/pasm/traceability.json
```

CI runs `validate` and a gating `scan` through vellum's `pasm-validate`
composite action, then produces the traceability report and uploads both as the
`pasm-reports` artifact.

To work on the tool itself, edit a sibling vellum checkout and uncomment the
`[tool.uv.sources]` block in `pyproject.toml`. That override must never be
committed active: it resolves whatever is on disk instead of the pinned rev,
which makes the pin worthless. Land the change in vellum, then bump the rev.

`pasm validate` resolves implementation paths relative to `pasm/spec` by default. For nested fixtures or an external spec root, pass `--workspace-root <path>` explicitly.

## Design and tooling notes

The rest of this file is phoenix's running record of what the model asserts and
why. Entries describing PASM phases record the tool capabilities this repo
relies on; the remainder are this game's own architecture decisions.

- The PASM design docs live beside the authored YAML under [`spec/`](./spec/), making `pasm/spec/` the canonical specification root for this game.
- Authored PASM YAML lives under [`spec/`](./spec/). The seed files there are intentionally small so `pasm validate` has a real default target.
- The first parser pass uses **PyYAML**'s composed node tree instead of a custom grammar. That keeps the implementation simple while still preserving source line/column information.
- The restricted YAML subset currently accepts mappings, sequences, strings, and explicit booleans only. Unknown fields are rejected, anchors and custom tags are rejected.
- Phase 3 types the `architecture` section and validates ownership, forbidden dependencies, trust-boundary message validation, and non-authoritative ownership of authoritative state.
- Phase 4 types the `implementation` section and validates declared paths, empty mappings, and missing implementation coverage for shipped entities. `pasm query implementation <id>` shows the declared mapping without needing repository scanning.
- Phase 5 builds a repository-wide observed model: `pasm scan --json` records the Git revision, Cargo packages, Rust/JS/TS/HTML files, source-located symbols and imports, and resolved local file dependency edges. It keeps the declared-file symbol/message checks as a compatibility layer.
- PASM derives entity-level conformance only from direct resolvable local file edges with unambiguous PASM ownership. It warns when a declared dependency has no matching direct edge or code introduces an undeclared edge, and errors when an observed edge violates `must_not_depend_on`. Package imports, shared implementation files, and indirect runtime relationships remain inventory evidence rather than pretending to be semantic dependencies.
- `pasm scan` emits its inventory even when `pasm validate` has findings, while returning the same nonzero exit code so CI and migration work remain honest.
- `symbols` means top-level Rust items, top-level/exported JavaScript/TypeScript declarations, and HTML `id` attributes. JSON keys like `core_systems` should be recorded via the surrounding builder symbol or a declared message, not as implementation symbols.
- The first real Phoenix slice is intentionally the shipped **damage ownership + repair flow**, not a fictional diagnosis mechanic. The PASM docs recommend "engineering damage diagnosis", but the current repo surface is closer to authoritative damage state, replicated repair blackboards, and repair-team dispatch, so the authored PASM spec models that truthfully.
- The second real Phoenix slice is the shipped **helm control + helm AI flow**. PASM models the code as it exists today: human helm uses admitted client-to-host commands, while helm AI reads doctrine/world snapshots and writes authoritative motion plus impulse state directly instead of synthesizing helm commands.
- Helm capability is ship-specific rather than universal. PASM should treat engines, steering, lateral thrust, vertical thrust, impulse, and boost as authored per-ship capabilities; a ship with no engines is effectively a starbase and should not be special-cased elsewhere.
- The intended next-step helm architecture is a shared ship-level 3D desired-motion plus hazard-assessment surface, not separate per-subsystem avoidance math. Fine helm systems should consume that shared intent and express it according to their own capabilities and hazard filters.
- That shared helm contract should include both `desired_velocity_local` and `desired_facing_local`, because arc-bearing and docking concerns can diverge from pure travel direction.
- The planned vertical movement capability uses `vertical_movement_mode = none | bounded | full_3d`. `bounded` is AI-only above/below avoidance within authored limits; `full_3d` implies six-degree-of-freedom craft.
- Hazard assessment should publish reusable capability-style facts such as `movable`, `dangerous`, and `size_rating` instead of rigid object categories; subsystems decide how much those facts matter.
- `helm-vertical-thrust` is intended as a separate fine system even before it becomes player-facing. Its first responsibility is collision avoidance against moving hazards only, while engines and lateral thrust may still respond to static hazards.
- In bounded vertical mode, ships should drift back toward their authored cruise plane gradually after danger passes.
- Once ship Y separation becomes real simulation truth, torpedoes should move in full 3D as well.
- Helm AI is intended to select the same typed actuator inputs as a player, with no human-versus-AI branch after input arbitration. The current direct-write helm AI remains an explicitly temporary implementation shape.
- Arc-bearing requests control facing, not translational drift. Docking is a separate future motion intent that may legitimately request slow reverse and lateral movement.
- Impulse steering uses an authored harsh multiplier (standard `0.1`) rather than a total steering lockout; boost is disabled while impulse is active (issue #740).
- Central hazard assessment should publish 3D force contributions as well as `movable`, `dangerous`, and `size_rating`. A ship ignores hazards smaller than itself, and each actuator's authored sensitivity to those forces determines its response priority.
- The Helm shared-motion migration deliberately uses symbol-removal conditions, not a legacy-caller allowlist. Phase 5 observes whole files and cannot attribute references within shared `src/ship_plugin.rs` to individual PASM entities; caller findings there would be noise. The remaining pending removal conditions still expose the real unfinished migration.
- TOML observation is deliberately narrow: PASM inventories authored TOML and resolves only explicit template/world path keys. It does not pretend to execute arbitrary authoring semantics, but it supplies evidence for content-pack and template composition references.
- Host-page console actions use a dedicated local token rather than a remote station session. PASM treats it as a bounded host-local authority path to the local ship, never as an implicit station-ownership bypass for arbitrary entities.
- Deprecated code is represented only while it remains in the repository and has a removal or migration condition. Once deleted, remove its PASM entity rather than preserving a historical implementation record.
- Phase 6 adds a typed `migration_plan` section on `migration` entities and `pasm query migration <id>`. The initial migration runtime is deliberately narrow: it evaluates fixed removal-condition predicates, checks undeclared legacy callers against observed declared files, flags overlapping legacy/target writers, and reports target-side legacy residue as a heuristic rather than pretending to solve general refactor analysis.
- The Red Alert design slice's migration from `ToggleRedAlert` to `SetRedAlert { active }` has landed (issue #748). The explicit desired state is idempotent across retries, duplicates, and stale UI state: the host assigns the requested value rather than inverting. The former toggle command, handler, and AI producer are removed, so the transitional `current-*` residue entities and the migration-plan entity have been deleted rather than preserved as a historical record.
- Red Alert target intelligence is deliberately scoped to the selected Sensors radar contact. A target that has no Red Alert capability exposes no alert value; PASM does not require alert markers for every radar blip.
- Every behaviour-driven AI ship must have an AI-only `red-alert` capability. The intended spawn rule adds it when an NPC author omitted it, while allowing explicit TOML declaration for clarity.
- Phase 7 models game design as typed optional sections on normal PASM entities. Relationships use existing semantic entity IDs; local descriptive values remain strings. Phase 7 validates design-internal ownership, reference integrity, reveal conditions, resource flow, and recovery only. It deliberately does not infer code enforcement until Phase 8.
- Phase 7 accepts both the initial concise entity vocabulary (`role`, `verb`, `decision`, `information`, `failure`) and the v1.0 writing-guide vocabulary (`player_role`, `action`, `information_set`, `failure_state`). `player_role` and singular `reveal_condition` are supported as documented aliases; authored output normalises them to the typed owner-role and reveal-condition fields.
- The authored Phase 7 slices cover repair diagnosis, Helm control, and Red Alert. They record intended player agency and information gates without asserting that proposed architecture is implemented; Phase 8 now makes those design-to-architecture checks explicit.
- Phase 8 adds typed `architecture_links` and `enforcement_links` to game-design declarations. Links must target architecture entities. `pasm traceability` reports each design entity's architecture path, mapped implementation files, and actual mapping status (`declared`, `confirmed`, `suspected`, or `mixed`) rather than inferring implementation from a path alone. Cross-domain validation requires linked player verbs, enforces restricted-information and protected-decision boundaries, and errors when a linked implemented or partially implemented architecture entity lacks an implementation mapping. Proposed architecture is reported as design-only rather than treated as missing code.
- Phase 8 does not claim to prove runtime automation or bypasses from prose such as `must_not_be`; that needs a later semantic/dataflow audit. The current deterministic check verifies that protected decisions name an authoritative enforcement boundary and leaves deeper behavioural analysis visible as future scope.
- Phase 9 adds `pasm scenario <file>` for lightweight ordered facts, actions, reveals, failures, and recoveries. Scenarios may declare `initial_facts` and action `requires_facts`; PASM also evaluates their finite, monotonic fact-transition closure independently of the written step order. It does not attempt general runtime reachability, production physics, or gameplay-value simulation.
- The world-file design keeps an authored `name` as the unique internal reference key and adds independent player-facing `display_name` text. World authoring is all-or-nothing: duplicate or unresolved references prevent the complete root-world composition from activating. The current runtime supports layered worlds and `parent:` flag lookup; PASM records broader `parent.<name>` and `child_world.<name>` object references as a proposed composition contract until implemented.
- The bounded Phase 5/6 audit also scans the whole supported-source inventory for lexical legacy-symbol references outside PASM-mapped files. These are source-located migration candidates, not asserted calls or dataflow; PASM still does not claim to prove runtime authority, actor identity, or information flow.
- Phase 10 adds provider-neutral semantic audit handoff: `pasm audit bundle <entity>` emits only that entity's PASM declaration and declared source slices, including a deterministic bundle fingerprint. `pasm audit report <json> --bundle <bundle.json> --persist-dir <dir>` validates source-linked structured findings for `architecture`, `migration`, or `design-alignment` and persists canonical revision-bound evidence plus the exact reviewed bundle. Semantic reports remain advisory and never replace deterministic `pasm validate` findings.
- Phase 11 adds `pasm context --entity <id> [--depth N]`, a deterministic task bundle of explicitly linked PASM entities, implementation paths, migration/evidence declarations, and reported omissions. It deliberately omits source contents and unmodelled runtime relationships.
- Phase 12 runs `pytest`, `pasm validate`, revision-linked `pasm scan`, and `pasm traceability` in CI. The scan and traceability JSON files are retained as the `pasm-reports` CI artifact; external semantic-audit reports remain advisory.
- The next-round flow is a shared pre-scenario selection lobby: the server QR is available before a world loads; either the server screen or any connected player may make the first valid scenario and ship selection; there is no vote; and GameOver returns every participant to that same selection state. The second round re-enters the same `SelectScenario` / `SelectPlayerShip` arbiter flow as the first, reusing the already-loaded world without a page reload (issue #756 retired the legacy `ConfirmScenario` compatibility path).
- Comms dialogue responses are immediate and irreversible once submitted. Future world authoring uses an optional `important: true` response marker solely to request a clear client-side warning and confirmation before submission; it neither changes station authority nor adds server-side reversal semantics.
- Out-of-range Comms responses should remain visible but greyed out. A stale or forced response rejected by the host must briefly flash red on the attempted control; the necessary rejection feedback is future protocol work.
- Navigation owns the authoritative shared waypoint, visible on Navigation and radar surfaces. Helm AI should consume that destination regardless of whether Navigation AI or a human Navigation operator set it. `NavigateTo` remains an explicit level-3 advisory request, not the normal AI movement path.
- AI-controlled Shields focuses the arc with concentrated recent incoming damage using authored windows and thresholds; health imbalance is a fallback. Human Shields remains exclusive while it controls the system. The current damage-history implementation is recorded as partially implemented because it compares damage deltas as prior health.
- AI-controlled Sensors should choose an appropriate hostile only within its own damage-scaled horizon, then use the existing level-3 designation and frequency-intelligence paths to advise Tactical. Tactical retains firing-target authority.
- Future Power AI is data-defined per authored power group, never a fixed Helm/Weapons/Sensors rule list. Every authored rule declares the minimum battery reserve at which it may apply, so allocation planning avoids avoidable brownouts without a global emergency-mode exception.
- All NPC and Backfill AI is intended to move to inline data-driven policies owned by individual fine systems. Policies share the world-trigger expression style, may be stateless or stateful, rank targets per system, emit only typed system inputs, and replace top-level `[behaviour.doctrine]` rather than coexist with it permanently. The normative contract is `spec/DATA_DRIVEN_FINE_SYSTEM_AI.md`.
- Shared viewscreen arbitration should be last valid command wins, independent of requesting console. Fixed source priorities are a current implementation detail to remove; command admission still controls which modes each station may request.
- On Game Over return, connected players retain identity and connection but all station claims and ready state clear before the next shared scenario and ship selection.
- A station holder may change its authored rating during an active round. This is an authoritative control-source transition, not a local settings preference: the host applies it immediately and replicates the selected rating to every client.
- Editor validation distinguishes errors from warnings: definite errors block save and mod-pack export, while warnings remain visible but non-blocking for multi-file authoring.
- A future mod pack is a TOML-only ZIP uploaded to the server page before scenario selection. It is session-scoped, overlays base content by exact supported authored path, and uses a required manifest to name only the root worlds added to the selectable scenario catalog. The pack is rejected as a whole if its manifest or composition is invalid.
- Regular selectable scenarios should likewise be explicitly listed in `assets/scenarios.toml`, using the same manifest contract as a mod pack. The selection catalog composes those base entries with validated mod-manifest entries.
- The server page carries one settings cog (top-left) with Debug/Cheat, Audio, and Gameplay tabs. The Debug/Cheat tab remains available in every build except the public demo, which omits it entirely (`PHOENIX_DEMO_BUILD=true`, a flag distinct from the `TRUNK_BUILD_RELEASE` size switch so the GitHub Pages dev host keeps its debug tooling). Its diagnostic views are host-local, while god mode, instagib, and teleport-to-waypoint are intentional host-only simulation overrides and must not be misdescribed as harmless diagnostics. Teleport is disabled when no shared Navigation waypoint exists. Pause and exit-to-lobby live on the un-gated Gameplay tab because they are gameplay controls; the mid-mission exit is honoured for the host page only.
- Captain does not need a separate default-camera control: selecting Fore or Cinematic is the deliberate way to reclaim the shared viewscreen from a system surface.
- Future in-game help is ship-authored rather than static client text. Each ship manual has tabs for all stations: ship-authored station overview explains the role, while configured systems generate vessel-specific manual sections from their actual values and rating-to-AI-control mapping.
- During an active round, the existing Backfill-to-human hand-off after claiming an empty station is presented as **Take Station**. **Ready** remains the lobby-only label for collective round start.
- Leaving a station during an active round requires client confirmation because it immediately restores Backfill AI; releasing a station in the lobby remains immediate.
