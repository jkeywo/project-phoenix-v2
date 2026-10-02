---
title: Shared Behaviour Modules
type: concept
tags: [architecture, client, testing]
sources: [src/ai/standing_operation.rs, src/native_host/panes/operator.rs, scripts/fleet-digest-evidence.mjs, src/native_file.rs, src/native_host/media_devices.rs, editor/workshop-asset-job.js, editor/workshop-acceptance.js, gui/gm-action-feedback.js, gui/gm-feedback-presentation.js, gui/gm-npc-panel.js, gui/gm-contact-panel.js, gui/gm-despawn-panel.js, gui/gm-journal-panel.js, gui/gm-comms-panel.js]
updated: 2026-10-02
---

# Shared Behaviour Modules

GM panels use GmActionFeedback for request ownership, timers, exact operator/correlation settlement and reset. Request-shaped access keeps timer metadata out of panel state. NPC, contact, despawn and journal retain one pending request; Comms retains its 32-request limit and local result history. Session, Mission, Direct Effect and Spawn share safe operator lookup, refusal text, common result identity validation and ordered log construction through gm-feedback-presentation. Mission, Direct Effect and Spawn track before sending through GmActionFeedback.submit; Session retains semantic-registry Pending and deferred local refusal. Each panel still validates typed result fields and owns confirmation, row text and completion policy.

Workshop LOD generation and billboard capture share the asset-job lifecycle. Starts and cancellations are serialized; stale work cannot retire a replacement. Workshop acceptance separates private candidate preparation from a freshness-checked commit, preserving each tool's review timing and one undo group.

The native media catalogue keeps discovered identity, handle and ambiguity in one entry. Explicit selection and surface preflight are shared by microphone/output tests and room/private playback; adapters retain stream ownership and their existing diagnostics.

Native preference stores and Workshop share native_file for byte replacement. Preference PID temporaries retain their matching debris recognizer; Workshop retains exclusive UUID temporaries. Layout timing and Workshop multi-file transaction recovery remain in their own modules.

Fleet acceptance uses one pure digest evaluator for browser replacement, mixed recovery, native redial and mixed matrix evidence. It validates all captured observations before the scenario cutoff, rejects conflicting duplicates, and requires two complete matching checkpoints. Runtime digest production is unchanged.

Native operator layouts share a bounded placement sanitizer for Authoring, Test and Live. It normalizes the tree and global panel inventory once; each context retains its stored vocabulary, migration, defaults and postprocessing. Browser validation stays independent and uses parity fixtures.

Tractor, Dock and Umbilical share standing-operation ownership decisions. Their adapters retain Control Source gates, Objective selection, physical readiness and admitted command emission. Ownership is derived; withdrawal reports precede command-handler release reports. Dock retains its distinct engaged-only start condition.
