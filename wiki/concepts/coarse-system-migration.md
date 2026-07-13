---
title: Coarse-system migration
type: concept
tags: [stations, systems, migration, system-registry, prd-487, prd-517]
updated: 2026-06-23
---

# Coarse-system migration

Status of the migration from per-console message dispatch to the unified coarse-system control path. All 9 consoles must register a system kind, accept `ControlSystem` dispatch, gate on `ControlSourceResolver::policy_for`, and emit channel-3 traffic via `CoordinationEnqueue`.

## SystemId naming convention

Pinned by issue #525. All `SystemId` wire strings follow one of three patterns:

| Pattern | Rule | Examples |
|---------|------|---------|
| **Coarse system** | Lowercase kebab matching the system kind id | `"helm"`, `"tactical"`, `"red-alert"` |
| **Fine system** | Kind id + `-` + instance suffix | `"phaser-fore"`, `"torpedo-tube-fore-port"` |
| **Ownerless capability** | Bare capability id (lowercase kebab) | `"red-alert"`, `"viewscreen"` |

Multi-word ids always use hyphens (`-`), never underscores. The `*_SYSTEM_ID` constants in `src/ship/system_registry.rs` are the authoritative source; always use the helpers (`helm_system_id()`, `tactical_system_id()`, etc.) rather than inline string literals.

### `red_alert` vs `red-alert` quirk

The registry kind key uses `"red_alert"` (snake_case, `RED_ALERT_KIND`) for legacy reasons, while the wire `SystemId` is `"red-alert"` (kebab, `RED_ALERT_SYSTEM_ID`). All other systems have identical `*_KIND` and `*_SYSTEM_ID` values. New systems must use the same lowercase-kebab string for both.

## Coarse-system status (as of issue #529)

| Console | Kind registered | `ControlSystem` dispatch | `policy_for` gating | Channel-3 via `CoordinationEnqueue` | Issue |
|---------|----------------|--------------------------|---------------------|--------------------------------------|-------|
| Captain | ✅ `captain` | ✅ | ✅ | n/a | #499 |
| Helm | ✅ `helm` | ✅ | ✅ | ✅ | #497 |
| Tactical | ✅ `tactical` | ✅ | ✅ | ✅ | #491 |
| Power | ✅ `power` | ✅ | ✅ | n/a | #500 |
| Sensors | ✅ `sensors` | ✅ | ✅ | ✅ | #498 |
| Shields | ✅ `shields` | ✅ | ✅ | ✅ (#528) | #502/#528 |
| Comms | ✅ `comms` | ✅ | ✅ | ✅ | #503 |
| Viewscreen | ✅ `viewscreen` | ✅ | ✅ | n/a | #505 |
| Repair | ✅ `repair` | ✅ (#526) | ✅ (#526) | n/a | #525/#526 |
| Navigation | ✅ `navigation` | ✅ (#527) | ✅ (#527) | n/a | #527 |

## Fine-system ids (in-flight, PRD C)

Fine-system decomposition (e.g. `"phaser-fore"`, `"torpedo-tube-fore-port"`) is tracked by issues #511–#515. Three have shipped:

| Issue | Coarse system | Shipped fine kinds |
|-------|---------------|--------------------|
| #511 | Helm | `helm_joystick`, `helm_engine` (port + starboard), `helm_radar`, `helm_impulse` |
| #512 | Tactical | `phaser_bank` (fore + aft), `torpedo_tube` (fore-port + fore-starboard + aft), `torpedo_magazine` |
| #513 | Power | `power_reactor`, `power_battery` |
| #514 | Shields | `shield_arc` (variable count, per `[[shield_arc]]` TOML block; player ship = 4 arcs fore/port/aft/starboard; NPCs = 1 omni arc) |
| #515 | Comms / Captain / Viewscreen | closed as substantially done — Captain / Red-Alert / Viewscreen shipped via PRD #487; Comms deliberately left coarse (single narrow console; splitting into inbox/transmitter/scanner deferred pending a damage-driven rationale) |

Under #512, the coarse `tactical` `[[system]]` block was **deleted** from all 5 ship TOMLs (player_ship + 4 NPC ships). `TACTICAL_SYSTEM_ID = "tactical"` is retained as a coordination surface for ship-level operations (SetTarget / SetPhaserMode / SetPhaserFrequency); their authorisation gate is "any phaser bank accepts human input" (option c in the issue), so no coarse block is needed. Fine kinds registered but not present on a given ship default to a fallback coarse-tactical gate — this preserves NPC behaviour where a ship declares bank ids that don't match the player-ship convention (`"port"`/`"starboard"` versus `"fore"`/`"aft"`).

Tube-to-magazine communication uses [`InterSystemPayload::ClaimTorpedoRound { tube }`](../../src/core/messages.rs) on channel 2, mirroring the `DrainWeaponsBattery` pattern from #559. The magazine consumer refuses claims when its hull entry — declared as `[[hull.system_hull]] system_id = "torpedo-magazine"` in TOML (pre-#619 this block was spelled `[[hull.console_hull]] console = "TorpedoMagazine"`) — is at Disabled/Destroyed tier.

Under #513, the coarse `power` `[[system]]` block was **deleted** from all 6 ship TOMLs (player_ship + 5 NPC ships). Both `power_reactor` and `power_battery` live on the `power` station (single station holder). Allocation input (`SetPower` / `SetPowerGroupAllocation`) targets `power-reactor`; channel-2 battery drain (`DrainWeaponsBattery` from active phaser beams) targets `power-battery`. Both fine systems read shared state via the same per-entity `ShipPowerSystem` component (option (a) — same-ship, same-tick), which is why no inter-system messaging is required *between* reactor and battery. `POWER_SYSTEM_ID = "power"` is retained as a stable string only so the JS panel can continue to read the aggregate `blackboards['power']` entry.

**Deferred (issue #513):** the ship TOML's `[power] capacity/rates/emergency_threshold` and `[power.ai]` blocks remain ship-wide rather than being split into per-fine-system config blocks. Runtime `ShipPowerSystem` is still per-ship; a designer-facing knob split (reactor-specific capacity vs battery-specific capacity, etc.) is a future PRD.

Under #514, the coarse `shields` `[[system]]` block was **deleted** from `player_ship.toml`, along with the coarse hull entry that guarded it (spelled `[[hull.console_hull]] console = "Shields"` in the pre-#619 TOML shape; post-#619 the equivalent block would be `[[hull.system_hull]] system_id = "shields"`, but it too is gone under #514). `[[shield_arc]]` blocks in the ship TOML are the designer-authoring surface: each block declares `id`, `label`, `center_deg`, `width_deg`, and optional per-arc `max_hp` / `regen_per_sec` / `offline_duration` / `hull_max_hp` overrides; the parser auto-synthesises a matching `[[system]] kind = "shield_arc" id = "shield-arc-<id>"` entry. Player ships have a `shields` station so synthesised arcs are player-controlled (`ai_only = false`); NPC ships have no `shields` station so their single omni arc is `ai_only = true` and ownerless. `SHIELDS_SYSTEM_ID = "shields"` is retained as a stable string only for the aggregate `blackboards['shields']` entry (the JS panel's primary read); all control-input and coordination lookups target per-arc `shield-arc-<id>` SystemIds. `SetShieldArcFocus { focused: bool }` replaces the old `SetShieldFocus { facing: Option<ViewDirection> }` payload — each arc button targets its own SystemId. Per-arc hull HP is tracked in a new `EntityShipArcHull` component (wrapping pure `ShipArcHull`); damage is distributed proportionally to overall hull damage, and `sync_console_damage_tiers` iterates the arc-hull map to flip `shield-arc-<id>` in/out of `offline_systems` on Disabled/Destroyed tiers.

## Key files

- `src/ship/system_registry.rs` — All `*_SYSTEM_ID`, `*_KIND`, `*_AI_CONTROLLER` constants and `*_system_id()` helpers.
- `src/ship/control_source.rs` — `ControlSourceResolver` and `policy_for`.
- `src/ship/coordination.rs` — `process_coordination_lag` delivers channel-3 messages to `PendingArcBearingRequest` (`coordination.rs:1495`)
- `src/ship_plugin.rs` — `PendingArcBearingRequest` component set when AI Helm consumes `ArcBearingRequest` (`ship_plugin.rs:69`); `operate_helm_ai` reads pending bearing and biases steering via `steer_toward`
- `src/ai/mod.rs` — `steer_toward(yaw, target_dir, deadband_rad, full_steer_rad)` pure steering helper; `PATROL_DEADBAND_RAD = 0.05`, `PATROL_FULL_STEER_RAD = π/4`.

## AI ship unification and per-kind AI plugins (PRD #520)

After `ControlSourceResolver` was established as the control-gating authority (PRD #517), PRD #520 extended it to NPC ships. Each coarse system now has (or will have) a dedicated `operate_<kind>_ai` Bevy system that runs after `AiTickLabel` and is gated on `policy_for(system_id).operate_ai`. This makes the AI/human split uniform: the same gate that prevents a human from driving a Backfill console also enables the per-kind plugin to operate it.

NPC ships carry `ShipSystemControlSources` seeded with `ControlSource::Ai` for all systems; player ships default to `ControlSource::Human` (modified by rating changes).

See [AI Ship Unification](./ai-ship-unification.md) for the full architecture.

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](../sources/prd-487-station-console-system-redesign.md)
- [PRD #517 - Consistency cleanup for the 9 coarse systems](../sources/prd-517-consistency-cleanup.md)
- [Issue #525 - SystemId naming convention](../sources/issue-525-systemid-naming.md)
- [PRD #520 - AI ship unification](../sources/prd-520-ai-ship-unification.md)
- [AI Ship Unification](./ai-ship-unification.md)
