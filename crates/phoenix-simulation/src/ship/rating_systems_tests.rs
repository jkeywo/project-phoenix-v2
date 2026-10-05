use super::*;
use crate::core::messages::StationId;
use crate::server_app::Ship;
use crate::ship::control_source::ControlSource;
use crate::ship::test_support::*;

// ── Station Rating tests ─────────────────────────────────────────────

/// Build a ShipConfig with a captain station that has an "Assisted" rating
/// declaring red-alert as automated. Used by rating-mechanism tests that
/// need automation to be configured independently of the ship TOML.
fn ship_config_with_assisted_captain() -> ShipConfigComponent {
    const TOML: &str = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Captain"
rank = "Cpt."
short_code = "CPT"
console = "captain"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "helm"
name = "Helm"
description = "Helm"
rank = "Ltn."
short_code = "HLM"
console = "helm"

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "captain"
kind = "captain"
station = "captain"

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"

[[system]]
id = "viewscreen"
kind = "viewscreen"
ai_only = true

[[system]]
id = "helm"
kind = "helm"
station = "helm"
"#;
    const KINDS: &[&str] = &["captain", "red_alert", "viewscreen", "helm"];
    ShipConfigComponent(
        crate::ship::config::parse_and_validate(TOML, KINDS).expect("test config must be valid"),
    )
}

fn install_assisted_captain(app: &mut App) {
    // Apply the custom config directly on the Ship entity — PendingShipConfig
    // is consumed by spawn_game_start_entities which is not in the test app.
    let custom_config = ship_config_with_assisted_captain();
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
    for mut cfg in q.iter_mut(app.world_mut()) {
        *cfg = custom_config.clone();
    }
}

#[test]
fn set_station_rating_sets_ai_for_automated_systems() {
    let mut app = test_app();
    install_assisted_captain(&mut app);
    start_game_with_helm_and_science(&mut app);

    // Captain "Assisted" rating has red-alert in automated_systems.
    push(
        &mut app,
        "captain",
        ClientMessage::SetStationRating {
            rating_name: "Assisted".into(),
        },
    );
    tick_twice(&mut app);

    let sources = get_ship_control_sources(&mut app);
    assert_eq!(
        sources
            .0
            .source_for(&crate::ship::system_registry::red_alert_system_id()),
        ControlSource::Ai
    );
}

#[test]
fn set_station_rating_manual_leaves_all_systems_human() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::SetStationRating {
            rating_name: "Std".into(),
        },
    );
    tick_twice(&mut app);

    let sources = get_ship_control_sources(&mut app);
    assert_eq!(
        sources
            .0
            .source_for(&crate::ship::system_registry::red_alert_system_id()),
        ControlSource::Human
    );
}

#[test]
fn set_station_rating_backfill_automates_all_station_systems() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::SetStationRating {
            rating_name: rating::BACKFILL_RATING.into(),
        },
    );
    tick_twice(&mut app);

    let sources = get_ship_control_sources(&mut app);
    assert_eq!(
        sources
            .0
            .source_for(&crate::ship::system_registry::red_alert_system_id()),
        ControlSource::Ai
    );
    // viewscreen is now owned by the captain station, so backfill must automate it too.
    assert_eq!(
        sources
            .0
            .source_for(&crate::ship::system_registry::viewscreen_system_id()),
        ControlSource::Ai,
        "backfill should also automate the captain-owned viewscreen system"
    );
}

#[test]
fn set_station_rating_from_non_holder_is_ignored() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);

    // "helm" player holds Helm console, not captain.
    // SetStationRating for the captain station should be ignored.
    push(
        &mut app,
        "helm",
        ClientMessage::SetStationRating {
            rating_name: "Assisted".into(),
        },
    );
    tick_twice(&mut app);

    let sources = get_ship_control_sources(&mut app);
    // Default is Human
    assert_eq!(
        sources
            .0
            .source_for(&crate::ship::system_registry::red_alert_system_id()),
        ControlSource::Human
    );
}

#[test]
fn set_station_rating_updates_active_ratings() {
    let mut app = test_app();
    install_assisted_captain(&mut app);
    start_game_with_helm_and_science(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::SetStationRating {
            rating_name: "Assisted".into(),
        },
    );
    tick_twice(&mut app);

    let active = get_ship_active_ratings(&mut app);
    assert_eq!(
        active
            .0
            .get(&StationId("captain".into()))
            .map(|s| s.as_str()),
        Some("Assisted")
    );
}

#[test]
fn set_station_rating_emits_rating_changed() {
    let mut app = test_app();
    install_assisted_captain(&mut app);
    start_game_with_helm_and_science(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::SetStationRating {
            rating_name: "Assisted".into(),
        },
    );
    tick_twice(&mut app);

    let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
    let has_rating_changed = outbox.0.iter().any(|(_, msg)| {
        matches!(
            msg,
            crate::core::messages::ServerMessage::RatingChanged {
                station_id,
                rating_name,
            } if station_id.0 == "captain" && rating_name == "Assisted"
        )
    });
    assert!(has_rating_changed, "expected RatingChanged in outbox");
}
