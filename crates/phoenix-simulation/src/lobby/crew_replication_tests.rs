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

[[console]]
id = "captain-console"
home_station = "captain"
screen = "fixture.html"

[[system]]
id = "red-alert"
kind = "red_alert"
console = "captain-console"
power_group = "ops"
"#;

fn assign(station: &str, rating: &str) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId(ASSIGN_STATION_RATING_SYSTEM_ID.to_string()),
        payload: SystemControlPayload::AssignStationRating {
            occupied: None,
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
            ShipCrewPresence::default(),
        ))
        .id()
}

#[test]
fn actual_admitted_presence_is_idempotent_and_rating_only_human_cannot_invent_participation() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, assign("captain", "Manual"), ControlSource::Ai);
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert_eq!(source_of(&app, ship, "red-alert"), ControlSource::Human);
    assert!(app
        .world()
        .get::<ShipCrewPresence>(ship)
        .unwrap()
        .0
        .is_empty());
    let mut occupied = assign("captain", "Manual");
    if let SystemControlPayload::AssignStationRating {
        occupied: value, ..
    } = &mut occupied.payload
    {
        *value = Some(true);
    }
    app.world_mut()
        .entity_mut(ship)
        .insert(AdmittedCommands(vec![occupied.clone(), occupied]));
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert_eq!(
        app.world().get::<ShipCrewPresence>(ship).unwrap().0.len(),
        1
    );
    let mut away = assign("captain", "Backfill");
    if let SystemControlPayload::AssignStationRating { occupied, .. } = &mut away.payload {
        *occupied = Some(false);
    }
    app.world_mut()
        .entity_mut(ship)
        .insert(AdmittedCommands(vec![away]));
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert!(app
        .world()
        .get::<ShipCrewPresence>(ship)
        .unwrap()
        .0
        .is_empty());
    let before = app
        .world()
        .get::<ActiveStationRatings>(ship)
        .unwrap()
        .0
        .clone();
    let mut invalid = assign("missing-station", "Manual");
    if let SystemControlPayload::AssignStationRating { occupied, .. } = &mut invalid.payload {
        *occupied = Some(true);
    }
    app.world_mut()
        .entity_mut(ship)
        .insert(AdmittedCommands(vec![invalid]));
    app.world_mut()
        .run_system_cached(apply_assigned_station_rating)
        .unwrap();
    assert_eq!(
        app.world().get::<ActiveStationRatings>(ship).unwrap().0,
        before
    );
    assert!(app
        .world()
        .get::<ShipCrewPresence>(ship)
        .unwrap()
        .0
        .is_empty());
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

#[test]
fn admitted_fleet_crew_handoffs_survive_console_resolution_on_every_peer() {
    use crate::command_admission::HostSlot;
    use crate::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
    use crate::ship::components::{HumanSeekingHosts, ScenarioDetailFloor};
    use crate::ship_plugin::ConsoleHosts;

    let station = StationId("captain".into());
    let system = SystemId("red-alert".into());
    let mut peers = [HostSlot(1), HostSlot(2)].map(|local_slot| {
        let config = parse_and_validate(CONFIG_TOML, KINDS).unwrap();
        let (sources, ratings) = crate::ship::rating::seed_boot_ratings(&config, |_| {
            crate::ship::rating::BACKFILL_RATING.into()
        });
        let mut app = App::new();
        app.init_resource::<Time<Fixed>>();
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ));
        let mut world_config = crate::world::config::WorldConfig::default();
        world_config.global.seed = Some(7);
        app.insert_resource(world_config);
        let roster = FleetRoster::new(
            [HostSlot(1), HostSlot(2)]
                .into_iter()
                .map(FleetShip::new)
                .collect(),
            local_slot,
        );
        assert!(crate::lockstep::join_fleet(app.world_mut(), roster, 2));
        // The local crew views deliberately disagree. Only the admitted
        // transition below may affect this ship's Console on either peer.
        if local_slot == HostSlot(1) {
            let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
            sessions
                .0
                .register("local-officer".into(), "Officer".into())
                .unwrap();
            sessions
                .0
                .set_station("local-officer", Some(station.clone()));
        }
        let ship = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                FleetSlotOf(HostSlot(1)),
                ShipConfigComponent(config),
                ShipSystemControlSources(sources),
                ActiveStationRatings(ratings),
                AdmittedCommands::default(),
                ScenarioDetailFloor::default(),
                HumanSeekingHosts::default(),
                ConsoleHosts::default(),
            ))
            .id();
        (app, ship)
    });
    for (rating, expected_source, expected_host) in [
        (None, ControlSource::Ai, None),
        (Some("Manual"), ControlSource::Human, Some(station.clone())),
        (
            Some(crate::ship::rating::BACKFILL_RATING),
            ControlSource::Ai,
            None,
        ),
        (Some("Manual"), ControlSource::Human, Some(station.clone())),
    ] {
        for (app, ship) in &mut peers {
            if let Some(rating) = rating {
                app.world_mut()
                    .get_mut::<AdmittedCommands>(*ship)
                    .unwrap()
                    .0 = vec![assign("captain", rating)];
                app.world_mut()
                    .run_system_cached(apply_assigned_station_rating)
                    .unwrap();
            }
            app.world_mut()
                .run_system_cached(crate::ship::coordination_systems::resolve_human_seeking_hosts)
                .unwrap();
            assert_eq!(source_of(app, *ship, &system.0), expected_source);
            let assignments = &app.world().get::<ConsoleHosts>(*ship).unwrap().0;
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].host, expected_host);
            assert_eq!(
                app.world()
                    .get::<HumanSeekingHosts>(*ship)
                    .unwrap()
                    .host_for(&system),
                expected_host.as_ref(),
            );
            assert!(app
                .world()
                .resource::<FleetRoster>()
                .crew_of(HostSlot(1))
                .is_empty());
        }
        assert_eq!(
            peers[0]
                .0
                .world()
                .get::<ConsoleHosts>(peers[0].1)
                .unwrap()
                .0,
            peers[1]
                .0
                .world()
                .get::<ConsoleHosts>(peers[1].1)
                .unwrap()
                .0,
        );
    }
    // A standalone resume intentionally releases the old remote crew; retained
    // ratings alone must not re-seat them once the active mesh is gone.
    let (remote_app, remote_ship) = &mut peers[1];
    remote_app
        .world_mut()
        .remove_resource::<crate::lockstep::FleetLockstep>();
    remote_app
        .world_mut()
        .run_system_cached(crate::ship::coordination_systems::resolve_human_seeking_hosts)
        .unwrap();
    assert_eq!(
        source_of(remote_app, *remote_ship, &system.0),
        ControlSource::Ai
    );
    assert!(remote_app
        .world()
        .get::<ConsoleHosts>(*remote_ship)
        .unwrap()
        .0[0]
        .host
        .is_none());
}
