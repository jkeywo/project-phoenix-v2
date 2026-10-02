---
title: Shared Behaviour Modules
type: concept
tags: [architecture, client, testing]
sources: [src/sim_digest.rs, src/sim_digest_tests.rs, src/native_capture.rs, src/gm_information/pair_map.rs, src/core/task_lifecycle.rs, editor/barrel-pattern-validate.js, gui/components/continuous-helm-input.js, src/workshop/validation_support.rs, src/gm_action.rs, gui/settings-gamepad-presentation.js, gui/settings-panel.js, gui/server-settings.js, gui/workshop-edit-session.js, gui/workshop-definitions-panel.js, gui/workshop-composition-panel.js, gui/workshop-entity-panel.js, gui/stations/contextual-controls.js, src/ai/standing_operation.rs, src/native_host/panes/operator.rs, scripts/fleet-digest-evidence.mjs, src/native_file.rs, src/native_host/media_devices.rs, editor/workshop-asset-job.js, editor/workshop-acceptance.js, gui/gm-action-feedback.js, gui/gm-feedback-presentation.js, gui/gm-npc-panel.js, gui/gm-contact-panel.js, gui/gm-despawn-panel.js, gui/gm-journal-panel.js, gui/gm-comms-panel.js, editor/ordered-form.js, gui/ordered-form-controls.js]
updated: 2026-10-02
---

# Shared Behaviour Modules

Nine optional digest namespaces share component-row collection, UUID ordering with entity-index tie-breaking, silent empty handling and namespace headers through sim_digest::fold_optional_namespace. Each adapter retains row selection and exact payload folding; namespace stage order and the distinct entity, asteroid, collision and scenario walks remain local.

GM panels use GmActionFeedback for request ownership, timers, exact operator/correlation settlement and reset. Request-shaped access keeps timer metadata out of panel state. NPC, contact, despawn and journal retain one pending request; Comms retains its 32-request limit and local result history. Session, Mission, Direct Effect and Spawn share safe operator lookup, refusal text, common result identity validation and ordered log construction through gm-feedback-presentation. Mission, Direct Effect and Spawn track before sending through GmActionFeedback.submit; Session retains semantic-registry Pending and deferred local refusal. Each panel still validates typed result fields and owns confirmation, row text and completion policy.

Workshop LOD generation and billboard capture share the asset-job lifecycle. Starts and cancellations are serialized; stale work cannot retire a replacement. Workshop acceptance separates private candidate preparation from a freshness-checked commit, preserving each tool's review timing and one undo group.

The native media catalogue keeps discovered identity, handle and ambiguity in one entry. Explicit selection and surface preflight are shared by microphone/output tests and room/private playback; adapters retain stream ownership and their existing diagnostics.

Native preference stores and Workshop share native_file for byte replacement. Preference PID temporaries retain their matching debris recognizer; Workshop retains exclusive UUID temporaries. Layout timing and Workshop multi-file transaction recovery remain in their own modules.

Fleet acceptance uses one pure digest evaluator for browser replacement, mixed recovery, native redial and mixed matrix evidence. It validates all captured observations before the scenario cutoff, rejects conflicting duplicates, and requires two complete matching checkpoints. Runtime digest production is unchanged.

Native operator layouts share a bounded placement sanitizer for Authoring, Test and Live. It normalizes the tree and global panel inventory once; each context retains its stored vocabulary, migration, defaults and postprocessing. Browser validation stays independent and uses parity fixtures.

Tractor, Dock and Umbilical share standing-operation ownership decisions. Their adapters retain Control Source gates, Objective selection, physical readiness and admitted command emission. Ownership is derived; withdrawal reports precede command-handler release reports. Dock retains its distinct engaged-only start condition.

Contextual Station controls share light-DOM mounting, synchronous authoritative painting and listener lifetime. Hull documents explicitly compose each control and preserve their existing action adapters. Helm and Engineering keep compatible render exports.

Workshop Definitions, Composition and Entity share read freshness and unapplied-form retention in workshop-edit-session. Forms are captured after a validated current answer arrives, then restored after rebuilding only when selection identity and exact source match. Typed catalogs, runtime edits, Test holds and focus remain panel-owned.

Client and host Settings share device selectors and accessible gamepad status through settings-gamepad-presentation. Explicit policies preserve assignment and capability-loss choices on the client and the host's simpler status policy. Polling refreshes existing controls; input ownership and panel rebuild timing stay with the adapters. Helm touch visibility remains in gamepad-presentation.

Workshop Authoring layout generations derive vocabulary, migration placement and defaults from ordered introduction records. Vocabulary order remains distinct from migration order. Browser and native histories remain independent and retain their existing supported versions.

GM canonical journal projection uses one grant fold with explicit recorded-result and derived-result policies. Frontier reconstruction deliberately derives its results even when recorded results exist; normal projection retains durable results and derives only the remaining prefix.

Workshop validation_support owns ordered dependency overlays, source provenance, counted introduced violations and deterministic error findings. Entity, Composition and Presets retain domain rules and refusal text; Definitions shares finding construction while retaining warning and faction ranking policy.

Helm and lateral-thrust joysticks share continuous-helm-input for pointer identity, keyboard state, listener lifetime and animation scheduling. The adapters retain geometry and semantic actions. Explicit pointer policy keeps lateral submissions immediate and the main joystick on its heartbeat; disconnect retains input state without submitting neutral.

Authored blaster and torpedo patterns share barrel-pattern-validate for traversal, duplicate identities, barrel bounds and offsets. Existing validators retain their exported names, paths and domain diagnostic wording; validateFile continues using those adapters to block invalid saves.

Physical-work adapters for Tractor, Dock and Umbilical share ordered activation reports in core::task_lifecycle. Dock retains mating-state activation; Tractor and Umbilical retain subject-change activation. Physical verdicts, optional identities and component writes stay local.

Workshop ordered forms share live-row traversal, neighbour availability, swaps and surviving-row selection through ordered-form. Preset and widget adapters retain their movable-content rules; roots remain unrestricted. ordered-form-controls rebuilds before restoring adapter-owned focus. Removal tombstones, splices, DOM identities, labels and the ratings’ strictly-next focus policy remain local.

Workshop Ship, Slots, Model structure and Rhai save presentations use workshop-edit-session’s mutation runner for synchronous busy acquisition, asynchronous invocation, outcome delegation and final release. Adapters retain their generation, draft, selection and disposal predicates; Models and Scripts retain unguarded success presentation. Existing guarded, land and reading refresh behavior is unchanged.

GM detection overrides, classifications and report policies share absolute observer/target map edits and liveness pruning. Report comparisons use policy only to preserve samples; detection retains Normal-as-absence and legacy stored Normal idempotence. Ghost cleanup remains observer-only.

Native Workshop captures share directory visitation, relative-path normalization, bounded reads and file accumulation checks through `src/native_capture.rs`. Selected-root inventory, immutable dependency boot capture and Test shader support retain their own traversal admission, link refusal, limits and check timing.
