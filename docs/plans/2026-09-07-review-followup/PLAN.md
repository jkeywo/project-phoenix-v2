# Codebase review follow-up plan

Status: implementation authorized by John on 7 September 2026. Current delivery and validation status is in PROGRESS.md; completed profiling results and conditional stop decisions are in RESULTS.md.

- Worktree: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan
- Branch: codex/review-followup-plan
- Starting commit: 40af72382d731c789d3b904bd6e86ae5a73e1f60
- Review source: 99932cfdbcba290efadd5c389aa0cc59a75124e8

## Outcomes and priority

First remove the native/browser correctness differences: startup restore must finish or fail visibly, a Session must have one effective connection owner, and both hosts must publish complete scenario metadata. Then reduce measured presentation work and deepen the World and snapshot modules without changing gameplay.

The [evidence note](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/docs/plans/2026-09-07-review-followup/EVIDENCE.md) records the measurements, their limits and links to the full reviews. This is the approved plan for the requested worktree; PROGRESS.md maps its slices to their GitHub issues and integrated commits. GitHub remains the planning authority. Intended architecture belongs in PASM alongside each implemented change, and the wiki changes only when code navigation changes.

The worktree started from committed main and was rebased onto main 76d36d24 after the pane-thread implementation landed as #1404 (718d9a90). Its interfaces are therefore integrated from the delivered commit.

## Delivery map

IDs below identify plan slices, not newly created GitHub issues. AFK means automated evidence can establish the result; HITL means a real bridge/phone run contributes to acceptance.

| Order / ID | Deliverable | Dependency | Completion evidence |
|---|---|---|---|
| First / R1 | Shared startup-restore lifecycle | None; coordinate with #1181 and #1363 | Terminal outcomes and save-gate recovery on both hosts |
| First / R2a, R2b | Session connection ownership: native fix, then browser adoption | Registry work independent; pane-adapter integration follows delivered #1404 | Shared ownership transcripts and real reconnect coverage |
| First / R3 | Complete shared scenario-catalogue projection | Independent; coordinate with #1362 | Same metadata in the real phone reducer from both hosts |
| Parallel / G0 | Reliable profiling harness and uncontended baseline | None; full pane baseline follows #1404 | Repeatable captures with provenance and validity checks |
| Existing / #1404 | Dedicated pane thread and its acceptance | Owned by the existing task | Committed implementation and existing hardware acceptance |
| Next / P1 | Surface attribution under #1405 | #1404 final event/stat interfaces | Named copy/upload causes and exact event counters |
| Next / P2 | Apply each HUD revision once | #1404 and P1 | Static HUD stops repeated forced full uploads |
| Next / P3 | Defer hidden DOM painting | P1 and P2 | Less hidden work; current state on reveal |
| Conditional / P4 | Cadence, render-scale and memory experiments | G0, P1–P3; demonstrated remaining cost | Measured benefit plus acceptable input/text behavior, or a stop decision |
| Parallel / P5 | Named renderer CPU and GPU attribution | G0 | Critical-path evidence or explicit no-change conclusion |
| Parallel / P6 | Named headless attribution and spike diagnosis | G0 | Explained costs with matched continuation/digests |
| Next / A1 | Complete successful World script-call application | Independent of host fixes | All four adapters use one complete operation |
| Next / A2 | Structural trigger pairing and generation ownership | After A1 to serialize World edits | Correct layer/handler identity through unload and restore |
| Later / A3a, A3b | Power continuation pilot, then conditional Control migration | After R1; serialize snapshot edits with A2 | Owners restore state and produce the same next action |
| Later / A4 | Complete lobby result application in one adapter | After R2's Identify/disconnect changes | Same outcomes before and after a Ship exists |
| Later / A5 | One World materialization registration | After A2; coordinate with R1 | Boot/deferred equivalence at the same lifecycle point |

Recommended execution lanes:

- **Host policy:** R1, then R2a/R2b. R3 can run in another implementation worktree; serialize edits to the browser bridge and native lobby at integration.
- **World architecture:** A1 → A2 → A5. A3 and A4 follow the host-policy changes in their shared files. A3b is conditional on the Power pilot improving locality.
- **Presentation:** G0 → delivered #1404 → P1 → P2 → P3 → conditional P4.
- **Profiling:** P5/P6 can be prepared independently of #1404. All actual captures run serially on the same quiet machine; builds and profiling never overlap intentionally.

A1 does not logically require R1, nor does R3 require R2. These lanes permit useful parallel work without three agents editing the same lifecycle module at once.

## R1 — Shared startup-restore lifecycle

**Change.** Add one target-independent startup-restore module beside the [save lifecycle][save-lifecycle]. It owns staged state, the durable GameStart roster prerequisite, layer reconciliation, readiness/rebuild decisions, verification and one terminal outcome. [Native storage][save-store] and the [browser bridge][bridge] retain their staging, persistence and reporting adapters.

**Policy.** The presence of the completed GameStart roster is durable; leaving InProgress must not disable the driver. Use the existing 1,800-frame patience value for post-bootstrap waits, explicitly covering unresolved layers as well as entity readiness so a wait cannot evade expiry. Verification precedes success reporting. Failure follows the existing cancellation path and releases capture suspension; this slice does not introduce atomic World rollback after a partial restore. Preserve fresh-session-only restore.

**Acceptance (AFK).** Reproduce GameOver during a pending restore through the production lifecycle. Cover normal readiness, late rebuildable entities, never-ready layers, failed layers, expiry, incomplete restore and digest mismatch. Assert exactly one terminal result and subsequent manual/autosave behavior, not only a predicate result. Extend [native save coverage][save-tests], [native snapshot coverage][native-snapshot] and browser save/snapshot smoke tests.

**Record.** Update the t2-peer-local-save-slot-lifecycle owner/interface links in [T2 save design][t2-design] and the affected [deterministic-simulation record][deterministic]. Coordinate with [#1363 Load Game](https://github.com/jkeywo/project-phoenix-v2/issues/1363) and [#1181 bridge Resources](https://github.com/jkeywo/project-phoenix-v2/issues/1181); their UI/global-state work is not part of this fix.

## R2 — One effective connection owner per Session

**Module.** A pure host connection registry, separate from authoritative Session state, owns binding, replacement, accepted inbound routing, stale departures and recipient selection. Its connection handle includes the transport leg and incarnation. Physical connections, send/close and channel readiness remain adapter responsibilities.

**Chosen policy.** The last accepted connection replaces the prior owner before that prior connection is closed. Binding is immutable per connection; same-token Identify is idempotent, different-token Identify is refused. Validate token length at ingress using the existing 64-character bound; reject malformed/empty/overlong and reserved identities instead of letting downstream truncation change the routing key. Keep valid browser hex tokens, native UUID tokens and existing bounded opaque tokens compatible.

**R2a (AFK).** Land the registry with the complete native cross-leg fix, integrating [transport composition][transport], [relay admission/routing][relay] and the pane adapter. Every targeted message reaches one current owner; stale inputs and stale disconnects do not reach Session policy. A superseded pane must not trigger the crash-recreate path or silently change Station layout. Integrate this adapter against delivered #1404, preserving epochs and thread/queue ownership.

**R2b (AFK plus reconnect smoke).** Expose the same policy through narrow browser-host WASM exports; JS retains its connection objects and applies registry decisions. Make registry availability part of host transport startup before accepting crew, independent of ECS App initialization. Phone clients remain pure JS. Delete the second ownership policy after both real adapters pass the same transcripts; do not land an unused registry.

**Acceptance.** Pair real RelayTransports through the existing fake-socket seams: LAN Identify(A), cloud Identify(A), then old-leg close. Assert one owner, one targeted delivery, retained Station control and no stale-input admission. Cover both delivery classes, broadcasts/exclusions, same-link re-Identify, reserved/overlong tokens, late close after replacement and pane recreation on the same token. Run the same transcripts through the browser adapter and a reconnect smoke test that closes the old channel after the replacement owns the Station.

**Record.** Update [Sessions][sessions], [rendezvous transport][rendezvous] and [native delivery][native-spec]. Keep Session authority and the normal Admission path intact; this is connection ownership, not a new transport or protocol.

## R3 — Complete typed scenario-catalogue projection

**Change.** Define one typed catalogue message/projection with scenario provenance and active-pack id/name/version metadata. Use it for native broadcasts, browser-host exports and the native scenario surface; share the scenario representation with the [delivery payload][payload]. Remove the separate field inventories in the [arbiter][arbiter], payload builder and browser message assembly.

**Compatibility.** Keep the browser's current wire spelling, including source/base and active_packs, deterministic pack ordering, first-valid-wins arbitration and curated/pinned selection behavior. Supply explicit defaults for absent additive fields. Exercise old/new JSON and binary codec fixtures against the [protocol contract][messages]; retain the version only if they establish compatibility, otherwise bump it and test the normal join refusal. Do not change compatibility policy merely to avoid a bump.

**Acceptance (AFK).** Base scenarios plus two installed mod packs produce equivalent phone LobbyState badges and active-pack rows from both hosts: before selection, after locking, on update and on reconnect. Include no-pack and curated cases. Drive actual native pack installation and browser mod-pack coverage, rather than comparing two handcrafted expected objects. Remove the stale claim that native packs are inert from the parity fixture and PASM.

**Record.** Update [game flow][game-flow], [Ship/entity configuration][ship-config] and [native delivery][native-spec]. Coordinate with [#1362 World and ship pickers](https://github.com/jkeywo/project-phoenix-v2/issues/1362); preserve its presentation and selection behavior.

## G0 — Make comparisons repeatable

Adapt the [existing profiling harness and captures][review-pack], keeping the benchmark framework already present in Phoenix. Port only needed harness logic into maintained scripts; do not commit SDK DLLs, binaries, raw traces or the copied client bundle.

Pin source revision, executable/PDB and content/bundle hashes, feature/profile settings, seed 42, Alliance Destroyer, GPU/driver, power mode, physical resolution and DPI. Use explicit monitor profiles, loopback delivery and private save/APPDATA state. Verify loaded assets and real Helm/Tactical consoles with Backfill before measurement. Close only the harness's own processes.

Run Combat Test and Falling Skyway with 40 seconds warm-up and at least 30 seconds observation for renderer-only, HUD/lobby, HUD+Helm and HUD+Helm+Tactical at 1080p. Repeat each condition three times in rotated/interleaved order with bracketing controls. Repeat the six 60-simulation-second headless runs, excluding the first 300 updates. Capture raw frame samples and exact event totals; report per-run mean/p95/p99/max, main-frame cadence, pane iterations, input/message age, fixed-tick catch-up, pixel/upload volume and private memory separately.

No simultaneous compilation or asset generation. Record background CPU and reject contaminated runs as comparative evidence; do not terminate unrelated work. Separate headless release, native measure, real browser rendering and browser automation provenance. Matching final digests support the headless comparison, not proof of every intermediate state. A performance change is accepted only when its effect is repeatable beyond control variation and its correctness/input checks pass. The old contended FPS matrix is diagnostic evidence, not an adoption baseline.

## P1–P4 — Ultralight work under existing #1405

[#1404](https://github.com/jkeywo/project-phoenix-v2/issues/1404) remains the owner of the dedicated-thread implementation. First validate its SDK creation/use/destruction thread, resize/recovery/shutdown, input routing and existing three-Station hardware target. The two-Station review matrix does not close that three-Station acceptance criterion. No new thread implementation is proposed here.

**P1: surface attribution (AFK).** Extend the final worker stats/event interfaces with surface kind/id, physical size, visibility, push count/revision, dirty/copied/uploaded pixels, full-copy reason, queue age and exact deferred/lost/stale counters. Keep main event-drain/upload queueing per frame separate from pane update/pump/render/copy per iteration. Global SDK update/render timings stay aggregate unless the SDK supplies genuine per-view measurements; controlled single-surface experiments identify raster contributors. Label residual time unattributed. Test attribution/epoch/recycling behavior at the existing seam.

**P2: HUD revisions (AFK + visual pass).** Cache encoded HUD state by revision and track the last successfully applied revision per loaded view. Apply a new revision once, retry failures, and force the latest state on load/reload/reveal/resize as needed. Avoid same-value DOM assignments. A static non-alert HUD must cease continuous forced full uploads. Heading/hull changes, red-alert animation, game-over, first paint and recovery still draw correctly. Preserve dirty detection for animations and known SVG dirty-bound exceptions; never restore blanket noforce.

**P3: hidden painting (AFK + reveal pass).** Retain latest hidden HUD/lobby state and necessary command servicing while deferring unnecessary DOM application. Reveal must paint current state immediately without reloading the permanent page. Test phase transitions, F9, resize while hidden and commands that remain required. Do not stop the entire SDK or discard reliable messages because one view is hidden. Keep the change only when P1/G0 attribute a reduction without lifecycle regressions.

**P4: conditional experiments (HITL).** If real cost remains, test selective visual cadence independently of input/message service. Global render may still rasterize, so reduced push/copy rate is not evidence of reduced raster cost. Test render scale as physical-size and device-scale changes together, with text legibility and coordinate mapping judged on the bridge. No blanket rAF slowdown. Measure 4K/high-DPI and repeated resize cycles only for relevant deployment/scaling concerns; investigate lazy staging for never-painted permanent views if measured memory justifies it. Stop with a recorded no-benefit result when appropriate.

Across P1–P4 retain opaque BGRA row copies, in-place texture updates, the three-buffer staging floor, snapshot supersession, reliable ordering, bounded queues and the recovery budget. No renderer replacement, SDK GPU backend, new visual design or promised percentage speedup is in scope. Update [native delivery][native-spec], [performance measurement][perf-spec] and the affected acceptance notes. These are deliverables within [#1405](https://github.com/jkeywo/project-phoenix-v2/issues/1405), not duplicate top-level issues.

## P5–P6 — Renderer and headless investigation

**P5 (AFK evidence, hardware capture).** Obtain genuine named-system CPU spans/stacks on the renderer-only workload, starting with PrepareBindGroups, ManageViews, PrepareAssets, PreUpdate and PostUpdate. Use a symbol-compatible profiling build and control/instrumented/control runs. Correct extraction deferred overlap and account for parallel execution; accumulated System wall durations are not exclusive stage costs. Seek GPU timestamp queries or a GPU capture only through a supported profiler/adapter path; record availability explicitly. Implement only an attributed repeated allocation, regeneration or preparation cost. Lowering geometry or visual quality requires actual GPU evidence. Stop if no material critical-path cost is established.

**P6 (AFK).** Attribute named Systems inside Phoenix Physics, Publish and PublishAggregate, and correlate Combat Test spikes with simulation ticks/events. Preserve deterministic iteration and continuation; distinguish Phoenix Physics from Rapier. Compare equal hull/seed/tick counts with controls and repeated captures. Add a longer interval, seed sweep or entity-scale case only if the initial attribution exposes a workload-dependent concern. Do not retune Rapier or change gameplay based on the current aggregate timings. A documented explanation and sufficient measured headroom is a valid completion.

Record any resulting collector/interpretation changes in [performance measurement][perf-spec]. Performance observers remain outside authoritative state. No new CI wall-clock gate or baseline adoption is part of this plan without the existing deliberate adoption step.

## A1 — Complete successful World script-call application

Deepen the existing [World script module][script-effects] so one operation consumes the complete CallEffects plus invocation context, clock and event destination. Its implementation owns immediate actions, delayed work, callbacks, Comms opens, deadline changes and commitments. Migrate trigger, callback, Comms root and Comms response adapters together; retain their owner context and event eligibility.

Keep the pure command dispatcher. Deadline mutation remains outside ActionCmd because it edits callback scheduling. Test one call exercising all six collections through the production interface, anchored/unanchored clocks, layer propagation and a raising script that commits nothing. Preserve trigger-produced same-tick event chaining and the other adapters' current eligibility.

Done means callers no longer carry the completion protocol; a wrapper that only bundles the same obligations fails the deletion test. Update [scenario scripting][scripting], [trigger pipeline][triggers] and [Comms][comms-spec]. No authored ordering or budget change.

## A2 — Own trigger pairing and generation

One World trigger module owns ordered entries pairing continuation with an optional ScriptHandlerRef and their observer generation. Its interface provides layer append/remove, ordered evaluation, continuation capture/restore and observer reads. ASTs and execution budgets remain with WorldScriptRuntime.

Replace independent mutation of trigger_states and handlers. Migrate execution, snapshot, digest and observers together. Test load A/B/C → remove B → fire C; only C's script executes. Reload B deterministically, check same-length remove/add advances generation, and save/resume layered latches before firing their handlers. Retain failed-activation and same-tick unload/callback tests.

Preserve serialized trigger-index meaning, layer activation order, digest traversal and optional script-runtime fixtures. No stable-ID or save-format redesign. Update [scenario scripting][scripting], [trigger pipeline][triggers] and [deterministic simulation][deterministic]. Sequence after A1 to avoid overlapping edits to [World server][world-server].

## A3 — Owner-held continuation, starting with Power

**A3a.** Move Power's explicit saved projection/conversion beside [Ship Power][ship-power] and [PowerSystem][power-system]. Give the owner the capture/restore operation while [snapshot orchestration][snapshot] keeps the envelope, canonical traversal, identity rebinding, prerequisites and cross-System ordering. Preserve PowerState's serialized fields; a compatibility re-export is acceptable.

Restore non-default allocations in insertion order, depleted batteries and exhaustion locks; also restore default saved values over a non-default bootstrap. Exercise the next Power tick and resulting modifiers, plus [fresh-app continuation][snapshot-tests]. Equality immediately after restore is insufficient.

**A3b, conditional.** If the pilot removes caller knowledge, apply the pattern to Helm control and Radar target memory while preserving the ControlState payload. Test desired versus last-applied inputs, impulse phase, cleared/default inputs, target presence/absence and the first resumed action.

Update [deterministic simulation][deterministic], [Power][power-spec] and only the Helm/Radar realization records actually changed. Do not add a generic save registry, use State Census as implicit fold order, derive serialization on runtime/third-party types indiscriminately, or widen digest coverage. Broader snapshot-divergence issues are separate work.

## A4 — Complete lobby result application

Deepen the existing [Bevy lobby adapter][lobby-server] so it resolves pre-spawn versus loaded-Ship state and applies a result completely: Station Ratings, Control Sources, countdown, phase and outbound messages. Identify, SelectStation, ReleaseStation, SetReady, SetSpectator and other consumers evaluate their pure handlers once.

Test observable outcomes with and without a Ship, including reconnect yielding, claims/releases, readiness, phase changes and disconnect-before-Identify in one tick. Preserve per-variant handlers, authority policy, phase gates and scheduling. Do not recreate the retired monolithic dispatcher.

Land after R2's connection work in the Identify/disconnect area. Update [Sessions][sessions] and [Station authority][station-authority]. No performance gain is claimed.

## A5 — One World materialization registration

A shared composition operation registers the authoritative compile/setup/spawn/init/load chain for a supplied schedule. Both Startup and [native runtime loading][world-load] use it. Preserve compile_world_scripts before setup_world before spawn_world_entities, including ordering relative to other plugins.

Native transactional ingest, hull selection, roster reset, radar replacement, tick-zero mint parking/restoration, re-Welcome and rollback remain in its adapter; render-only startup remains outside the shared authoritative chain. Prefer shared registration over a nested schedule that silently breaks existing edges.

Extend [native lobby equivalence][native-lobby-tests]: boot-load and deferred-load the same composed World after different lobby delays; compare name→UUID association, entities, layer order, trigger continuation, flags and selected hull at equivalent lifecycle points. Retain failed-ingest unwind, ready-flag clearing, re-Welcome, subsequent-mint and Boot Profile parity coverage.

Sequence after A2 and coordinate with R1. Tests that do not open panes need not wait for #1404. Update [native delivery][native-spec], [World files][world-files] and the affected [script ordering record][scripting]. No Boot Profile merger or World identity change.

## Verification and handoff

For each code slice: reproduce the failure or characterize the public behavior, implement through the real module interface, run targeted tests/checks, then review the final diff. Do not commit a failing regression test as a standalone deliverable. Reuse existing native integration, client reducer and browser smoke coverage; add tests for the newly shared behavior rather than mirroring implementation details.

During implementation use cargo check and relevant test targets. Before each eventual push, run the repository's complete required gates once at the prescribed final integration point, including PASM validation/scan/traceability when its records change. A Rust-only pass does not establish browser compatibility; touched JS/wire paths need their real client and WASM/browser checks. Ultralight SDK and physical bridge checks supplement the feature-off tests. Reuse completed evidence while its relevant inputs remain unchanged.

Update PASM with intended ownership/behavior in the same implementation slice, then update wiki realization links after code changes. Run the wiki schema lint when closing the batch. Do not update performance baselines silently or present a temporarily skipped acceptance leg as completed.

PROGRESS.md records the current handoff and remaining checks. Recheck current GitHub state and source before filing additional slices; tracker checkboxes can lag delivered implementation. Scope changes discovered by regression tests belong explicitly in the slice description, not in unrelated cleanup.

[save-lifecycle]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/save_slots_lifecycle.rs
[save-store]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/save_slots_store.rs
[bridge]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/server/bridge.rs
[save-tests]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/tests/save_slots_persistence.rs
[native-snapshot]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/tests/native_host_snapshot.rs
[t2-design]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/design/t2-platform-input-feedback.yaml
[deterministic]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/deterministic-simulation.yaml
[transport]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/native_host/transport.rs
[relay]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/native_host/relay_transport.rs
[sessions]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/sessions-replication.yaml
[rendezvous]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/rendezvous-transport.yaml
[native-spec]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/native-delivery.yaml
[payload]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/delivery/payload.rs
[arbiter]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/lobby/scenario_arbiter.rs
[messages]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/core/messages.rs
[game-flow]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/game-flow.yaml
[ship-config]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/ship-entity-configuration.yaml
[perf-spec]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/performance-measurement.yaml
[script-effects]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/world/script/schedule.rs
[scripting]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/scenario-scripting.yaml
[triggers]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/trigger-pipeline.yaml
[comms-spec]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/comms.yaml
[world-server]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/world/server.rs
[ship-power]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/ship/power.rs
[power-system]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/modifiers/power_system.rs
[snapshot]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/snapshot.rs
[snapshot-tests]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/tests/snapshot_resume.rs
[power-spec]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/power-modifiers-regions.yaml
[lobby-server]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/lobby/server.rs
[station-authority]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/station-system-authority.yaml
[world-load]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/src/native_host/world_load.rs
[native-lobby-tests]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/tests/native_host_lobby.rs
[world-files]: C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/pasm/spec/architecture/world-files.yaml
[review-pack]: C:/Coding/project-phoenix-v2/.phoenix/reviews/2026-09-07/README.md
