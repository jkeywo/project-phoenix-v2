use super::*;
use crate::core::messages::{AdmittedCommand, AdmittedCommands};
use crate::ship::config::parse_and_validate;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

const KINDS: &[&str] = &["red_alert", "viewscreen"];

/// A minimal hull whose Captain owns one automatable system, with a rating
/// that automates it and one that does not — enough to prove the applier
/// drives the control source in both directions.
const CONFIG_TOML: &str = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Manual"
automated_systems = []

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"
"#;

fn assign(station: &str, rating: &str) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId(ASSIGN_STATION_RATING_SYSTEM_ID.to_string()),
        payload: SystemControlPayload::AssignStationRating {
            station: StationId(station.into()),
            rating: rating.into(),
        },
        response_token: None,
        feedback_correlation: None,
    }
}

fn source_of(app: &App, ship: Entity, system: &str) -> ControlSource {
    app.world()
        .entity(ship)
        .get::<ShipSystemControlSources>()
        .unwrap()
        .0
        .source_for(&SystemId(system.into()))
}

fn spawn_ship(app: &mut App, cmd: AdmittedCommand, seed: ControlSource) -> Entity {
    let config = parse_and_validate(CONFIG_TOML, KINDS).expect("hull parses");
    let mut sources = ControlSourceResolver::default();
    sources.set(SystemId("red-alert".into()), seed);
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            AdmittedCommands(vec![cmd]),
            ShipConfigComponent(config),
            ShipSystemControlSources(sources),
            ActiveStationRatings::default(),
        ))
        .id()
}

#[test]
fn an_admitted_assign_backfills_the_captains_own_system_and_records_the_rating() {
    let mut app = App::new();
    // Seed the human-held state a live crew member would have set.
    let ship = spawn_ship(
        &mut app,
        assign("captain", "Assisted"),
        ControlSource::Human,
    );
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert_eq!(
        source_of(&app, ship, "red-alert"),
        ControlSource::Ai,
        "the Assisted rating automates the Captain's red-alert on this peer"
    );
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<ActiveStationRatings>()
            .unwrap()
            .0
            .get(&StationId("captain".into()))
            .map(String::as_str),
        Some("Assisted"),
        "the replicated rating is recorded for the GM projection to read"
    );
}

#[test]
fn an_admitted_assign_restores_human_control_when_the_rating_automates_nothing() {
    let mut app = App::new();
    // A Station the AI was holding; a returning human's Manual rating claims it.
    let ship = spawn_ship(&mut app, assign("captain", "Manual"), ControlSource::Ai);
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert_eq!(
        source_of(&app, ship, "red-alert"),
        ControlSource::Human,
        "a rating that automates nothing hands the Station back to the human"
    );
}

#[test]
fn a_command_for_another_target_is_ignored() {
    let mut app = App::new();
    let mut other = assign("captain", "Assisted");
    other.target = SystemId("red-alert".into());
    let ship = spawn_ship(&mut app, other, ControlSource::Human);
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert_eq!(
        source_of(&app, ship, "red-alert"),
        ControlSource::Human,
        "the applier reads only its own ownerless system id"
    );
}
