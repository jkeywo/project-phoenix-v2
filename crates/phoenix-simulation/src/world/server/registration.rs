//! Registration.
use super::*;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        crate::entities::craft::register(app);
        app.declare_state::<ObjectiveManagerRes>(StateClass::Folded, "objective-runtime-state");
        app.declare_state::<crate::recipients::RecipientDiagnostics>(
            StateClass::Presentation,
            "recipient-authoring-diagnostics",
        )
        .init_resource::<crate::recipients::RecipientDiagnostics>();
        app.declare_state::<ObjectiveInstanceManagerRes>(
            StateClass::Folded,
            "objective-instance-runtime-state",
        );
        app.declare_state::<crate::ship_slots::AuthoredShipSlotId>(
            StateClass::Folded,
            "objective-instance-state",
        );
        app.declare_state::<crate::ship_slots::FrozenShipSlots>(
            StateClass::Folded,
            "launch-frozen-ship-slots",
        );
        app.declare_state::<ScriptActivationGate>(
            StateClass::Derived,
            "scenario-script-loader-validation",
        );
        // The comms half of the pre-#816 WorldPlugin lives in
        // `CommsWorldPlugin`. Added here so every app that installs the
        // world also gets comms, and so the cross-plugin ordering
        // constraints (`init_comms_runtime` after `init_world_runtime`;
        // `open_scripted_comms_threads` between `tick_script_callbacks` and
        // `tick_delayed_actions`; `broadcast_objective_summary` after
        // `broadcast_comms_state`) all resolve against systems guaranteed to be
        // registered.
        // Infrastructure condition (issue #1025) is added here for the same
        // reason comms is: it writes `WorldContentRuntime`'s flag store and
        // world-event queue, so it has no meaning in an app that has no world.
        // External operations (issue #1026) join for the same reason again: a
        // script start arrives on `WorldContentRuntime`'s queue and a completion
        // pays into the infrastructure queue beside it, so the plugin is
        // meaningless in an app with no world — and its tick is explicitly
        // ordered before `InfrastructurePlugin`'s, which needs both registered.
        // Civilian traffic (issue #1028) is added here for the same reason:
        // routes are world data, the order queue is a field on
        // `WorldContentRuntime`, and a dock target is resolved through its name
        // table — none of which exist in an app with no world.
        app.add_plugins(crate::comms::CommsWorldPlugin)
            .add_plugins(crate::infrastructure::InfrastructurePlugin)
            // The tractor beam (issue #1156). Added here alongside its sibling
            // because a coupled target is moved through the same after-integration
            // `SimSet::Modifiers` window the rig uses.
            .add_plugins(crate::tractor::server::TractorPlugin)
            // Helm docking (issue #1159) stands beside the tractor for the same
            // reason: the own ship is flown onto its mate through the same
            // after-integration `SimSet::Modifiers` window the tractor rig uses,
            // and it is ordered after that rig so a hull that is both a docking
            // ship and a tractor target has a deterministic last writer.
            .add_plugins(crate::dock::server::DockPlugin)
            // The transfer umbilical (issue #1160) stands beside the dock it gates
            // on: its flow tick runs after the dock tick (so it reads the fresh
            // docked state) and before the infrastructure tick (so the capacity it
            // queues moves the same tick), the ordering the tractor's arrest keeps.
            .add_plugins(crate::umbilical::server::UmbilicalPlugin)
            // Security teams (issue #1346) stand beside them for the same reason:
            // the targets a team crosses to are world furniture spawned from world
            // files, and the consequence a completed action raises is a flag on
            // this plugin's own `WorldContentRuntime` store, so a scenario trigger
            // fires the tick the work lands.
            .add_plugins(crate::security::SecurityPlugin)
            // The rescue transporter (issue #1348) stands beside them: what it
            // recovers is the civilians an authored contact carries, its discovery
            // reads the `scan.<id>.taken` flag on this plugin's own
            // `WorldContentRuntime` store, and the completion/casualty facts it
            // raises are flags on that same store, so an authored beat fires the
            // tick the rescue lands or a carrier is lost.
            .add_plugins(crate::transporter::server::TransporterPlugin)
            .add_plugins(crate::civilian::CivilianPlugin)
            // Dossiers (issue #1030) join them for the same reason: the
            // commitments a fact sheet lists are a field on
            // `WorldContentRuntime`, the comms standing it reports is
            // `CommsRuntime`'s, and every subject on its roster came out of a
            // world file. The plugin registers a publisher and nothing else.
            .add_plugins(crate::dossier::DossierPlugin)
            // The science scan (issue #1032) for the same reason again: what it
            // reads is an authored structure's condition track, so it is
            // meaningless in an app with no world — and its tick is explicitly
            // ordered AFTER `InfrastructurePlugin`'s, so a scan taken on the
            // tick a repair lands reads the repaired number.
            .add_plugins(crate::science::SciencePlugin)
            // Debris hazards (issue #1347) stand beside the science plugin, and
            // are ordered after it inside `SimSet::Modifiers`: what promotes a
            // drifting rock to a confirmed threat is a scan, so the tick that
            // latches an assessment has to run after the tick that takes one.
            .add_plugins(crate::debris::DebrisPlugin)
            // Controlled demolition (issue #1350) stands beside Security, whose
            // command target it borrows and whose team states it reads: the
            // obstruction it clears is world furniture spawned from a world file,
            // and the four outcomes it decides raise flags on this plugin's own
            // `WorldContentRuntime` store, so a scenario trigger fires the tick the
            // charges go off.
            .add_plugins(crate::demolition::DemolitionPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<ObjectiveInstanceManagerRes>()
            .init_resource::<PendingScenarioLoad>()
            .init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<WorldEventBuffer>();
        crate::world::materialization::register(app, Startup);
        app.add_systems(
            FixedUpdate,
            broadcast_objective_summary
                .in_set(crate::sim_sets::FixedStep::BroadcastObjectiveSummary)
                .in_set(crate::sim_sets::SimSet::Broadcast)
                .after(crate::comms::server::broadcast_comms_state)
                .before(
                    crate::core::broadcast::broadcaster::dispatch::<crate::core::broadcast::Sim>,
                ),
        )
        // The comms half of the Physics set is registered by
        // `CommsWorldPlugin`, ordered against these systems from that side.
        // It was a four-system `.chain()` (#718/#719) until issue #985
        // deleted `tick_pending_follow_ups` and `inject_comms_templates`;
        // `open_scripted_comms_threads` is what remains, and it sits after
        // the callback drain rather than around the event collector.
        // The mission clock (#960). `SimSet::Physics` is gated on
        // `GamePhase::InProgress`, so the first run of
        // `anchor_mission_clock` is the first simulation tick of the
        // mission; `arm_mission_clock` re-opens it for a second round.
        .add_systems(OnEnter(GamePhase::InProgress), arm_mission_clock)
        .add_systems(
            FixedUpdate,
            (
                anchor_mission_clock.in_set(crate::sim_sets::FixedStep::AnchorMissionClock),
                // Immediately after the anchor and before anything reads the
                // table: a `[[deadline]]` is due N seconds into the MISSION,
                // so it is keyed off the same first-InProgress-tick moment
                // (issue #1024). Runs its body exactly once per mission.
                arm_mission_deadlines.in_set(crate::sim_sets::FixedStep::ArmMissionDeadlines),
                // Beside the deadline arm, and for its reason: a
                // `[[workforce]]`'s authored strike status is the situation
                // the crew ARRIVE INTO, so it must be true before the first
                // handler runs and before any operation is offered
                // (issue #1035). Runs its body exactly once per mission.
                arm_mission_workforces.in_set(crate::sim_sets::FixedStep::ArmMissionWorkforces),
                collect_world_events.in_set(crate::sim_sets::FixedStep::CollectWorldEvents),
                tick_trigger_pipeline.in_set(crate::sim_sets::FixedStep::TickTriggerPipeline),
            )
                .chain()
                .in_set(crate::sim_sets::SimSet::Physics),
        )
        // Cursor advancement is a `Modifiers` evaluator: `Physics` has
        // finished moving every ship by then, so waypoint arrival is
        // judged against this tick's final positions. It emits
        // `AiWaypointReached`, which `collect_world_events` turns into a
        // `WorldEvent::WaypointReached` on the next tick (the same
        // one-tick event bridge `AiEntityAttacked` already uses).
        .add_systems(
            FixedUpdate,
            crate::ai::server::advance_objective_cursors
                .in_set(crate::sim_sets::FixedStep::AdvanceObjectiveCursors)
                .in_set(crate::sim_sets::SimSet::Modifiers),
        )
        .add_systems(
            FixedUpdate,
            crate::objective_instances::reconcile_memberships
                .in_set(crate::sim_sets::SimSet::Membership),
        )
        // The scripted-callback drain (issue #984, Rhai M6 phase 2b):
        // `after(n, |ctx| …)` callbacks that scripted handlers scheduled are
        // drained here once due. Ordered AFTER `tick_trigger_pipeline` (so it
        // shares that system's per-tick budget reset, and sees this tick's
        // freshly-scheduled callbacks) and BEFORE `tick_delayed_actions` (so a
        // callback's own `in_seconds` effect reaches the delayed queue in the
        // same tick a trigger's would). A no-op for every script-free world:
        // no `WorldScriptRuntime` → early return before any `DerefMut`.
        .add_systems(
            FixedUpdate,
            tick_script_callbacks
                .in_set(crate::sim_sets::FixedStep::TickScriptCallbacks)
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(tick_trigger_pipeline)
                .before(tick_delayed_actions),
        )
        .add_systems(
            FixedUpdate,
            tick_delayed_actions
                .in_set(crate::sim_sets::FixedStep::TickDelayedActions)
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(tick_trigger_pipeline),
        )
        .add_systems(
            FixedUpdate,
            apply_pending_scenario_loads
                .in_set(crate::sim_sets::FixedStep::ApplyPendingScenarioLoads)
                .in_set(crate::sim_sets::SimSet::Physics),
        )
        .add_systems(
            FixedUpdate,
            apply_world_layer_changes
                .in_set(crate::sim_sets::FixedStep::ApplyWorldLayerChanges)
                .in_set(crate::sim_sets::SimSet::Physics)
                .before(collect_world_events)
                .before(tick_script_callbacks),
        )
        .add_observer(handle_region_entered_event)
        .add_observer(handle_region_exited_event);
    }
}
