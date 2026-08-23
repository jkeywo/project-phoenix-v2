//! Structured debug observability (PRD #1144).
//!
//! One pipeline for making the running simulation inspectable: every debug
//! surface is a `serde` struct ([`payload`]) produced by a read-only `collect`
//! projection off authoritative state, encoded to JSON in
//! [`crate::core::codec`], and consumed identically by the host-page debug dock,
//! the headless report, and (later) the GM console Live Inspector. This is the
//! home the four legacy text overlays in [`crate::debug_overlay`] migrate onto,
//! and the schema the later PRD #1144 slices extend — read [`payload`] for the
//! conventions they follow.
//!
//! This first slice ([`station_activity`], issue #1145) drives station activity
//! end-to-end: an always-on tracker at command admission, the shared payload
//! schema, a debug flag + WASM bridge getter, and the dock chart — establishing
//! the schema and transport pattern the rest of the pipeline reuses.
//!
//! # Determinism and demo builds
//!
//! Capture is a read-only projection off authoritative state, so enabling it
//! never moves the #894 digest. The counters are always-on on every target; only
//! the JSON publish is flag-gated. Demo builds keep cfg-stripping the debug
//! *surface* (the WASM toggle export and its client route) exactly as the legacy
//! overlays do; the counters, being invisible and inert to the sim, stay.

pub mod ai_state;
pub mod console_latency;
pub mod damage;
pub mod entities;
pub mod inspector;
pub mod modifiers;
pub mod payload;
pub mod scenario;
pub mod station_activity;

use bevy::prelude::*;

pub use ai_state::{AiDoctrineCapture, DebugAiDoctrineEnabled};
pub use console_latency::{
    ConsoleLatencyCapture, ConsoleLatencyTracker, DebugConsoleLatencyEnabled, TickAdmissionInstant,
};
pub use payload::{
    ActionLatencyEntry, ActivitySource, AiStatePayload, ConsoleLatencyPayload,
    ConsoleLatencySample, DamageDebugPayload, DamageEntry, DoctrineCandidate, DoctrineChoice,
    EntityBehaviorEntry, EntityBehaviorPayload, EntityInspectorPayload, FloatContribution,
    FloatModifierEntry, HostBlockedView, HostMemoryEntry, HostPolicyView, HostTransitionView,
    InspectorEntity, InspectorHullEntry, InspectorPlayer, InspectorShieldFacing, IntContribution,
    IntModifierEntry, LatencySummary, LatencySurface, ModifierDebugPayload, ModifierFlagEntry,
    PredicateValue, ScenarioCommitment, ScenarioDeadline, ScenarioDelayedAction,
    ScenarioDossierEntry, ScenarioFlag, ScenarioObjective, ScenarioStatePayload, ScenarioTrigger,
    ShipDoctrine, StationActivityBucket, StationActivityEntry, StationActivityPayload, TriggerFire,
    DEBUG_SCHEMA_VERSION,
};
pub use scenario::{DebugScenarioStateEnabled, ScenarioStateCapture, TriggerFireRecorder};
pub use station_activity::{
    DebugStationActivityEnabled, StationActivityCapture, StationActivityTracker,
};

pub use damage::DamageDebugCapture;
pub use entities::EntityBehaviorCapture;
pub use inspector::EntityInspectorCapture;
pub use modifiers::ModifierDebugCapture;

/// Wires the always-on debug counters and their flag-gated JSON publish into the
/// simulation app on every target (issue #1145).
///
/// Added by `server_app::add_simulation_plugins_with`, so the browser host, the
/// headless runner and the native host all get the same counters and the same
/// capture path. The three resources it owns are declared
/// `StateClass::Presentation` (read-only diagnostic surfaces nothing in the
/// fixed tick reads), which is what keeps the authoritative-state enumeration
/// guard green without folding them into the digest.
pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<StationActivityTracker>()
            .init_resource::<DebugStationActivityEnabled>()
            .init_resource::<StationActivityCapture>()
            .init_resource::<DebugAiDoctrineEnabled>()
            .init_resource::<AiDoctrineCapture>()
            .init_resource::<DebugScenarioStateEnabled>()
            .init_resource::<ScenarioStateCapture>()
            .init_resource::<TriggerFireRecorder>();

        // Console input-to-feedback latency (issue #1169). The tracker holds
        // both halves of the picture: the host's own admission→broadcast window,
        // and the durations connected clients measure on their own clocks and
        // report up. All four are wall-clock-derived, so all four are
        // `Presentation` and none is ever folded.
        app.init_resource::<DebugConsoleLatencyEnabled>()
            .init_resource::<ConsoleLatencyTracker>()
            .init_resource::<ConsoleLatencyCapture>()
            .init_resource::<TickAdmissionInstant>();

        // The four legacy-overlay capture sinks (issue #1150). Each is the
        // target-agnostic JSON home for one migrated surface, the analogue of
        // `StationActivityCapture`. `None` until the flag-gated publish first
        // runs; on the browser host the publish ALSO feeds a WASM bridge
        // thread-local the dock reads.
        app.init_resource::<ModifierDebugCapture>()
            .init_resource::<DamageDebugCapture>()
            .init_resource::<EntityBehaviorCapture>()
            .init_resource::<EntityInspectorCapture>();

        // The observability surfaces are read-only projections nothing in the
        // fixed tick reads back, so they are digest EXCLUSIONS, not authoritative
        // state — see `crate::authoritative` and the enumeration guard.
        app.declare_state::<StationActivityTracker>(
            StateClass::Presentation,
            "debug-station-activity",
        )
        .declare_state::<DebugStationActivityEnabled>(
            StateClass::Presentation,
            "debug-station-activity",
        )
        .declare_state::<StationActivityCapture>(StateClass::Presentation, "debug-station-activity")
        .declare_state::<DebugAiDoctrineEnabled>(StateClass::Presentation, "debug-ai-doctrine")
        .declare_state::<AiDoctrineCapture>(StateClass::Presentation, "debug-ai-doctrine")
        .declare_state::<DebugScenarioStateEnabled>(
            StateClass::Presentation,
            "debug-scenario-state",
        )
        .declare_state::<ScenarioStateCapture>(StateClass::Presentation, "debug-scenario-state")
        // The trigger-fire recorder (issue #1151): a read-only projection nothing
        // in the fixed tick reads back, so it is a digest EXCLUSION exactly like
        // the scenario capture it accompanies.
        .declare_state::<TriggerFireRecorder>(StateClass::Presentation, "debug-scenario-state")
        // Console latency (issue #1169). The strongest case on this pipeline for
        // the `Presentation` class: these hold WALL-CLOCK-derived numbers, which
        // are non-deterministic by construction and could never be folded
        // without breaking the #894 digest contract outright.
        .declare_state::<DebugConsoleLatencyEnabled>(
            StateClass::Presentation,
            "debug-console-latency",
        )
        .declare_state::<ConsoleLatencyTracker>(StateClass::Presentation, "debug-console-latency")
        .declare_state::<ConsoleLatencyCapture>(StateClass::Presentation, "debug-console-latency")
        .declare_state::<TickAdmissionInstant>(StateClass::Presentation, "debug-console-latency")
        .declare_state::<ModifierDebugCapture>(StateClass::Presentation, "debug-legacy-overlays")
        .declare_state::<DamageDebugCapture>(StateClass::Presentation, "debug-legacy-overlays")
        .declare_state::<EntityBehaviorCapture>(StateClass::Presentation, "debug-legacy-overlays")
        .declare_state::<EntityInspectorCapture>(StateClass::Presentation, "debug-legacy-overlays");

        // Counters: always-on, after the whole tick's admission has run (the
        // same window the unrouted-command lint observes), gated only on there
        // being a run in progress.
        app.add_systems(
            FixedUpdate,
            station_activity::record_station_activity
                .after(crate::sim_sets::SimSet::Broadcast)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress)),
        );

        // Publish: flag-gated, after the counters have taken this tick's tally.
        app.add_systems(
            FixedUpdate,
            station_activity::publish_station_activity
                .after(station_activity::record_station_activity)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugStationActivityEnabled>| flag.0),
        );

        // Console input-to-feedback latency (issue #1169). THREE flag-gated
        // systems, and the gate is on measurement rather than only on the
        // publish: `Instant::now()` is a clock read the simulation must not take
        // unasked (PRD #1144's "no observable overhead when off"), and skipping
        // the pair of them wholesale is what makes the determinism A/B in
        // `tests/console_latency.rs` a real experiment rather than a tautology.
        //
        // The stamp is pinned INSIDE the tick it measures — after the tick's
        // commands exist, strictly before the sim consumes them — so the window
        // it opens is the simulation's own service window and nothing else. The
        // recorder closes it at the same end-of-tick window
        // `record_station_activity` uses, which is the only place
        // `AdmittedCommands` holds both the network-admitted and the AI-emitted
        // halves.
        app.add_systems(
            FixedUpdate,
            console_latency::stamp_admission_instant
                .after(crate::command_admission::AdmissionSet)
                .before(crate::sim_sets::SimSet::Input)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugConsoleLatencyEnabled>| flag.0),
        );
        app.add_systems(
            FixedUpdate,
            console_latency::record_admit_to_broadcast
                .after(crate::sim_sets::SimSet::Broadcast)
                // Explicit, and not merely implied by the set ordering above: in
                // a fixture where `AdmissionSet` and the `SimSet` chain have no
                // members, "after admission, before Input" constrains nothing,
                // and the recorder could run before the stamp it reads. A direct
                // edge between the two systems holds in every app.
                .after(console_latency::stamp_admission_instant)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugConsoleLatencyEnabled>| flag.0),
        );
        app.add_systems(
            FixedUpdate,
            console_latency::publish_console_latency
                .after(console_latency::record_admit_to_broadcast)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugConsoleLatencyEnabled>| flag.0),
        );

        // AI doctrine-pool projection (issue #1149): flag-gated, after
        // `SimSet::Broadcast` so the tick's viewscreen pool (written in
        // `SimSet::PublishAggregate`) is final. Read-only, so it never moves the
        // digest whether the flag is on or off.
        app.add_systems(
            FixedUpdate,
            ai_state::publish_ai_doctrine
                .after(crate::sim_sets::SimSet::Broadcast)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugAiDoctrineEnabled>| flag.0),
        );

        // Scenario state (issue #1148): a read-only projection off the world
        // content runtime, with no counters to feed — the whole surface is this
        // flag-gated publish. Ordered after `SimSet::Broadcast` so it reads the
        // trigger pipeline's end-of-tick state, the same window the
        // station-activity tap uses.
        //
        // Trigger-fire recording (issue #1151) runs in the same flag-gated window
        // BEFORE the publish, so the fire rings the publish folds in are current.
        // Both are read-only projections into `Presentation`-class resources, so
        // gating them together gives a crisp determinism A/B: the whole
        // scenario-debug surface on vs off leaves the seeded digest identical
        // (`tests/scenario_state.rs`).
        app.add_systems(
            FixedUpdate,
            scenario::record_trigger_fires
                .after(crate::sim_sets::SimSet::Broadcast)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugScenarioStateEnabled>| flag.0),
        );
        app.add_systems(
            FixedUpdate,
            scenario::publish_scenario_state
                .after(scenario::record_trigger_fires)
                .run_if(in_state(crate::core::messages::GamePhase::InProgress))
                .run_if(|flag: Res<DebugScenarioStateEnabled>| flag.0),
        );

        // The four migrated legacy overlays (issue #1150). Each is a read-only
        // projection published in `PostUpdate` — the schedule the retired text
        // overlays ran in, after `Update`'s render sync has settled the
        // `Transform`s the entity/inspector surfaces read. They run on every
        // target so headless and the determinism guard get the same capture path.
        //
        // The flag-gate lives INSIDE each system (each takes its
        // `debug_overlay` enabled flag as `Option<Res<..>>` and returns early
        // when it is absent or off), not on a `run_if`: those flags are only
        // inserted where `DebugOverlayPlugin` ran (the browser host), while a
        // headless run merely declares them — so a `run_if` fetching the flag as
        // `Res` could touch a resource that does not exist. The projections cost
        // nothing when the flag is off, and the determinism guard inserts the
        // flags to drive the publish deliberately.
        app.add_systems(PostUpdate, modifiers::publish_modifier_debug);
        app.add_systems(PostUpdate, damage::publish_damage_debug);
        app.add_systems(PostUpdate, entities::publish_entity_behavior_debug);
        app.add_systems(PostUpdate, inspector::publish_entity_inspector_debug);
    }
}
