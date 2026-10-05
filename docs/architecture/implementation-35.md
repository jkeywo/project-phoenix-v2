# Implementation record: 35 module-audit improvements

This records the selected improvements against baseline `2de7d5329fba55e06ad00fbd8ca13f0cb4e8c73c`. Each audit reference is the original report's numbered finding; the original report remains unchanged. Links below point to maintained source, rather than temporary audit or handoff files.

All 35 selected improvements are implemented and integrated. The full native workspace suite passed **8,990 tests**, and the integrated JavaScript suite passed **8,601 tests**. Workspace formatting and feature-enabled clippy passed with warnings denied; PASM validate/scan/traceability each exited zero, and wiki lint checked **67 pages with zero failures**. Fresh Phoenix WASM and pure-JavaScript phone builds passed, followed by **14 browser smoke tests**, including exact native/WASM checkpoint digests, fleet admission/start policy, world startup and Workshop controls. Independent simulation passed **4,608 tests** with default features disabled, viewer passed **31 focused tests**, demo passed **11 tests**, and **three software-rendered viewscreen tests** passed. Native host and capture-tool builds passed. The layer dependency-tree check and HTML graph freshness check passed.

### 1. Resolve selected headless hull once — audit 2.1

[Boot preparation](../../src/boot/mod.rs) returns `PreparedHullContent`, carrying the parsed configuration, digest record and optional primary sidecar from one disk resolution. [Headless composition](../../src/headless/app.rs) consumes that result instead of resolving the selected hull again; native composition uses the same handoff.

Verification: boot regression tests exercise preparation without mutating the ledger and passed in the full integrated native suite. This changes content preparation, not hull selection policy.

### 2. Share the demo-build literal — audit 2.2

[Runtime build flags](../../crates/phoenix-simulation/src/build_flags.rs) include the same [literal parser](../../src/demo_build_value.rs) used by [build.rs](../../build.rs). Compile-time and runtime gates therefore interpret the same environment value.

Verification: build-flag cases passed in ordinary and demo configurations, including route absence under `PHOENIX_DEMO_BUILD=true`. PASM gates passed after the source-path correction. No second parser remains in the simulation crate.

### 3. Concentrate host registration and presentation policy — audit 3.1

[`compose_live_host`](../../src/server_app/mod.rs) owns schedule-sensitive registration: lifecycle, coordination, lobby, simulation, optional RNG installation, then world registration. [Boot profiles](../../src/boot/mod.rs) own whether a live host has the render stack, and browser, native and headless callers use that composition seam.

Verification: integrated clippy and the full host/headless native suite passed. Native Workshop remains a separate profile rather than entering live-host composition.

### 4. Own browser fleet staging transitions — audit 3.2

[`FleetStaging`](../../src/server/fleet_staging.rs) owns generation advancement, roster admission, join/leave queues, bounded lobby inputs, retry and completion. [Browser bridge callers](../../src/server/bridge.rs) use these operations; stale completion cannot overwrite the current generation.

Verification: [staging tests](../../src/server/fleet_staging_tests.rs) cover replacement, refusal and generation handling and passed in the full integrated native suite. Staging still waits for simulation adoption rather than claiming immediate acceptance.

### 5. Share initial and later entity snapshot construction — audit 6.2

[Broadcast publication](../../crates/phoenix-simulation/src/server_app/broadcast_publish.rs) uses one entity projection for initial Welcome upserts and later `EntitySpawned` messages. Initial seeding emits no spawn messages; later additions retain UUID sorting and existing removal behaviour.

Verification: the parity regression includes torus geometry, short RGB input, radar, hull, shields, infrastructure and Objective targets and passed in the integrated native suite.

### 6. Share hull damage rules — audit 10.1

[Hull damage](../../crates/phoenix-sim-gameplay/src/ship/damage.rs) shares private damage-tier and ordered weighted-selection rules through adapters for system and arc hull maps. Mutation remains local to each hull representation.

Verification: gameplay tests and the added fixed-PCG regressions passed in the integrated native suite. They pin iteration, spill/fallback and subsequent RNG draw, empty scopes and strict tier boundaries.

### 7. Own ordinary phaser cooldown completion — audit 11.1

[`ActiveBeam::complete_bank`](../../crates/phoenix-sim-gameplay/src/console/weapons/beam.rs) applies the stored positive pending cooldown, with the legacy authored cooldown fallback. All four ordinary completion paths call it.

Verification: gameplay beam tests passed. Exceptional relight handling retains its existing `end_bank` path; this consolidates ordinary completion without changing that exception.

### 8. Reuse Rhai lexical analysis — audit 14.1

[World configuration](../../crates/phoenix-sim-world/src/world/config.rs) and [script validation](../../crates/phoenix-sim-world/src/world/script/validate.rs) share significant-token scanning. Spawn-reference extraction decodes only proven string literals; computed backtick expressions remain unclaimed.

Verification: world tests passed for nested comments, escaped Unicode, raw strings and source lines. Independent review found an EOF escaped-quote bounds panic; the integrator clamped the lexer and passed two focused regressions. Extraction does not evaluate arbitrary authored expressions.

### 9. Project ship-effective Objectives once — audit 15.1

[Objective projection](../../crates/phoenix-sim-world/src/objectives.rs) combines definitions with optional instance-manager state through `effective_scored_for_ship` and `effective_visible_for_ship`. Power AI, broadcasting and Captain consumers use it; [visibility rules](../../crates/phoenix-sim-contracts/src/objective_utility.rs) have a shared contract owner.

Verification: existing tests and the projection regression passed in the integrated native suite. Visibility precedes history projection, and active terminal or unassigned instances are suppressed consistently.

### 10. Give script fixtures source identity — audit 15.3

[Script fixtures](../../crates/phoenix-sim-world/src/world/script/fixture.rs) carry `source_path` when firing and support qualified `call_in`. Direct `call_at` resolves a unique defining AST and refuses ambiguity instead of choosing an unrelated same-named function.

Verification: world fixture tests passed for duplicate names across units and a uniquely defined second-unit function. Each invocation retains a fresh execution budget.

### 11. Own fleet agreement arrival ordering — audit 19.1

[`MeshAgreement`](../../crates/phoenix-sim-session/src/lockstep/mod.rs) retains local and peer evidence through `record_local` and `record_peer`, comparing when either arrival order completes the pair. Parent lockstep callers delegate evidence retention to these operations.

Verification: agreement tests passed for arrival order, duplicate evidence and discovery. Existing public evidence and comparison access remain available for recovery/debug compatibility.

### 12. Give summary input one authority rule — audit 23.1

[Authority contracts](../../crates/phoenix-sim-contracts/src/authority.rs) combine payload eligibility (`target.summary`) with the policy boolean `target.control.accept_summary_input`. The parent admission policy no longer duplicates that conjunction.

Verification: authority matrix tests passed. Summary eligibility is checked at both admission and acceptance; control authority alone does not grant summary authority.

### 13. Isolate legacy mutable viewer workflows — audit 25.1

[Viewer composition](../../crates/phoenix-presentation/src/viewer/mod.rs) makes mutable ladder/reload/capture behaviour an explicit `LegacyViewerWorkflowPlugin` opt-in. Ordinary `ViewerPlugin`, including immutable Workshop preview, installs no legacy capture resources or polling.

Verification: viewer tests passed. Public legacy command variants and WASM exports remain compatible; the workflow is retained behind explicit composition rather than deleted.

### 14. Rebuild preview lighting only after lighting changes — audit 25.3

[Viewer command handling](../../crates/phoenix-presentation/src/viewer/mod.rs) no longer marks lighting changed after every command. Actual lighting writes still trigger [lighting application](../../crates/phoenix-presentation/src/viewer/lighting.rs); camera, gizmo and LOD commands preserve the lights.

Verification: viewer regressions cover unchanged light entities and real lighting changes. Renderer-free tests passed; no GPU screenshot comparison was performed by the worker.

### 15. Share effective primary-tier rigs — audit 26.1

[`effective_tier_rig`](../../crates/phoenix-presentation/src/entities/glb_visual.rs) gives generated identity tiers the primary offset and rotation with unit child scale. Game LOD updates and [preview loading](../../crates/phoenix-presentation/src/viewer/preview.rs) share it; root scale applies once.

Verification: pose tests passed. Undeclared baked tiers retain their own sidecar semantics; worker validation did not render GPU captures.

### 16. Retain asynchronous visual request identity — audit 27.2

[GLB visual loading](../../crates/phoenix-presentation/src/entities/glb_visual.rs) keeps pack path and variant alongside the strong scene handle in `PendingSceneHandle`. Attachment requires the current request identity; reversal and non-GLB changes retire pending work while preserving the current visual.

Verification: a renderer-free stale-B/current-A regression checks the attached `SceneRoot`, with pack and variant cases. Callers migrated to the identity-bearing constructor; real GPU timing remains outside worker evidence.

### 17. Remove disabled dust implementation — audit 28.1

The unreachable dust implementation and false registrations were removed. [Active native visuals](../../crates/phoenix-presentation/src/server/native_visuals/mod.rs) own pack-texture reset, mote removal and pool reset; the compatibility reset entry delegates there.

Verification: active-effect tests cover reset, replacement and unrelated entities. Authored dust texture paths, assets and shader support remain; pack-qualified material identity handles replacement with unchanged settings.

### 18. Make Comms priority authoritative — audit 29.2

[`CommsMessage`](../../crates/phoenix-model/src/messages.rs) stores canonical priority and derives compatibility urgency instead of retaining a second mutable boolean. Legacy decoding remains supported, and explicit `Routine` wins over a legacy urgent flag.

Verification: codec compatibility cases cover JSON/RON and passed in the integrated native suite. Digest folding derives the boolean in its existing position, preserving valid wire and digest shape; contact/fact urgency is unchanged.

### 19. Enforce the ReadinessTally invariant — audit 31.1

[`ReadinessTally`](../../crates/phoenix-model/src/wire.rs) validates deserialization through its constructor and validates both operands before checked addition. [Native GM start](../../src/native_host/native_gm/start.rs) and session policy consume the checked result.

Verification: all 16 model wire tests passed. Public raw fields remain for diagnostic/refusal paths; malformed decoded tallies are rejected while valid snapshot representation stays unchanged.

### 20. Preserve accepted-pack handoff invariants — audit 35.1

[`ValidatedModPack::into_active_pack`](../../crates/phoenix-simulation/src/world/mod_pack.rs) consumes an accepted candidate and its retained validated identity. Browser and [native pack adapters](../../src/native_host/host_lobby/packs.rs) no longer reconstruct identity or install a partial candidate.

Verification: mod-pack tests exercise refused handoffs and retained accepted identity, including id, name, version, non-blocking warnings, file content and retained archive. They passed in the integrated native suite. Acceptance and retained identity gate installation.

### 21. Own Runtime continuation transitions — audit 39.1

[Runtime continuation](../../crates/phoenix-runtime/src/continuation.rs) keeps state/replay progress private and exposes request, refusal, replay acknowledgement and exact retry operations. A prepared commit proof is consumed by guarded commit after Phoenix owner/departure effects succeed.

Verification: 39 Runtime tests passed, with migrated host/fleet consumers. Read-only status, transaction and committed accessors preserve observation; callers can no longer synthesize committed progress by field assignment.

### 22. Retain transient socket lifecycle edges — audit 43.1

[`RelaySocket::poll_events`](../../crates/phoenix-transport/src/socket.rs) carries ordered `Opened`, `Closed` and text events. [The redial supervisor](../../crates/phoenix-transport/src/relay_socket.rs) retains a disconnect/reconnect occurring entirely between game frames; relay protocol state clears before fresh registration.

Verification: socket/relay regressions cover ordered lifecycle processing and passed in integrated native execution. Old text-only adapters keep the default seam, and deployed-service behaviour still requires the existing live test.

### 23. Own successful surface/pool generation changes — audit 47.1

[`PaneFrameSink::configure`](../../crates/phoenix-platform/src/frames.rs) makes pool configuration part of the producer seam. [PaneLoop](../../src/native_host/panes/pane_thread.rs) configures after successful creation or resize, instead of inferring success from an outer command wrapper.

Verification: pane-loop regressions cover failed creation and generation changes and passed in integrated native execution. Recording sinks may use the default no-op, while pooled sinks replace the recycling generation.

### 24. Validate bounded histories on decoding — audit 51.1

[Bounded histories](../../crates/phoenix-math/src/bounded_history.rs) use custom deserialization for numeric and generic forms. Decoded length must fit capacity, including capacity zero; malformed input is refused rather than silently truncated.

Verification: 87 Maths tests passed. Serialized field shape and valid-history behaviour remain unchanged; this intentionally strengthens malformed-state admission.

### 25. Own page readiness and teardown — audit 55.1

[Page startup](../../gui/page-startup.js) installs the classic bootstrap before deferred modules and owns chrome/audio/channel readiness queues. Channel startup waits for audio publication; pagehide/pageshow and terminal disposal handle late feature arrival.

Verification: browser unit tests and the real classic/deferred startup smoke passed. [Chrome lifecycle](../../gui/page-chrome.js) releases late wake locks and preserves reacquisition intent across pending requests.

### 26. Own Workshop operation lifetimes — audit 55.2

[`createWorkshopOperations`](../../gui/workshop-edit-session.js) grants exclusive acquisition/release capabilities. Shell and panels share that gate for import, recovery and Test, check freshness, and release before restoring focus; stale owners cannot release a newer operation.

Verification: focused browser tests passed for invalidation and reread failure after committed edits. Raw Source repair deliberately retains its unvalidated editing path.

### 27. Carry delivery class through transport operations — audit 59.1

[Host connection sending](../../packages/transport/src/rendezvous-transport.js) accepts payload and delivery class together. Snapshot delivery uses the snapshot channel when open and falls back to reliable when unavailable; snapshot send failures shed that frame and reliable failures isolate their link.

Verification: transport tests passed and Phoenix/Grid adapters delegate class selection. Raw snapshot-channel access remains for compatibility; it is no longer required for ordinary adapter dispatch.

### 28. Move application preparation into adapters — audit 59.2

[Reusable join transport](../../packages/transport/src/rendezvous-transport.js) owns compatibility and opaque delivery, exposing acceptance generation and sending. Phoenix GUI owns reconnect Identify, Welcome localization/catalogue handling and raw retention; Fleet and Grid own their application acceptance/preparation.

Verification: focused adapter tests passed. Reusable transport no longer prepares Phoenix-specific application state; phone consoles remain JavaScript-only.

### 29. Install identity through one guarded path — audit 63.1

[Session-token installation](../../packages/session/src/session-token.js) guards Storage acquisition, reads and writes, removes leases/stops heartbeat on pagehide, and renews on BFCache pageshow. Phone joining requires that installer and reports transport unavailable if loading fails.

Verification: Storage-failure and lifecycle tests passed. Existing namespace/keys remain; fake Storage tests do not prove simultaneous multi-tab atomicity.

### 30. Own continuation framing and lifetime — audit 63.2

[Continuation wire](../../packages/session/src/fleet-continuation-wire.js) owns mandatory framing, bounded assembly, generation replacement and terminal disposal. Duplicate same-generation setup retains partial data; a different generation abandons it. Fleet caches the wire per physical connection.

Verification: focused tests cover malformed/nested chunks, replacements and closed operations. In-memory multi-peer proofs remain distinct from the single-host Grid network smoke.

### 31. Bound TURN acquisition — audit 67.1

[TURN worker](../../worker/src/index.js) bounds provider fetch and body consumption with independent five-second abort/race deadlines. [Browser transport](../../packages/transport/src/rendezvous-transport.js) bounds fetch/body acquisition at seven seconds and retains worker, OpenRelay and no-relay fallback handling.

Verification: deadline/failure tests passed, including cleanup. Successful providers still produce the flat response with TTL, CORS and no-store semantics; tests do not certify external provider uptime.

### 32. Own rendezvous terminal lifecycle — audit 67.2

[Rendezvous service](../../worker-rendezvous/src/index.js) makes dropping idempotent and removes maps/registry state. Non-open or throwing sends retire immediately; refusal frames precede policy closure and late callbacks become inert.

Verification: controlled `WebSocketPair` tests passed. This establishes service lifecycle logic locally; it is not a deployed Cloudflare-runtime smoke.

### 33. Admit Grid traffic through the shared protocol — audit 71.1

[Grid protocol](../../examples/grid/src/protocol.rs) uses typed application input on native and WASM. Browser admission reuses the connection registry for bounded token syntax, binding/supersession and identified recipients; JavaScript holds physical handles while WASM owns admission and stamping.

Verification: five native Grid tests and the browser/WASM checks passed, including hostile inputs. Grid phone consoles remain pure JavaScript and carry no WASM.

### 34. Own Grid activation and teardown — audit 71.2

[Grid application](../../examples/grid/app.js) owns page-operation lifetime, checks freshness after asynchronous imports/initialization, and keeps resources local to the operation. Teardown retires admission handles before freeing WASM; late callbacks and superseded connections cannot touch freed state.

Verification: focused browser lifecycle tests and Grid smoke passed. Supersession and asynchronous freed-instance access were reviewed and corrected before handoff.

### 35. Prove Grid cross-target continuation — audit 71.3

[Grid smoke](../../tests/layers/grid-smoke.mjs) restarts a native process from a temporary checkpoint to prove persistence. Native `--verify-continuation` restores known queued commands, advances five ticks and emits state/checkpoint for exact native/WASM comparison.

Verification: all four Grid smoke cases passed: native movement/reconnect/no-WASM, fresh-process persistence, matching continuation state/digest, and WASM restored reload. [Grid documentation](../../examples/grid/README.md) states the single-host smoke boundary. The separately rebuilt Phoenix WASM host also passed its native/WASM determinism smoke.

## Evidence and finalization

Owner validation included Maths (87), model wire (16), Runtime (39), contracts (136), gameplay (1,253), session (27), world (551), and presentation/viewer (331) tests. Independent simulation checking with default features disabled passed before the three late core regressions. Browser focused validation passed 424 tests, layer checking, Grid native/WASM builds and the smokes described above.

The integrated native, clippy, JavaScript, PASM and Phoenix browser results at the top cover the combined implementation. The full native run includes the late core regressions and integration fixes. Grid and startup owner proofs are recorded separately above; they do not imply a deployed Cloudflare check or a hardware multi-monitor acceptance run.
