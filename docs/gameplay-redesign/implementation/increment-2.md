# Increment 2: composable Systems and common status

The user authorised starting this increment after the automated exit of
increment 1. Its physical phone/native multi-monitor and human-workload checks
remain pending and are not retrospectively marked accepted.

## Implementation contract

- Keep Station/Console/System identity and existing command admission. Add
  System-local component IDs and a closed vocabulary of supported mechanism
  definitions, typed channel declarations and bindings.
- Prove composition with a docking actuator and a power input. Active Dock
  tuning moves into its component; passive endpoint authoring remains valid.
  Power group ownership remains the System's existing authored binding.
- Use one shared channel state for Dock's engagement and latched target, with
  mechanical mating/clearance state separate. Existing Dock/Undock operations
  retain nearest-target selection and all current lifecycle/refusal semantics.
  Arbitrary target selection is not added by this migration.
- Validate supported channel meanings/types and component references before
  spawn. Human and AI operations change the same channels through existing
  System-addressed admission. Internal target latching does not grant a new
  human command or bypass knowledge/eligibility checks.
- Publish current status from authoritative damage/control and, for migrated
  Dock, its real power/operating constraints. Do not interpret historical
  refusal or absence of a nearby target as hardware failure. Include exact
  System IDs, known reasons and component references where available.
- Mount one shared status/details widget through Console core. It lists
  owned passive equipment, hides delegated rows behind a summary, preserves
  Console isolation and offers keyboard-accessible live details.
- Add visual component authoring, ordering, typed fields/bindings and inherited
  materialisation to the existing Workshop document/history workflow. No
  separate authoring store or programming surface.
- Integrate all authoritative state with save/restore/digests immediately;
  update affected compatibility versions. Migrate shipped active Dock data
  and fixtures together, with no competing legacy runtime state.

## Verification

Use targeted component/config/Dock, authority, continuation, status projection,
widget and Workshop tests during development. Demonstrate ordinary human and
AI Dock operation, passive berths, power/damage failures, ownership isolation,
working save/resume and real Workshop Test. Run required integrated gates once
on the final result before any push; record browser/native acceptance honestly.
Compare representative gameplay with increment 1 without retuning balance.

AI state trees, generic jobs/resources, new damage, new power and redesigned
combat remain in their scheduled increments.

## Active Dock content conversion

Keep the existing Dock System ID, owning Console and power-group binding. Move
its former top-level `[dock]` values into the mating component's `config`, then
remove that top-level table. Preserve the rig's dock markers. The following is
the shipped component shape; use the original content's numeric values when
converting a pack rather than retuning it.

```toml
[[system]]
id = "dock"
kind = "dock"
console = "helm"
power_group = "helm"

[[system.channel]]
name = "enabled"
kind = "boolean"
[[system.channel]]
name = "target"
kind = "entity_target"

[[system.component]]
id = "power"
kind = "power_input"
[[system.component]]
id = "mating"
kind = "dock_mating"
enabled_channel = "enabled"
target_channel = "target"
power_input = "power"
[system.component.config]
range = 200.0
engage_distance = 400.0
approach_speed = 60.0
mate_tolerance = 4.0
undock_clear_distance = 120.0
min_power_level = 2
```

Passive berths with no active Dock System retain `[dock]`. The current runtime
supports one active Dock System per entity. Channel names can be changed when
the component bindings are changed with them. Save format 38 does not convert
older live command/clearance state; resume those saves with their original build.
