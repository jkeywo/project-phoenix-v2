//! Adapter tests for the controlled-demolition operation (issue #1350).
//!
//! The pure half — the detonation verdict, the four-outcome decision and the
//! config validation — is tested in [`crate::demolition::ops`]. What is tested
//! here is everything only the world can answer: that a `DetonateCharges` command
//! routed on the `security` target reaches the operation, that the outcome the
//! world's team/tractor state implies is the flag that gets raised, that the
//! rising edge reaches the world-event stream a scenario trigger reacts to, that
//! a second detonation is refused, that the refusal projection and the readout
//! publish, and that the backfill host fires exactly the safe shot a console
//! would.

use super::*;
use crate::core::messages::{AdmittedCommand, DemolitionBlackboard, PowerGroupId};
use crate::demolition::ops::{DemolitionConfig, DemolitionRefusal};
use crate::security::teams::{SecurityAction, SecurityConfig};
use crate::security::ShipSecurityTeams;
use crate::tractor::TractorBeam;

const OPERATOR: &str = "destroyer-1";
const OBSTRUCTION: &str = "obstruction-1";
const CHARGES: &str = "obstruction_charges_placed";
const DETONATED: &str = "obstruction_detonated";
const SAFE: &str = "obstruction_cleared_safe";
const UNSUPPORTED: &str = "obstruction_cleared_unsupported";
const PREMATURE: &str = "obstruction_detonated_premature";

fn demo_config(stabilization_required: bool) -> DemolitionConfig {
    DemolitionConfig {
        charges_flag: CHARGES.to_string(),
        detonated_flag: DETONATED.to_string(),
        safe_flag: SAFE.to_string(),
        unsupported_flag: UNSUPPORTED.to_string(),
        premature_flag: PREMATURE.to_string(),
        stabilization_required,
        warning: Some("demolition.warning.test".to_string()),
    }
}

fn security_config() -> SecurityConfig {
    SecurityConfig {
        team_count: 1,
        deploy_duration_secs: 2.0,
        withdraw_duration_secs: 2.0,
        range: 400.0,
    }
}

/// A tractor beam already coupled to `target`, built with the least authored
/// terms that construct one — only `coupled_target` is read by the demolition
/// adapter.
fn coupled_beam(target: &str) -> TractorBeam {
    use crate::tractor::coupling::{TowLoadCurve, TractorConfig};
    let mut beam = TractorBeam::new(
        TractorConfig {
            range: 500.0,
            coupling_offset: [0.0, 0.0, -10.0],
            min_power_level: 1,
            tow_load: TowLoadCurve {
                half_penalty_mass: 100.0,
                max_penalty: 0.5,
            },
        },
        PowerGroupId("engineering".to_string()),
    );
    beam.engaged = true;
    beam.coupled_target = Some(target.to_string());
    beam
}

/// A bare app carrying the handler and the publisher, plus a per-tick
/// `AdmittedCommands` clear so a single command is consumed exactly once — the
/// same thing production's admission refill does.
fn app_with(stabilization_required: bool, spawn_target: bool) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.add_systems(
        Update,
        (
            handle_demolition_commands,
            publish_demolition_blackboard,
            clear_admitted,
        )
            .chain(),
    );
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.to_string()),
            Transform::from_translation(Vec3::ZERO),
            AdmittedCommands::default(),
            ShipSecurityTeams::new(security_config()),
            DemolitionControl::default(),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();
    if spawn_target {
        app.world_mut().spawn((
            EntityUuid(OBSTRUCTION.to_string()),
            EntityName("world.test.obstruction.name".to_string()),
            Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)),
            DemolitionTarget(demo_config(stabilization_required)),
        ));
    }
    (app, operator)
}

fn clear_admitted(mut inboxes: Query<&mut AdmittedCommands>) {
    for mut inbox in inboxes.iter_mut() {
        if !inbox.0.is_empty() {
            inbox.0.clear();
        }
    }
}

fn set_flag(app: &mut App, name: &str) {
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .flags
        .set_flag_value(name, 1);
}

fn flag(app: &App, name: &str) -> bool {
    app.world()
        .resource::<WorldContentRuntime>()
        .flags
        .flag(name)
}

fn detonate(app: &mut App, operator: Entity, target: &str) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<AdmittedCommands>()
        .expect("the operator carries an admitted-command inbox")
        .0
        .push(AdmittedCommand {
            target: security_system_id(),
            payload: SystemControlPayload::DetonateCharges {
                target: target.to_string(),
            },
            response_token: None,
            feedback_correlation: None,
        });
}

/// Put the operator's one team on the obstruction, committed (deploying) — the
/// team-not-clear state a premature shot catches.
fn commit_team_to_obstruction(app: &mut App, operator: Entity) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<ShipSecurityTeams>()
        .expect("the operator musters Security teams")
        .teams[0]
        .deploy(
            OBSTRUCTION.to_string(),
            SecurityAction::PlaceCharges,
            0.5,
            2.0,
        );
}

fn set_beam(app: &mut App, operator: Entity, beam: TractorBeam) {
    app.world_mut().entity_mut(operator).insert(beam);
}

fn refusal(app: &App, operator: Entity) -> Option<DemolitionRefusal> {
    app.world()
        .entity(operator)
        .get::<DemolitionControl>()
        .expect("the operator carries demolition control state")
        .last_refusal
}

fn blackboard(app: &App, operator: Entity) -> DemolitionBlackboard {
    match app
        .world()
        .entity(operator)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("the operator publishes blackboards")
        .0
        .get(&demolition_blackboard_key())
    {
        Some(SystemBlackboard::Demolition(bb)) => bb.clone(),
        other => panic!("expected a Demolition blackboard, got {other:?}"),
    }
}

/// AC1/AC2: a charge placement that has completed does NOT arm an automatic
/// detonation, and detonating an uncharged obstruction is refused — the crew are
/// told there is nothing to fire.
#[test]
fn an_uncharged_obstruction_cannot_be_detonated() {
    let (mut app, operator) = app_with(false, true);
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();

    assert_eq!(refusal(&app, operator), Some(DemolitionRefusal::NotCharged));
    assert!(!flag(&app, DETONATED), "nothing was fired");
    assert!(!flag(&app, SAFE));
}

/// AC3: charges placed, the team clear, and no hold required — the obstruction
/// clears with the SAFE outcome, and the rising edge reaches the world-event
/// stream the scenario hangs its limited-debris consequence off.
#[test]
fn a_clear_unheld_shot_that_needs_no_hold_is_safe() {
    let (mut app, operator) = app_with(false, true);
    set_flag(&mut app, CHARGES);
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();

    assert_eq!(refusal(&app, operator), None);
    assert!(flag(&app, SAFE), "the safe outcome flag rises");
    assert!(flag(&app, DETONATED), "the operation is marked spent");
    assert!(!flag(&app, UNSUPPORTED));
    assert!(!flag(&app, PREMATURE));

    let events = &app
        .world()
        .resource::<WorldContentRuntime>()
        .pending_world_events;
    let raised: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            crate::world::content::WorldEvent::FlagSet { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(raised.contains(&SAFE), "the scenario sees the safe edge");
    assert!(
        raised.contains(&DETONATED),
        "and the generic detonated edge"
    );
}

/// AC3: charges placed and the team clear, but the mass needed a tractor hold and
/// had none — the UNSUPPORTED outcome, more debris.
#[test]
fn a_required_hold_that_is_absent_is_unsupported() {
    let (mut app, operator) = app_with(true, true);
    set_flag(&mut app, CHARGES);
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();

    assert!(flag(&app, UNSUPPORTED), "no hold on a mass that needed one");
    assert!(flag(&app, DETONATED));
    assert!(!flag(&app, SAFE));
    assert!(!flag(&app, PREMATURE));
}

/// AC3: charges placed, the mass held where it needed holding, the team clear —
/// the SAFE outcome even though a hold was required, because the beam is on it.
#[test]
fn a_required_hold_that_is_present_is_safe() {
    let (mut app, operator) = app_with(true, true);
    set_beam(&mut app, operator, coupled_beam(OBSTRUCTION));
    set_flag(&mut app, CHARGES);
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();

    assert!(flag(&app, SAFE), "a held mass that needed holding is safe");
    assert!(!flag(&app, UNSUPPORTED));
}

/// AC3: detonating while a Security team is still committed to the target is the
/// PREMATURE outcome — casualties — regardless of whether the mass was held.
#[test]
fn detonating_with_the_team_still_on_the_target_is_premature() {
    let (mut app, operator) = app_with(true, true);
    set_beam(&mut app, operator, coupled_beam(OBSTRUCTION));
    set_flag(&mut app, CHARGES);
    commit_team_to_obstruction(&mut app, operator);
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();

    assert!(
        flag(&app, PREMATURE),
        "a team on the target dies even with the mass held"
    );
    assert!(!flag(&app, SAFE));
    assert!(!flag(&app, UNSUPPORTED));
    assert!(flag(&app, DETONATED));
}

/// AC1: detonation is a one-shot. A second `DetonateCharges` after the charges
/// have gone off is refused rather than firing again.
#[test]
fn a_second_detonation_is_refused_as_already_detonated() {
    let (mut app, operator) = app_with(false, true);
    set_flag(&mut app, CHARGES);
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();
    assert!(flag(&app, SAFE));

    detonate(&mut app, operator, OBSTRUCTION);
    app.update();
    assert_eq!(
        refusal(&app, operator),
        Some(DemolitionRefusal::AlreadyDetonated)
    );
}

/// AC2: a disabled Security System fires nothing — the station that sets off the
/// charges is out, and the crew are told so before anything about the operation.
#[test]
fn a_disabled_security_system_refuses_the_detonation() {
    use crate::ship::damage::SystemHull;

    let (mut app, operator) = app_with(false, true);
    set_flag(&mut app, CHARGES);
    // A Security System with no hull left reads as damaged out — the station that
    // fires the charges is gone.
    let hull = SystemHull::from_config(&[(security_system_id(), 0.0)]);
    app.world_mut()
        .entity_mut(operator)
        .insert(EntitySystemHull(hull));
    detonate(&mut app, operator, OBSTRUCTION);
    app.update();

    assert_eq!(refusal(&app, operator), Some(DemolitionRefusal::Disabled));
    assert!(!flag(&app, DETONATED));
}

/// A detonation naming a uuid nothing in the world answers to is refused.
#[test]
fn an_unknown_target_is_refused() {
    let (mut app, operator) = app_with(false, false);
    detonate(&mut app, operator, "no-such-obstruction");
    app.update();
    assert_eq!(
        refusal(&app, operator),
        Some(DemolitionRefusal::NoSuchTarget)
    );
}

/// AC1/AC2: the operation's state is VISIBLE — the readout carries the charged,
/// team-clear, stabilisation and detonated facts a Tactical seat reads before
/// firing, so a completed placement is not mistaken for a fired charge.
#[test]
fn the_readout_publishes_the_operation_state() {
    let (mut app, operator) = app_with(true, true);
    app.update();
    let bb = blackboard(&app, operator);
    assert_eq!(bb.targets.len(), 1);
    let slot = &bb.targets[0];
    assert_eq!(slot.target, OBSTRUCTION);
    assert!(!slot.charged, "nothing placed yet");
    assert!(slot.team_clear, "no team out");
    assert!(slot.stabilization_required);
    assert!(!slot.stabilized);
    assert!(!slot.detonated);
    assert_eq!(slot.warning.as_deref(), Some("demolition.warning.test"));

    // Place charges and the readout arms; commit a team and team-clear drops.
    set_flag(&mut app, CHARGES);
    commit_team_to_obstruction(&mut app, operator);
    app.update();
    let slot = blackboard(&app, operator).targets.remove(0);
    assert!(slot.charged, "the detonate control is armed");
    assert!(!slot.team_clear, "the team is on the target");
}

// ── The backfill host ────────────────────────────────────────────────────────

/// The host and the resources admission needs, with the Security System's control
/// source set to `Ai` — the whole of what makes this the backfill seat.
fn ai_app(stabilization_required: bool) -> (App, Entity) {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.add_systems(Update, operate_demolition_ai);

    let mut sources = ControlSourceResolver::new();
    sources.set(security_system_id(), ControlSource::Ai);
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.to_string()),
            Transform::from_translation(Vec3::ZERO),
            AdmittedCommands::default(),
            ShipSecurityTeams::new(security_config()),
            DemolitionControl::default(),
            crate::ship_plugin::ShipSystemControlSources(sources),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(OBSTRUCTION.to_string()),
        EntityName("world.test.obstruction.name".to_string()),
        Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)),
        DemolitionTarget(demo_config(stabilization_required)),
    ));
    (app, operator)
}

fn host_payloads(app: &App, operator: Entity) -> Vec<SystemControlPayload> {
    app.world()
        .entity(operator)
        .get::<AdmittedCommands>()
        .expect("inbox")
        .0
        .iter()
        .map(|c| c.payload.clone())
        .collect()
}

/// AC5: the Backfill fires the charges — the SAME `DetonateCharges` a console
/// sends — but only when the shot is the safe one: charged, team clear, and held
/// if a hold was needed.
#[test]
fn the_backfill_fires_only_the_safe_shot() {
    // Not charged yet: the host waits.
    let (mut app, operator) = ai_app(false);
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "nothing to detonate before charges are placed"
    );

    // Charged, team clear, no hold required: the host fires.
    set_flag(&mut app, CHARGES);
    app.update();
    assert_eq!(
        host_payloads(&app, operator),
        vec![SystemControlPayload::DetonateCharges {
            target: OBSTRUCTION.to_string(),
        }],
        "the host's decision is the console's command, field for field"
    );
}

/// AC5: the Backfill never chooses the premature or unsupported outcome. A team
/// still on the target, or a required hold that is absent, keeps the host's hand
/// off the button.
#[test]
fn the_backfill_will_not_fire_a_premature_or_unsupported_shot() {
    // A required hold that is absent: no safe shot, so no command.
    let (mut app, operator) = ai_app(true);
    set_flag(&mut app, CHARGES);
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "the host will not scatter debris on its own initiative"
    );

    // A team still committed: premature, so the host holds even with no hold
    // requirement.
    let (mut app, operator) = ai_app(false);
    set_flag(&mut app, CHARGES);
    commit_team_to_obstruction(&mut app, operator);
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "the host will not take casualties on its own initiative"
    );
}
