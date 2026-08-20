//! Unit tests for the Command console server wiring (issue #1107).
//!
//! These drive the plugin's systems directly over a hand-built world (post
//! admission), so they pin the CONTENT rules — what a stance order does once it
//! is admitted, how the alert switch and the AI operator behave, and what the
//! console blackboard reports. The admission AUTHORISATION of the order and the
//! wire round-trip are pinned in `command_admission`/`core::codec`.

use super::*;
use crate::messages::{
    AdmittedCommand, AdmittedCommands, StationId, SystemControlPayload, SystemId,
};
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

const KINDS: &[&str] = &["red_alert", "command", "phaser_bank"];

/// A captain, an AI-controllable proving Station ("tactical", owning a weapon
/// so `weapons_station()` resolves to it) with a full stance catalogue, and an
/// auxiliary Command station directing it.
fn command_config() -> ShipConfig {
    ShipConfig::from_toml(
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[station.rating]]
name = "Std"
automated_systems = []

[[station.stance]]
id = "weapons-free"
kind = "standard"
high_alert = true

[[station.stance]]
id = "hold"
kind = "standard"
high_alert = false

[[station.stance]]
id = "normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "high"
kind = "high_alert_neutral"
high_alert = true

[[station]]
id = "command"
name = "Command"
description = "Direct an AI station."
rank = "Cpt."
console = "gui/command-console.html"
auxiliary = true
human_seeking = true
host_order = ["captain"]
visiting_rating = "Std"
command_target = "tactical"

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "command"
kind = "command"
station = "command"
"#,
        KINDS,
    )
    .expect("command config parses")
}

fn tactical() -> StationId {
    StationId("tactical".into())
}

/// Spawn one Ship with the command config. `tactical_ai`/`command_ai` set the
/// initial control sources; everything else defaults.
fn spawn_ship(app: &mut App, tactical_ai: bool, command_ai: bool) -> Entity {
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        SystemId("phaser-fore".into()),
        if tactical_ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        },
    );
    sources.0.set(
        crate::system_registry::command_system_id(),
        if command_ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        },
    );
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipConfigComponent(command_config()),
            sources,
            AdmittedCommands::default(),
            ShipStationStances::default(),
            crate::ship_state::ShipRedAlert::default(),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id()
}

fn set_admitted(app: &mut App, ship: Entity, station: &str, stance: &str) {
    let mut admitted = app.world_mut().entity_mut(ship);
    let mut cmds = admitted.get_mut::<AdmittedCommands>().unwrap();
    cmds.0.push(AdmittedCommand {
        target: crate::system_registry::command_system_id(),
        payload: SystemControlPayload::SetStationStance {
            station: StationId(station.into()),
            stance: stance.into(),
        },
        response_token: None,
    });
}

fn stances(app: &App, ship: Entity) -> ShipStationStances {
    app.world()
        .entity(ship)
        .get::<ShipStationStances>()
        .unwrap()
        .clone()
}

#[test]
fn an_authored_stance_for_the_directed_ai_station_is_applied() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    set_admitted(&mut app, ship, "tactical", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
    );
}

#[test]
fn an_unauthored_stance_is_rejected() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    set_admitted(&mut app, ship, "tactical", "invented-order");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(
        stances(&app, ship).0.is_empty(),
        "Command invents no orders"
    );
}

#[test]
fn a_stance_is_refused_while_the_station_is_human_controlled() {
    // Criterion 2: Command applies a stance only while the target is AI.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false);
    set_admitted(&mut app, ship, "tactical", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(
        stances(&app, ship).0.is_empty(),
        "a human-held Station is off the Command board"
    );
}

#[test]
fn a_stance_for_an_undirected_station_is_refused() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    // Command directs "tactical", not "captain".
    set_admitted(&mut app, ship, "captain", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(stances(&app, ship).0.is_empty());
}

#[test]
fn changing_alert_switches_a_stored_neutral_but_not_a_standard_stance() {
    // Criterion 5. Start in the normal-alert neutral; raise the alert; the
    // stored neutral follows to the high-alert neutral.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipStationStances>()
        .unwrap()
        .0
        .insert(tactical(), "normal".into());

    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::ship_state::ShipRedAlert>()
        .unwrap()
        .0 = true;
    app.world_mut()
        .run_system_cached(apply_alert_change_to_stances)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("high"),
        "a stored neutral follows the alert to the other neutral"
    );

    // A standard stance is never overwritten by an alert change.
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipStationStances>()
        .unwrap()
        .0
        .insert(tactical(), "hold".into());
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::ship_state::ShipRedAlert>()
        .unwrap()
        .0 = false;
    app.world_mut()
        .run_system_cached(apply_alert_change_to_stances)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("hold"),
        "an explicit standard stance survives an alert change"
    );
}

#[test]
fn ai_command_resets_a_non_persistent_standard_stance_to_neutral() {
    // Lifecycle: when Command loses its human, a non-persistent standard order
    // clears back to tracking (an empty entry == the alert-tracking default).
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, true); // command AI
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipStationStances>()
        .unwrap()
        .0
        .insert(tactical(), "hold".into()); // standard, persist=false
    app.world_mut()
        .run_system_cached(operate_command_ai)
        .unwrap();
    assert!(
        !stances(&app, ship).0.contains_key(&tactical()),
        "AI Command drops the human's non-persistent order back to neutral"
    );
}

#[test]
fn the_stance_override_feeds_the_weapons_posture_only_when_directed_and_ai() {
    // Criterion 4: the selection feeds the weapons AI host's alert posture.
    let config = command_config();
    let mut sources = ControlSourceResolver::default();
    sources.set(SystemId("phaser-fore".into()), ControlSource::Ai);
    let mut selections = ShipStationStances::default();

    // No selection → None → tracks red alert (byte-identical default).
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), &config, &sources, false),
        None
    );

    // A human's "hold" stance forces stood-down even at red alert.
    selections.0.insert(tactical(), "hold".into());
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), &config, &sources, true),
        Some(false),
        "a hold stance overrides the ship's own red alert for the fire gate"
    );

    // The same selection is ignored once a human holds the weapons Station.
    let mut human_sources = ControlSourceResolver::default();
    human_sources.set(SystemId("phaser-fore".into()), ControlSource::Human);
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), &config, &human_sources, true),
        None,
        "Command does not direct a human-held weapons Station"
    );
}

#[test]
fn the_blackboard_lists_the_directed_station_and_its_stances() {
    // Criterion 2: the console shows the directed Station, its AI state and the
    // selectable stances (the persistent non-colour cue is derived from these).
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    app.world_mut()
        .run_system_cached(publish_command_blackboard)
        .unwrap();
    let bbs = app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    let SystemBlackboard::Command(bb) = bbs
        .0
        .get(&SystemId(
            crate::system_registry::COMMAND_SYSTEM_ID.to_string(),
        ))
        .expect("a command blackboard is published")
    else {
        panic!("expected a Command blackboard");
    };
    assert_eq!(bb.directed_station, tactical());
    assert_eq!(bb.directed_station_name, "Tactical");
    assert!(bb.directed_station_ai);
    assert_eq!(bb.stances.len(), 4);
    // With nothing selected the console shows the alert level's neutral.
    assert_eq!(bb.selected_stance, "normal");
}

#[test]
fn the_blackboard_marks_the_station_off_the_board_when_human_held() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    app.world_mut()
        .run_system_cached(publish_command_blackboard)
        .unwrap();
    let bbs = app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    let SystemBlackboard::Command(bb) = bbs
        .0
        .get(&SystemId(
            crate::system_registry::COMMAND_SYSTEM_ID.to_string(),
        ))
        .unwrap()
    else {
        panic!("expected a Command blackboard");
    };
    assert!(
        !bb.directed_station_ai,
        "a human at the directed Station takes it off the Command board"
    );
}
