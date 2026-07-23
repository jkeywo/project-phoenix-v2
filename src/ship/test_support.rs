//! Shared test fixtures for the ship-module system tests, extracted from the
//! old `ship_plugin.rs` `mod tests` during the #820 mechanical split. This is
//! the one sanctioned non-pure-move element of that split: these helpers were
//! shared by test groups that now live in different modules.
#![allow(dead_code)]

use bevy::prelude::*;

use crate::control_source::ControlSource;
use crate::lobby::{InboundMessage, LobbyPlugin};
use crate::messages::ClientMessage;
use crate::modifiers::ShipModifiers;
use crate::ship::components::{
    ActiveStationRatings, CoordinationQueue, HelmWaypointClearance, LastHelmInput,
    ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::helm::{SteeringInput, ThrustInput};
use crate::ship_plugin::ShipPlugin;
use crate::ship_state::ShipPhysics;
use crate::simulation::{LocalShip, Ship, ShipBoost, ShipImpulse};

pub fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        // Mirror the production SimSet chain (server_app) so cross-set ordering
        // holds: admission (`.before(SimSet::Input)`) → Input → Physics → …
        // Issue #830 moved `handle_navigation_waypoint` / `handle_dispatch_repair_team`
        // into Physics `.after(operate_*_ai)`, which only runs after admission
        // when Input precedes Physics — this chain is what guarantees it.
        .configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(ShipPlugin);
    let hull_config = &[
        (crate::messages::SystemId("helm".into()), 25.0_f32),
        (crate::messages::SystemId("tactical".into()), 25.0),
        (crate::messages::SystemId("power".into()), 25.0),
        (crate::messages::SystemId("shields".into()), 25.0),
    ];
    let ship = app
        .world_mut()
        .spawn((
            Ship,
            LocalShip,
            Transform::default(),
            ShipPhysics::default(),
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
            ActiveStationRatings::default(),
            CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(
                hull_config,
            )),
            LastHelmInput::default(),
            crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ))
        .id();
    app.world_mut().entity_mut(ship).insert((
        ShipModifiers::new(),
        ShipBoost::default(),
        crate::ai_plugin::AiHighFidelity,
        crate::console_ai_plugin::ShipFrequencyHintState::default(),
        crate::ship::power::ShipPowerAiState::default(),
    ));
    app.world_mut().entity_mut(ship).insert((
        crate::ship::helm::ThrustInput::default(),
        crate::ship::helm::SteeringInput::default(),
        crate::ship::helm::LateralThrustInput::default(),
        crate::ship::helm::VerticalThrustInput::default(),
        crate::ship::helm::ImpulseCommand::default(),
        crate::ship::helm::BoostCommand::default(),
        // The console-owned surfaces the AI helm derives its goals from
        // (issue #702). Production spawns all four on every ship; see
        // `HelmAiSurfaces`.
        crate::weapons_plugin::TacticalRadarSelection::default(),
        crate::navigation_plugin::NavigationWaypoint::default(),
        HelmWaypointClearance::default(),
        crate::ai_plugin::ObjectiveCursors::default(),
    ));
    app
}

pub fn get_last_helm_input(app: &mut App) -> LastHelmInput {
    app.world_mut()
        .query_filtered::<&LastHelmInput, With<LocalShip>>()
        .single(app.world())
        .copied()
        .unwrap_or_default()
}

pub fn set_last_helm_input(app: &mut App, val: LastHelmInput) {
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(ship).insert(val);
}

pub fn find_ship_entity(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .expect("LocalShip entity must exist")
}

pub fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

pub fn tick(app: &mut App) {
    app.update();
}

pub fn tick_twice(app: &mut App) {
    tick(app);
    tick(app);
}

pub fn start_game_with_helm_and_science(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "helm",
        ClientMessage::Identify {
            token: "helm".into(),
            name: "Hikaru".into(),
        },
    );
    tick(app);
    push(
        app,
        "helm",
        ClientMessage::SelectStation {
            station: "Helm".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "helm", ClientMessage::SetReady { ready: true });
    tick(app);
}

/// Put the whole helm — the coarse `helm` system and all four per-axis
/// systems — on `source`.
///
/// Before #704 this set the coarse system alone, which was enough: the
/// `operate_helm_ai` monolith gated on the coarse policy and drove every
/// axis whose own system was not AI, so "coarse = Ai" *was* "the helm is on
/// AI". #704 deleted the monolith and with it the coarse fallback, so the
/// coarse system alone now drives nothing at all and a fixture that set only
/// it would assert against a ship no system is flying — a vacuous pass.
///
/// Setting all five together is the faithful successor because it is what
/// the shipped hulls actually do: every one of the nine declares all four
/// axes with the same owner as the coarse `helm` (thrust/steering since
/// #800, impulse/lateral since #704), so an unmanned station backfills all
/// five to AI and a manned one leaves all five on the human. They move
/// together in content; they move together here.
///
/// Tests that need the axes to diverge from the coarse system — the
/// per-axis gate and stand-down tests — call `set_fine_control_source`
/// afterwards to override individual axes.
pub fn set_helm_control_source(app: &mut App, source: ControlSource) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
    for mut cs in q.iter_mut(app.world_mut()) {
        cs.0.set(crate::system_registry::helm_thrust_system_id(), source);
        cs.0.set(crate::system_registry::helm_steering_system_id(), source);
        cs.0.set(crate::system_registry::helm_impulse_system_id(), source);
        cs.0.set(crate::system_registry::helm_boost_system_id(), source);
        cs.0.set(crate::system_registry::lateral_thrust_system_id(), source);
    }
}

pub fn get_ship_physics(app: &mut App) -> ShipPhysics {
    let mut q = app.world_mut().query_filtered::<&ShipPhysics, With<Ship>>();
    *q.single(app.world())
        .expect("expected Ship entity with ShipPhysics")
}

// Test helper for directly seeding ship physics state — the avoidance
// tests use it to give the ship a forward speed, which the projection and
// the `AVOIDANCE_MIN_SPEED` gate both depend on.
pub fn set_ship_physics(app: &mut App, physics: ShipPhysics) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipPhysics, With<Ship>>();
    let mut p = q
        .single_mut(app.world_mut())
        .expect("expected Ship with ShipPhysics");
    *p = physics;
}

pub fn get_ship_control_sources(app: &mut App) -> ShipSystemControlSources {
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemControlSources, With<Ship>>();
    q.single(app.world())
        .expect("expected Ship entity with ShipSystemControlSources")
        .clone()
}

pub fn get_ship_active_ratings(app: &mut App) -> ActiveStationRatings {
    let mut q = app
        .world_mut()
        .query_filtered::<&ActiveStationRatings, With<Ship>>();
    q.single(app.world())
        .expect("expected Ship entity with ActiveStationRatings")
        .clone()
}

// ── Helm system control-source tests ───────────────────────────────────

pub fn get_ship_impulse(app: &mut App) -> crate::impulse::ImpulseState {
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipImpulse, With<LocalShip>>();
    q.single(app.world())
        .expect("expected LocalShip entity with ShipImpulse")
        .0
}

pub fn set_ship_impulse(app: &mut App, state: crate::impulse::ImpulseState) {
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipImpulse>()
        .unwrap()
        .0 = state;
}

pub fn reach_scored_objective(anchor: &str, score: f32) -> crate::messages::ScoredObjective {
    crate::messages::ScoredObjective {
        id: format!("reach-{anchor}"),
        score,
        directive: crate::messages::AiDirective::Reach {
            anchor: anchor.into(),
        },
        source: crate::messages::ObjectiveSource::Mission,
        relevance: vec![crate::messages::SystemAffinity::Helm],
        snapshot: crate::messages::ObjectiveSnapshot {
            id: format!("reach-{anchor}"),
            text: format!("Reach {anchor}"),
            mandatory: true,
            status: crate::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::messages::ObjectiveSource::Mission,
        },
    }
}

pub fn retreat_scored_objective(anchor: &str, score: f32) -> crate::messages::ScoredObjective {
    crate::messages::ScoredObjective {
        id: format!("retreat-{anchor}"),
        score,
        directive: crate::messages::AiDirective::Retreat {
            anchor: anchor.into(),
        },
        source: crate::messages::ObjectiveSource::Mission,
        relevance: vec![crate::messages::SystemAffinity::Helm],
        snapshot: crate::messages::ObjectiveSnapshot {
            id: format!("retreat-{anchor}"),
            text: format!("Retreat to {anchor}"),
            mandatory: false,
            status: crate::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::messages::ObjectiveSource::Mission,
        },
    }
}

/// Point a fine system's control source at `source` on every ship.
pub fn set_fine_control_source(
    app: &mut App,
    system_id: crate::messages::SystemId,
    source: ControlSource,
) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
    for mut cs in q.iter_mut(app.world_mut()) {
        cs.0.set(system_id.clone(), source);
    }
}

/// The pre-#800 shape: the coarse `helm` system on AI with all four per-axis
/// systems left Human — which is what an *undeclared* axis resolves to
/// (`ControlSource::default() == Human`, so `operate_ai == false`).
///
/// This was the configuration `operate_helm_ai` was built to serve, and the
/// one every shipped hull was in before #800/#704 declared the axes. Since
/// #704 deleted the monolith it drives nothing at all, and several tests
/// below exist to pin exactly that: the coarse system is inert on its own,
/// there is no coarse fallback, and re-introducing one would light these up.
pub fn set_coarse_helm_only_ai(app: &mut App) {
    set_helm_control_source(app, ControlSource::Human);
    set_fine_control_source(
        app,
        // #801: "helm" is a station id, not a system. Seeding it here is
        // the point of the test — it must drive nothing.
        crate::messages::SystemId(crate::system_registry::HELM_STATION_ID.to_string()),
        ControlSource::Ai,
    );
}

/// The "partial automation" wiring the per-axis systems exist for: the
/// coarse helm stays human-held while both per-axis systems are AI.
pub fn set_per_axis_helm_ai(app: &mut App) {
    set_helm_control_source(app, ControlSource::Human);
    set_fine_control_source(
        app,
        crate::system_registry::helm_thrust_system_id(),
        ControlSource::Ai,
    );
    set_fine_control_source(
        app,
        crate::system_registry::helm_steering_system_id(),
        ControlSource::Ai,
    );
}

pub fn get_thrust_input(app: &mut App) -> f32 {
    app.world_mut()
        .query_filtered::<&ThrustInput, With<Ship>>()
        .single(app.world())
        .expect("expected Ship with ThrustInput")
        .0
}

pub fn get_steering_input(app: &mut App) -> f32 {
    app.world_mut()
        .query_filtered::<&SteeringInput, With<Ship>>()
        .single(app.world())
        .expect("expected Ship with SteeringInput")
        .0
}

pub fn patrol_scored_objective(anchors: Vec<&str>, score: f32) -> crate::messages::ScoredObjective {
    crate::messages::ScoredObjective {
        id: "obj-defend".into(),
        score,
        directive: crate::messages::AiDirective::Patrol {
            anchors: anchors.into_iter().map(str::to_string).collect(),
            loop_path: true,
        },
        source: crate::messages::ObjectiveSource::Mission,
        relevance: vec![crate::messages::SystemAffinity::Helm],
        snapshot: crate::messages::ObjectiveSnapshot {
            id: "obj-defend".into(),
            text: "Defend Starbase Alpha".into(),
            mandatory: true,
            status: crate::messages::ObjectiveStatus::Active,
            targets: vec!["Starbase Alpha".into()],
            source: crate::messages::ObjectiveSource::Mission,
        },
    }
}

pub fn destroy_scored_objective(target: &str, score: f32) -> crate::messages::ScoredObjective {
    crate::messages::ScoredObjective {
        id: format!("destroy-{target}"),
        score,
        directive: crate::messages::AiDirective::Destroy {
            target: target.into(),
        },
        source: crate::messages::ObjectiveSource::Mission,
        relevance: vec![
            crate::messages::SystemAffinity::Helm,
            crate::messages::SystemAffinity::Weapons,
            crate::messages::SystemAffinity::Captain,
        ],
        snapshot: crate::messages::ObjectiveSnapshot {
            id: format!("destroy-{target}"),
            text: format!("Destroy {target}"),
            mandatory: true,
            status: crate::messages::ObjectiveStatus::Active,
            targets: vec![target.into()],
            source: crate::messages::ObjectiveSource::Mission,
        },
    }
}

// ── Fine Helm system tests (issue #511) ───────────────────────────────────

/// Build an app that includes HelmEnginePort + HelmEngineStarboard hull
/// entries alongside the usual coarse consoles. Used for engine-damage tests.
pub fn test_app_with_engine_hull() -> App {
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        // See `test_app` — mirror the production SimSet chain (issue #830).
        .configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(ShipPlugin);
    let hull_config = &[
        (crate::messages::SystemId("helm".into()), 25.0_f32),
        (crate::messages::SystemId("tactical".into()), 25.0),
        (crate::messages::SystemId("power".into()), 25.0),
        (crate::messages::SystemId("shields".into()), 25.0),
        (crate::messages::SystemId("helm-engine-port".into()), 15.0),
        (
            crate::messages::SystemId("helm-engine-starboard".into()),
            15.0,
        ),
    ];
    let ship = app
        .world_mut()
        .spawn((
            Ship,
            LocalShip,
            Transform::default(),
            ShipPhysics::default(),
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
            ActiveStationRatings::default(),
            CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(
                hull_config,
            )),
            LastHelmInput::default(),
            crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ))
        .id();
    app.world_mut()
        .entity_mut(ship)
        .insert((ShipModifiers::new(), ShipBoost::default()));
    // This ship carries no AiHighFidelity bundle by default (unlike
    // `test_app()`), but `integrate_ship_physics` (issue #695) is
    // scoped to `AiHighFidelity`, and these engine-thrust tests drive
    // `ShipPhysics` purely through `LastHelmInput` + the human
    // admission/physics pipeline. Add the marker + helm intent
    // components so physics keeps integrating for this ship, matching
    // pre-#695 behavior where `process_helm_inputs` computed physics
    // for any `LocalShip` unconditionally.
    app.world_mut().entity_mut(ship).insert((
        crate::ai_plugin::AiHighFidelity,
        crate::ship::helm::ThrustInput::default(),
        crate::ship::helm::SteeringInput::default(),
        crate::ship::helm::LateralThrustInput::default(),
        crate::ship::helm::VerticalThrustInput::default(),
        crate::ship::helm::ImpulseCommand::default(),
        crate::ship::helm::BoostCommand::default(),
        // The console-owned surfaces the AI helm derives its goals from
        // (issue #702) — see `HelmAiSurfaces`.
        crate::weapons_plugin::TacticalRadarSelection::default(),
        crate::navigation_plugin::NavigationWaypoint::default(),
        HelmWaypointClearance::default(),
        crate::ai_plugin::ObjectiveCursors::default(),
    ));
    app
}

/// Set the HP of a specific system on the LocalShip hull to `new_hp`.
/// Delegates to `SystemHull::set_hp` which directly sets the value.
pub fn set_console_hp_direct(app: &mut App, system_id: crate::messages::SystemId, new_hp: f32) {
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let mut entity_mut = app.world_mut().entity_mut(ship);
    let mut hull = entity_mut
        .get_mut::<crate::entity_spawner::EntitySystemHull>()
        .unwrap();
    hull.0.set_hp(&system_id, new_hp);
}
