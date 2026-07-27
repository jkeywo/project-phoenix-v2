---
title: Coarse-system migration
type: concept
tags: [stations, systems, migration, system-registry, prd-487, prd-517]
updated: 2026-07-16
---

# Coarse-system migration

The migration from per-console message dispatch to the unified system control path is **complete**. Every console registers its system kind(s), accepts `ControlSystem` dispatch, gates on `ControlSourceResolver::policy_for`, and (where applicable) emits channel-3 traffic via `CoordinationEnqueue`. Helm and Tactical went further: their coarse system ids were deleted entirely in favour of fine (per-instance / per-axis) systems, with `"helm"` / `"tactical"` surviving only as station ids. This page records the resulting id namespaces and the per-console shape.

## The three id namespaces (issue #801)

One id, one meaning. Every identifier in the station/system architecture belongs to exactly one namespace:

| Namespace | What it names | Examples |
|-----------|---------------|----------|
| **System id** | A declared `[[system]]` instance: gets a `ControlSource`, gates admission, can be damaged/repaired | `"helm-thrust"`, `"phaser-fore"`, `"sensors"` |
| **Station id** | A crew station (console). Keys console-level blackboards and channel-3 coordination routing | `"helm"`, `"tactical"`, `"science"` |
| **Wire target** | The `target` string of a `ClientMessage::ControlSystem` envelope — always a system id | `"helm-steering"`, `"tactical-radar"`, `"phaser-control"` |

`"helm"` and `"tactical"` are station ids only: no `[[system]]` block declares them, no `ControlSource` is registered for them, and no `ControlSystem` message targets them. Console-level blackboards (Helm console, Weapons console) are keyed by the station id via `helm_station_key()` / `tactical_station_key()`; the client-visible key strings (`blackboards['helm']`, `blackboards['tactical']`) are unchanged. Per-system blackboards (`"phaser-bank-*"`, `"power-reactor"`, `"helm-lateral-thrust"`) keep system-id keys.

## SystemId naming convention

Pinned by issue #525. All `SystemId` wire strings follow one of three patterns:

| Pattern | Rule | Examples |
|---------|------|---------|
| **Coarse system** | Lowercase kebab matching the system kind id | `"sensors"`, `"captain"`, `"red-alert"` |
| **Fine system** | Kind id + `-` + instance suffix | `"phaser-fore"`, `"torpedo-tube-fore-port"` |
| **Ownerless capability** | Bare capability id (lowercase kebab) | `"red-alert"`, `"viewscreen"` |

Multi-word ids always use hyphens (`-`), never underscores. The `*_SYSTEM_ID` constants in `src/ship/system_registry.rs` are the authoritative source; always use the helpers (`sensors_system_id()`, `helm_thrust_system_id()`, etc.) rather than inline string literals.

### `red_alert` vs `red-alert` quirk

The registry kind key uses `"red_alert"` (snake_case, `RED_ALERT_KIND`) for legacy reasons, while the wire `SystemId` is `"red-alert"` (kebab, `RED_ALERT_SYSTEM_ID`). All other systems have identical `*_KIND` and `*_SYSTEM_ID` values. New systems must use the same lowercase-kebab string for both.

## Per-console shape

| Console | Kind registered | `ControlSystem` dispatch | `policy_for` gating | Channel-3 via `CoordinationEnqueue` | Issue |
|---------|----------------|--------------------------|---------------------|--------------------------------------|-------|
| Captain | ✅ `captain` | ✅ | ✅ | n/a | #499 |
| Helm | ❌ deleted by #801 — per-axis fine systems only (`helm_thrust`, `helm_steering`, `helm_impulse`, `helm_boost`, `lateral_thrust`, …); `"helm"` survives as a station id | ✅ (per axis) | ✅ (per axis) | ✅ (station-key target) | #497/#801 |
| Tactical | ❌ deleted by #512/#801 — fine systems only (`phaser_bank`, `torpedo_tube`, `torpedo_magazine`, `tactical_radar`, `phaser_control`); `"tactical"` survives as a station id | ✅ (per system) | ✅ (per system) | ✅ (station-key target) | #491/#801 |
| Power | ✅ `power` | ✅ | ✅ | n/a | #500 |
| Sensors | ✅ `sensors` | ✅ | ✅ | ✅ | #498 |
| Shields | ✅ `shields` | ✅ | ✅ | ✅ (#528) | #502/#528 |
| Comms | ✅ `comms` | ✅ | ✅ | ✅ | #503 |
| Viewscreen | ✅ `viewscreen` | ✅ | ✅ | n/a | #505 |
| Repair | ✅ `repair` | ✅ (#526) | ✅ (#526) | n/a | #525/#526 |
| Navigation | ✅ `navigation` | ✅ (#527) | ✅ (#527) | n/a | #527 |

## Fine-system ids

Fine-system decomposition (e.g. `"phaser-fore"`, `"torpedo-tube-fore-port"`) shipped via issues #511–#515 and #701/#800/#801:

| Issue | Coarse system | Shipped fine kinds |
|-------|---------------|--------------------|
| #511 | Helm | `helm_joystick`, `helm_engine` (port + starboard), `helm_radar`, `helm_impulse` |
| #701/#800/#801 | Helm | `helm_thrust`, `helm_steering` (per-axis stick split), `helm_boost` (#801); the coarse `helm` `[[system]]` block is deleted from all 9 hull TOMLs |
| #512 | Tactical | `phaser_bank` (fore + aft), `torpedo_tube` (fore-port + fore-starboard + aft), `torpedo_magazine` |
| #801 | Tactical | `phaser_control` (ship-wide phaser mode + frequency); `SetTarget` re-targeted to `tactical-radar`; the coarse `tactical` id ceases to exist as a system |
| #513 | Power | `power_reactor`, `power_battery` |
| #514 | Shields | `shield_arc` (variable count, per `[[shield_arc]]` TOML block; player ship = 4 arcs fore/port/aft/starboard; NPCs = 1 omni arc) |
| #515 | Comms / Captain / Viewscreen | closed as substantially done — Captain / Red-Alert / Viewscreen shipped via PRD #487; Comms deliberately left coarse (single narrow console; splitting into inbox/transmitter/scanner deferred pending a damage-driven rationale) |

Under #512, the coarse `tactical` `[[system]]` block was **deleted** from all ship TOMLs. Under #801 the id itself stopped being a system: `SetTarget` targets `tactical-radar`, and `SetPhaserMode` / `SetPhaserFrequency` target the new `phaser-control` system (declared on every hull with phaser banks). `"tactical"` survives only as `TACTICAL_STATION_ID` — the station-id key for the Weapons console blackboard and coordination routing. The dead coarse-tactical fallbacks in `any_bank_operates_ai` / `any_blaster_bank_operates_ai` / `any_tactical_system_operates_ai` were deleted; unregistered fine ids resolve to the default-source policy.

Under #801 the coarse `helm` `[[system]]` block was likewise **deleted** from all 9 hull TOMLs. The combined `HelmInput { thrust, steering }` payload was split into `SetThrust { value }` → `helm-thrust` and `SetSteering { value }` → `helm-steering`, so a human's stick input is admitted per axis — fixing the #701 mismatch where the human's combined input was admitted on the coarse policy while the AI gated per axis. Impulse commands target `helm-impulse`; boost commands target the new `helm-boost` system. `"helm"` survives only as `HELM_STATION_ID` (Helm console blackboard key, coordination routing, and the helm station itself).

Tube-to-magazine communication uses [`InterSystemPayload::ClaimTorpedoRound { tube }`](../../src/core/messages.rs) on channel 2, mirroring the `DrainWeaponsBattery` pattern from #559. The magazine consumer refuses claims when its hull entry — declared as `[[hull.system_hull]] system_id = "torpedo-magazine"` in TOML (pre-#619 this block was spelled `[[hull.console_hull]] console = "TorpedoMagazine"`) — is at Disabled/Destroyed tier.

Under #513, the coarse `power` `[[system]]` block was **deleted** from all 6 ship TOMLs (player_ship + 5 NPC ships). Both `power_reactor` and `power_battery` live on the `power` station (single station holder). Allocation input (`SetPower` / `SetPowerGroupAllocation`) targets `power-reactor`; channel-2 battery drain (`DrainWeaponsBattery` from active phaser beams) targets `power-battery`. Both fine systems read shared state via the same per-entity `ShipPowerSystem` component (option (a) — same-ship, same-tick), which is why no inter-system messaging is required *between* reactor and battery. `POWER_SYSTEM_ID = "power"` is retained as a stable string only so the JS panel can continue to read the aggregate `blackboards['power']` entry.

**Deferred (issue #513):** the ship TOML's `[power] capacity/rates/emergency_threshold` and `[power.ai]` blocks remain ship-wide rather than being split into per-fine-system config blocks. Runtime `ShipPowerSystem` is still per-ship; a designer-facing knob split (reactor-specific capacity vs battery-specific capacity, etc.) is a future PRD.

Under #514, the coarse `shields` `[[system]]` block was **deleted** from `player_ship.toml`, along with the coarse hull entry that guarded it (spelled `[[hull.console_hull]] console = "Shields"` in the pre-#619 TOML shape; post-#619 the equivalent block would be `[[hull.system_hull]] system_id = "shields"`, but it too is gone under #514). `[[shield_arc]]` blocks in the ship TOML are the designer-authoring surface: each block declares `id`, `label`, `center_deg`, `width_deg`, and optional per-arc `max_hp` / `regen_per_sec` / `offline_duration` / `hull_max_hp` overrides; the parser auto-synthesises a matching `[[system]] kind = "shield_arc" id = "shield-arc-<id>"` entry. Player ships have a `shields` station so synthesised arcs are player-controlled (`ai_only = false`); NPC ships have no `shields` station so their single omni arc is `ai_only = true` and ownerless. `SHIELDS_SYSTEM_ID = "shields"` is retained as a stable string only for the aggregate `blackboards['shields']` entry (the JS panel's primary read); all control-input and coordination lookups target per-arc `shield-arc-<id>` SystemIds. `SetShieldArcFocus { focused: bool }` replaces the old `SetShieldFocus { facing: Option<ViewDirection> }` payload — each arc button targets its own SystemId. Per-arc hull HP is tracked in a new `EntityShipArcHull` component (wrapping pure `ShipArcHull`); damage is distributed proportionally to overall hull damage, and `sync_console_damage_tiers` iterates the arc-hull map to flip `shield-arc-<id>` in/out of `offline_systems` on Disabled/Destroyed tiers.

## Key files

- `src/ship/system_registry.rs` — All `*_SYSTEM_ID` and `*_KIND` constants, `*_system_id()` helpers, and the station-key helpers (`helm_station_key()`, `tactical_station_key()`). (The pre-#520 `*_AI_CONTROLLER` named-controller constants were dead weight and have been deleted.)
- `src/ship/control_source.rs` — `ControlSourceResolver` and `policy_for`.
- `src/ship/coordination.rs` — `process_coordination_lag` delivers channel-3 messages to `PendingArcBearingRequest` (`coordination.rs:1495`)
- `src/ship/components.rs` — `PendingArcBearingRequest` component set when AI Helm consumes `ArcBearingRequest`; `ai_helm_steering` reads the pending bearing and biases steering via `steer_toward` (see [AI Helm Decomposition](./ai-helm-decomposition.md))
- `src/ai/mod.rs` — `steer_toward(yaw, target_dir, deadband_rad, full_steer_rad)` pure steering helper; `PATROL_DEADBAND_RAD = 0.05`, `PATROL_FULL_STEER_RAD = π/4`.

## AI ship unification and per-kind AI plugins (PRD #520)

After `ControlSourceResolver` was established as the control-gating authority (PRD #517), PRD #520 extended it to NPC ships. Each system has a dedicated AI Bevy system (helm: the four per-axis systems — see [AI Helm Decomposition](./ai-helm-decomposition.md)) that runs after `AiTickLabel` and is gated on `policy_for(system_id).operate_ai`. This makes the AI/human split uniform: the same gate that prevents a human from driving a Backfill console also enables the per-kind system to operate it.

Every ship — player and NPC alike — is seeded by `ship::rating::seed_boot_ratings` (`src/ship/rating.rs`, issue #871): each station applies its boot rating, then every ownerless `ai_only` system is set to `ControlSource::Ai`. A ship with nobody connected boots every station on the implicit `Backfill` rating, which automates all systems that station owns, so an unmanned NPC ends up `ControlSource::Ai` throughout. A manned station boots on its lobby-chosen rating instead, leaving its non-automated systems `Human` (modified thereafter by rating changes). NPC hulls are ordinary stationed ships with nobody in the seats — before #871 they declared no stations at all and the spawner set every declared system to `Ai` by fiat.

See [AI Ship Unification](./ai-ship-unification.md) for the full architecture.

## Cross-references

- PRD #487 - Station / Console / System architecture redesign
- PRD #517 - Consistency cleanup for the 9 coarse systems
- Issue #525 - SystemId naming convention
- PRD #520 - AI ship unification
- [AI Ship Unification](./ai-ship-unification.md)
