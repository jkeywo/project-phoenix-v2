# Radar Target Authority & Admission Symmetry

Status: accepted design; **implemented** by issues #824–#833 (2026-07-21), with the
one deferred exception recorded in §2. Superseded statements have been corrected
against shipped code — this document describes what the code does, not what was
planned.

This document is the normative contract for two connected changes agreed in the
2026-07-20 architecture review: moving target selection into the radar systems'
blackboards, and completing full AI/human admission symmetry. Affected
architecture slices (`radar-sensors.yaml`, `protocol-targeting-and-observation.yaml`,
`coordination-blackboards.yaml`, `station-system-authority.yaml`, `weapons.yaml`)
were updated entity-by-entity as each migration PR landed; this document
supersedes them where they disagree.

## 1. Target authority

- A radar System that owns a target selection publishes it — together with any
  blips it owns — in its own `SystemBlackboard` variant. Radar blips leave the
  weapons blackboard.
  - **`tactical-radar`** → `SystemBlackboard::TacticalRadar`, carrying the
    selection (the Combat Lock) *and* the blips/regions that moved off
    `WeaponsBlackboard`.
  - **`sensor-radar`** → `SystemBlackboard::SensorRadar`, carrying the selection
    (the Science Target) only. It has **no blips**: sensor-radar blips are
    derived client-side in `gui/`, per the pure-JS client direction, so there is
    nothing server-side to publish.
  - **`helm-radar`** has **no variant and no selection**. There is no helm
    target-select path anywhere in the codebase; the helm radar is a
    range-limited *display* driven by `HelmBlackboard.radar_range`, and its blips
    are likewise client-derived. A variant would carry nothing.
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

- Every Control Source that emits through the `ControlSystem` channel emits
  `ControlSystem { target: SystemId, payload }`. `ai:` tokens resolve through
  `AiTokenRegistry` to the owning entity; admission writes into that entity's
  per-entity `AdmittedCommands`. No AI path on that channel mutates system state
  through private intent components.
- The console-AI decide systems (`ai_shield_focus`, `ai_power_allocation`,
  `operate_sensors_ai`, `operate_repair_ai`, `operate_navigation_ai`,
  `operate_captain_ai`, the four per-axis helm systems, …) keep their decision
  logic but emit admitted `SystemControlPayload`s. `ShieldArcIntents` (#826) and
  `PowerReactorIntents` (#831), and their paired `integrate_*` systems, are
  retired into each system's normal admitted-command application. This supersedes
  the transport shape wired by issues #692–#698, not the decision logic.
- **Deferred exception — the weapons fire/load channel.** `FirePhaser`,
  `FireTorpedo`, `LoadTube`, and `UnloadTube` remain top-level `ClientMessage`
  variants carrying no `SystemId`, authorized inline (`tactical_authorized`)
  rather than through admission. Because they are not `ControlSystem` messages,
  `TorpedoIntents` and `PhaserIntents` — with `integrate_weapons_state` as their
  paired applier — **survive as the AI transport for those weapons**. This is a
  known gap, not an oversight: the intent components cannot retire until the
  fire/load messages become admitted `ControlSystem` payloads, so **the two
  halves must land together**. Recorded in
  `protocol-targeting-and-observation.yaml` and in `command_admission/router.rs`'s
  module header. Until then, §2's "no private intent components" rule holds for
  every system *except* weapons fire/load.
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

**Landed 2026-07-21, in exactly that order:** #824 helm → #826 shields → #828
sensors → #829 radar/target seam → #830 repair/captain/navigation → #831
comms/power → #832 NPC-predicate + `LocalShip` cleanup → #833 SystemId router
(a load-time consumer registry + unrouted-command lint, deliberately **not** a
runtime dispatcher — consumers keep their own `SimSet` scheduling). The
behavioural gate held at every step: the 60s `combat_test.toml` run stayed
**bit-identical** to the pre-migration baseline on every simulation field, not
merely equivalent. The one item not delivered is the weapons fire/load channel
recorded as a deferred exception in §2.
