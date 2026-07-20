# Radar Target Authority & Admission Symmetry

Status: accepted design; implementation proposed (2026-07-20 architecture review).

This document is the normative contract for two connected changes agreed in the
2026-07-20 architecture review: moving target selection into the radar systems'
blackboards, and completing full AI/human admission symmetry. Affected
architecture slices (`radar-sensors.yaml`, `protocol-targeting-and-observation.yaml`,
`coordination-blackboards.yaml`, `station-system-authority.yaml`, `weapons.yaml`)
are updated entity-by-entity as each migration PR lands; until then this document
supersedes them where they disagree.

## 1. Target authority

- Each radar System (`helm-radar`, `tactical-radar`, `sensor-radar`) owns its own
  target selection and publishes it — together with its blips — in its own
  `SystemBlackboard` variant. Radar blips leave the weapons blackboard.
- The tactical radar's selection is the **Combat Lock**. The viewscreen
  aggregator lifts ship-wide facts from the radar blackboards: Combat Lock (from
  the tactical radar) and Science Target (from the sensor radar).
- Consumers — weapons firing paths, helm pursuit, shields, comms — read the
  **frozen viewscreen blackboard**. One-tick lag at 30Hz is accepted, including
  for firing. In-flight torpedoes keep their `TorpedoTargetSnapshot` latch.
- The `WeaponsTarget` and `SensorsTarget` components are retired once their
  consumers read the viewscreen blackboard.

Preserved decisions (not re-litigated):

- `sensors-target-designation` remains a Channel-3 advisory from Sensors to
  Tactical; it advises and does not replace Tactical target authority.
- The entity-targetability contract (selectable tags, OR-filter matching) and
  the shared radar-projection-service are unchanged.

## 2. Admission symmetry

- Every Control Source emits `ControlSystem { target: SystemId, payload }`.
  `ai:` tokens resolve through `AiTokenRegistry` to the owning entity; admission
  writes into that entity's per-entity `AdmittedCommands`. No AI path mutates
  system state through private intent components.
- The console-AI decide systems (`ai_shield_focus`, `ai_power_allocation`,
  `ai_torpedo_auto_fire`, …) keep their decision logic but emit admitted
  `SystemControlPayload`s. The bespoke intent components (`ShieldArcIntents`,
  `PowerReactorIntents`, `TorpedoIntents`) and their paired integrate systems
  retire into each system's normal admitted-command application. This supersedes
  the transport shape wired by issues #692–#698, not the decision logic.
- The host page keeps its dedicated `LOCAL_CONSOLE_TOKEN` identity and its
  recorded trust boundary: host-page actions may only address the local ship and
  never constitute a session or station claim. Host-page consoles route through
  `gui/action-map.js` → `ClientMessage`; the `UiAction` enum and its Rust
  translator are deleted.
- There is no coarse `helm` system (deleted in #801; `helm` is a station id).
  The helm decision seam is the shared, stateless surface assembly consumed by
  the per-axis fine systems — it must not become a coarse helm controller.

## 3. Must not

- No system may branch on human-vs-AI downstream of admission.
- No consumer may read another ship's radar blackboard as its own.
- Cross-system target reads must not bypass the viewscreen blackboard to reach a
  radar's live selection synchronously.

## 4. Migration

Per-system PRs in this order: helm, shields, sensors, radar/target seam, repair,
captain, navigation, comms, power; then NPC-predicate/LocalShip cleanup; then the
SystemId command router. Behavioural gate: headless combat runs
(`assets/worlds/combat_test.toml`) before/after the symmetry and radar/target
PRs must produce equivalent NPC combat outcomes.
