---
title: NPC Doctrine Controls
type: concept
tags: [ai, npc, gm, doctrine, scenario, replay]
sources: [src/gm_npc.rs, src/gm_action.rs, src/world/config.rs, src/world/dispatch.rs, src/world/script/effects.rs, src/world/server.rs, src/ai/server.rs, src/snapshot.rs, src/sim_digest.rs, src/gm_projection.rs, gui/gm-npc-panel.js, pasm/spec/design/gm-console-t2.yaml]
updated: 2026-09-07
---

# NPC Doctrine Controls

Issue #1308 adds `[[gm_npc_doctrine_palette]]` entries with a stable `id`, display
`label`, explicit NPC `targets`, and a `doctrine` list using the ordinary strict
`DoctrineObjective` schema. The map's NPC panel receives only compatible choice
identities and labels, the selected profile, and current scored AI intent.

`SetNpcDoctrine` carries the target UUID and palette ID through normal GM
admission, canonical ordering, duplicate handling and attributed terminal results.
The Rhai `ctx.effects.set_npc_doctrine(entity, id)` action resolves an authored
entity name through the ordinary scenario dispatcher and calls the same live
doctrine applier. It does not reuse the retired `set_ai_state` action.

Compatibility checks explicit target membership, NPC identity, configured
directive consumers and the global anchors read by ordinary AI. Fleet/player
hulls and civilian traffic actors are excluded; traffic retains its own doctrine
owner. Damage and temporary Station takeover do not change authored
compatibility. Hail and Order remain unavailable because their ordinary producers
consume only the local player's mission objectives. The application boundary repeats the same checks used by the
projection before changing anything.

The applied profile replaces `BehaviourSection.doctrine`; existing scoring,
blackboards, AI producers and command appliers operate it. Route cursors belonging
to the replaced doctrine restart while unrelated mission Objective progress is
retained. An exact repeated profile is a No-op. `NpcDoctrineState` retains the
selected contents and original baseline through snapshot/replay. Unloading a
layer withdraws its available choices while already-applied doctrine continues;
restoring a snapshot with no selection clears a bootstrap selection. Default
state contributes no new digest bytes.
The digest uses an exhaustive field representation for doctrine; its authoring
schema's flattened unknown-field map is unsuitable for direct postcard encoding.

The panel's optional shared confirmation request uses category `npc.directive`
and default `immediate`. It captures intent before acceptance and creates a
correlation/Pending state only after acceptance. Activity rows include the target
ship reference so normal semantic ship filtering retains attributed results.
Withdrawing a selected choice preserves it visibly and disables Apply until the
operator explicitly selects an available choice.

See [AI Ship Unification](./ai-ship-unification.md) for the ordinary consumers and
[GM Operator](../entities/gm-operator.md) for admission and presence.
