use super::*;
use crate::core::messages::{PowerGroupId, SystemId};
use crate::lobby::stations_config::{ShipStations, StationDef};
use crate::ship::config::{PowerGroupConfig, StationConfig, SystemInstanceConfig};

fn sm() -> SessionManager {
    SessionManager::new()
}

fn test_stations() -> ShipStations {
    let station = |id: &str, name: &str| StationDef {
        id: StationId(id.into()),
        name: name.into(),
        description: "".into(),
        rank: "".into(),
        short_code: "".into(),
        console: None,
        ratings: vec![],
        human_seeking: false,
        host_order: vec![],
        visiting_rating: None,
        auxiliary: false,
        command_target: None,
    };
    ShipStations {
        stations: [
            ("captain", "Captain"),
            ("helm", "Helm"),
            ("tactical", "Tactical"),
            ("repair", "Repair"),
            ("sensors", "Sensors"),
            ("shields", "Shields"),
            ("navigation", "Navigation"),
            ("power", "Power"),
            ("comms", "Comms"),
        ]
        .into_iter()
        .map(|(id, name)| station(id, name))
        .collect(),
    }
}

fn test_ship_config() -> ShipConfig {
    ShipConfig {
        stations: test_stations()
            .stations
            .into_iter()
            .map(|station| StationConfig {
                id: station.id,
                name: station.name,
                description: station.description,
                rank: station.rank,
                short_code: station.short_code,
                ratings: vec![],
                console: None,
                manual_overview: None,
                tutorials: vec![],
                human_seeking: false,
                host_order: vec![],
                visiting_rating: None,
                auxiliary: false,
                command_target: None,
                stances: vec![],
            })
            .collect(),
        systems: vec![SystemInstanceConfig {
            id: SystemId("dummy".into()),
            kind: "dummy".into(),
            station: None,
            ai_only: true,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }],
        power_groups: std::iter::once((
            PowerGroupId("ops".into()),
            PowerGroupConfig {
                label: "Ops".into(),
                default_level: 2,
                min_level: 1,
                max_level: 4,
            },
        ))
        .collect(),
        coordination_lag_secs: 2.0,
    }
}

#[test]
fn register_new_player() {
    let mut sm = sm();
    let p = sm.register("t1".into(), "Alice".into()).unwrap();
    assert_eq!(p.token, "t1");
    assert_eq!(p.name, "Alice");
    assert!(p.connected);
    assert!(p.station.is_none());
}

#[test]
fn fleet_lobby_ratings_follow_connected_seats_and_pending_choices() {
    let mut sm = sm();
    let mut stations = test_stations();
    stations.stations[0].ratings = vec!["Assisted".into()];
    stations.stations[3].auxiliary = true;
    for (token, station) in [
        ("captain-token", "captain"),
        ("helm-token", "helm"),
        ("departed-token", "tactical"),
        ("aux-token", "repair"),
        ("spectator-token", "sensors"),
    ] {
        sm.register(token.into(), format!("Private {token}"))
            .unwrap();
        sm.set_station(token, Some(StationId(station.into())));
    }
    sm.set_pending_rating(&StationId("helm".into()), "Manual".into());
    sm.set_last_rating("captain-token", Some("Old reconnect rating".into()));
    sm.disconnect("departed-token");
    sm.set_spectator("spectator-token", true);
    assert_eq!(
        sm.lobby_station_ratings(&stations),
        vec![
            (StationId("captain".into()), "Assisted".into()),
            (StationId("helm".into()), "Manual".into()),
        ]
    );
    sm.set_station("helm-token", None);
    assert_eq!(
        sm.lobby_station_ratings(&stations),
        vec![(StationId("captain".into()), "Assisted".into()),]
    );
}

#[test]
fn duplicate_token_fails() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(matches!(
        sm.register("t1".into(), "Bob".into()),
        Err(RegisterError::DuplicateToken)
    ));
}

#[test]
fn disconnect_marks_player_as_disconnected() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.disconnect("t1");
    assert!(!sm.players()[0].connected);
}

#[test]
fn disconnect_clears_ready_flag() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_ready("t1", true);
    assert!(sm.players()[0].ready);
    sm.disconnect("t1");
    assert!(!sm.players()[0].ready);
}

#[test]
fn reconnect_marks_player_as_connected() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.disconnect("t1");
    assert!(!sm.players()[0].connected);
    sm.reconnect("t1");
    assert!(sm.players()[0].connected);
}

#[test]
fn set_name_updates_name() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_name("t1", "Alicia".into());
    assert_eq!(sm.players()[0].name, "Alicia");
}

#[test]
fn players_returns_all_registered() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    assert_eq!(sm.players().len(), 2);
}

#[test]
fn holder_for_station_returns_player_at_captain_chair() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    assert_eq!(
        sm.holder_for_station(&StationId("captain".into())),
        Some("t1")
    );
}

#[test]
fn holder_for_station_returns_none_when_unclaimed() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert_eq!(sm.holder_for_station(&StationId("captain".into())), None);
}

#[test]
fn holder_for_station_returns_correct_helm() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_station("t2", Some(StationId("helm".into())));
    assert_eq!(sm.holder_for_station(&StationId("helm".into())), Some("t2"));
}

#[test]
fn holder_for_station_returns_none_when_holder_disconnected() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    sm.disconnect("t1");
    assert_eq!(sm.holder_for_station(&StationId("captain".into())), None);
}

#[test]
fn available_stations_returns_all_when_none_claimed() {
    let ship_config = test_ship_config();
    let sm = sm();
    let available = sm.available_stations(&ship_config);
    assert!(available.contains(&StationId("captain".into())));
    assert!(available.contains(&StationId("helm".into())));
    assert!(available.contains(&StationId("tactical".into())));
    assert!(available.contains(&StationId("repair".into())));
    assert!(available.contains(&StationId("sensors".into())));
    assert!(available.contains(&StationId("shields".into())));
    assert!(available.contains(&StationId("navigation".into())));
    assert!(available.contains(&StationId("power".into())));
    assert!(available.contains(&StationId("comms".into())));
}

#[test]
fn available_stations_excludes_claimed_stations() {
    let ship_config = test_ship_config();
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    let available = sm.available_stations(&ship_config);
    assert!(!available.contains(&StationId("captain".into())));
    assert!(available.contains(&StationId("helm".into())));
}

#[test]
fn available_stations_reappears_on_disconnect() {
    let ship_config = test_ship_config();
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    assert!(!sm
        .available_stations(&ship_config)
        .contains(&StationId("captain".into())));
    sm.disconnect("t1");
    assert!(sm
        .available_stations(&ship_config)
        .contains(&StationId("captain".into())));
}

#[test]
fn available_stations_excludes_disconnected_station_holders() {
    let ship_config = test_ship_config();
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    sm.set_station("t2", Some(StationId("helm".into())));
    assert!(!sm
        .available_stations(&ship_config)
        .contains(&StationId("captain".into())));
    assert!(!sm
        .available_stations(&ship_config)
        .contains(&StationId("helm".into())));
    sm.disconnect("t1");
    assert!(sm
        .available_stations(&ship_config)
        .contains(&StationId("captain".into())));
}

#[test]
fn station_for_token_returns_none_when_no_station() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert_eq!(sm.station_for_token("t1"), None);
}

#[test]
fn set_station_sets_and_clears() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    assert_eq!(
        sm.station_for_token("t1"),
        Some(&StationId("captain".into()))
    );
    sm.set_station("t1", None);
    assert_eq!(sm.station_for_token("t1"), None);
}

#[test]
fn set_last_rating_stores_and_clears() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(sm.players()[0].last_rating.is_none());
    sm.set_last_rating("t1", Some("Assisted".into()));
    assert_eq!(sm.players()[0].last_rating.as_deref(), Some("Assisted"));
    sm.set_last_rating("t1", None);
    assert!(sm.players()[0].last_rating.is_none());
}

#[test]
fn pending_rating_set_get_and_clear() {
    let mut sm = sm();
    let captain = StationId("captain".into());
    assert!(sm.pending_rating_for(&captain).is_none());

    sm.set_pending_rating(&captain, "Simplified".into());
    assert_eq!(
        sm.pending_rating_for(&captain),
        Some(&"Simplified".to_string())
    );
    assert_eq!(sm.pending_ratings().len(), 1);

    sm.clear_pending_rating(&captain);
    assert!(sm.pending_rating_for(&captain).is_none());
    assert!(sm.pending_ratings().is_empty());
}

#[test]
fn clear_all_pending_ratings_empties_the_map() {
    let mut sm = sm();
    sm.set_pending_rating(&StationId("captain".into()), "Simplified".into());
    sm.set_pending_rating(&StationId("tactical".into()), "Simplified".into());
    assert_eq!(sm.pending_ratings().len(), 2);

    sm.clear_all_pending_ratings();
    assert!(sm.pending_ratings().is_empty());
}

// ── Ready state ──────────────────────────────────────────────────

#[test]
fn set_ready_marks_player_ready() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(!sm.players()[0].ready);
    sm.set_ready("t1", true);
    assert!(sm.players()[0].ready);
    sm.set_ready("t1", false);
    assert!(!sm.players()[0].ready);
}

#[test]
fn set_ready_unknown_token_is_noop() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_ready("nonexistent", true); // should not panic
    assert!(!sm.players()[0].ready);
}

#[test]
fn all_ready_returns_false_when_zero_players() {
    let sm = sm();
    assert!(!sm.all_ready(), "zero players → all_ready must be false");
}

#[test]
fn all_ready_returns_true_when_single_player_ready() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_ready("t1", true);
    assert!(sm.all_ready());
}

#[test]
fn all_ready_returns_false_when_single_player_not_ready() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(!sm.all_ready());
}

#[test]
fn all_ready_requires_all_players_ready() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_ready("t1", true);
    assert!(!sm.all_ready(), "t2 not ready → all_ready false");
    sm.set_ready("t2", true);
    assert!(sm.all_ready(), "both ready → all_ready true");
}

#[test]
fn all_ready_ignores_disconnected_players() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_ready("t1", true);
    sm.disconnect("t2");
    assert!(
        sm.all_ready(),
        "disconnected player should not block all_ready"
    );
}

#[test]
fn readiness_tally_counts_stationless_crew_and_excludes_disconnected_rows() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.register("t3".into(), "Casey".into()).unwrap();
    sm.set_ready("t1", true);
    sm.set_ready("t2", true);
    sm.disconnect("t2");
    sm.set_spectator("t3", true);

    // t1 deliberately never selected a Station: presence, not seating,
    // decides whether crew participates in collective readiness.
    assert!(sm.station_for_token("t1").is_none());
    assert_eq!(
        sm.readiness_tally(),
        ReadinessTally {
            connected: 1,
            ready: 1
        }
    );
}

// ── Spectator role (issue #1105) ─────────────────────────────────────────

#[test]
fn set_and_is_spectator_roundtrip() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(!sm.is_spectator("t1"), "fresh player is not a spectator");
    sm.set_spectator("t1", true);
    assert!(sm.is_spectator("t1"));
    sm.set_spectator("t1", false);
    assert!(!sm.is_spectator("t1"));
    assert!(
        !sm.is_spectator("ghost"),
        "unknown token is not a spectator"
    );
}

#[test]
fn set_spectator_vacates_held_station() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    sm.set_spectator("t1", true);
    assert_eq!(
        sm.station_for_token("t1"),
        None,
        "becoming a spectator must vacate the seat (invariant)"
    );
    assert!(sm.is_spectator("t1"));
}

#[test]
fn set_station_clears_spectator_flag() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_spectator("t1", true);
    sm.set_station("t1", Some(StationId("helm".into())));
    assert!(
        !sm.is_spectator("t1"),
        "seating a spectator must clear the spectator role (invariant / #1106 seam)"
    );
    assert_eq!(sm.station_for_token("t1"), Some(&StationId("helm".into())));
}

#[test]
fn all_ready_ignores_spectators() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Watcher".into()).unwrap();
    sm.set_ready("t1", true);
    sm.set_spectator("t2", true);
    // t2 is a spectator and never readies; the lone crew member is ready.
    assert!(
        sm.all_ready(),
        "a spectator must not be counted in readiness or delay start"
    );
}

#[test]
fn all_ready_false_when_only_spectators() {
    let mut sm = sm();
    sm.register("t1".into(), "Watcher".into()).unwrap();
    sm.set_spectator("t1", true);
    assert!(
        !sm.all_ready(),
        "a spectator-only lobby must never auto-start"
    );
}

#[test]
fn spectator_flag_survives_disconnect_reconnect() {
    let mut sm = sm();
    sm.register("t1".into(), "Watcher".into()).unwrap();
    sm.set_spectator("t1", true);
    sm.disconnect("t1");
    // Record is never pruned; the flag rides on the Player record.
    assert!(sm.is_spectator("t1"), "flag persists across disconnect");
    sm.reconnect("t1");
    assert!(sm.is_spectator("t1"), "flag persists across reconnect");
    // And a reconnected spectator still cannot delay start.
    assert!(
        !sm.all_ready(),
        "reconnected spectator stays out of readiness"
    );
}

// ── AFK presence (issue #1104) ───────────────────────────────────────────

#[test]
fn set_and_is_afk_roundtrip() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(!sm.is_afk("t1"), "a fresh player is not AFK");
    sm.set_afk("t1", true);
    assert!(sm.is_afk("t1"));
    sm.set_afk("t1", false);
    assert!(!sm.is_afk("t1"));
    assert!(!sm.is_afk("ghost"), "unknown token is not AFK");
}

#[test]
fn register_defaults_afk_false() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(
        !sm.players()[0].afk,
        "a freshly registered player is not AFK"
    );
}

#[test]
fn set_afk_retains_the_held_station() {
    // Contrast `set_spectator`, which vacates the seat: AFK delegates the
    // Station's Systems WITHOUT relinquishing ownership (issue #1104 AC1).
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    sm.set_afk("t1", true);
    assert_eq!(
        sm.station_for_token("t1"),
        Some(&StationId("captain".into())),
        "entering AFK must keep the seat"
    );
    assert!(sm.is_afk("t1"));
}

#[test]
fn afk_flag_and_snapshot_survive_disconnect() {
    // AC5: an AFK holder that drops keeps both the presence flag and the
    // pre-AFK rating snapshot — disconnect must touch neither.
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    sm.set_afk("t1", true);
    sm.set_afk_prev_rating("t1", "Std".into());
    sm.disconnect("t1");
    assert!(sm.is_afk("t1"), "AFK flag persists across disconnect");
    assert_eq!(
        sm.afk_prev_rating_for("t1"),
        Some(&"Std".to_string()),
        "the pre-AFK snapshot survives the drop, un-clobbered by last_rating"
    );
    // The seat stays on the record for reconnect restore (occupancy is
    // gated on `connected` via `holder_for_station`, but the record keeps
    // the station).
    assert_eq!(
        sm.station_for_token("t1"),
        Some(&StationId("captain".into())),
        "the seat is retained on the record across the drop"
    );
    assert_eq!(
        sm.holder_for_station(&StationId("captain".into())),
        None,
        "but a disconnected holder does not occupy the seat"
    );
    sm.reconnect("t1");
    assert!(sm.is_afk("t1"), "AFK flag persists across reconnect");
}

#[test]
fn afk_prev_rating_set_restore_and_clear() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert_eq!(sm.afk_prev_rating_for("t1"), None, "no snapshot by default");
    sm.set_afk_prev_rating("t1", "Manual".into());
    assert_eq!(sm.afk_prev_rating_for("t1"), Some(&"Manual".to_string()));
    // A second entry replaces the first.
    sm.set_afk_prev_rating("t1", "Std".into());
    assert_eq!(sm.afk_prev_rating_for("t1"), Some(&"Std".to_string()));
    sm.clear_afk_prev_rating("t1");
    assert_eq!(sm.afk_prev_rating_for("t1"), None, "cleared after restore");
}

#[test]
fn reset_ready_clears_all() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_ready("t1", true);
    sm.set_ready("t2", true);
    sm.reset_ready();
    assert!(!sm.players()[0].ready);
    assert!(!sm.players()[1].ready);
}

#[test]
fn clear_all_stations_releases_every_seat_but_keeps_identity() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.register("t2".into(), "Bob".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    sm.set_station("t2", Some(StationId("helm".into())));
    sm.clear_all_stations();
    assert_eq!(sm.station_for_token("t1"), None);
    assert_eq!(sm.station_for_token("t2"), None);
    // Identity preserved: both players still registered and connected.
    assert_eq!(sm.players().len(), 2);
    assert_eq!(sm.players()[0].name, "Alice");
    assert!(sm.players()[0].connected);
    assert_eq!(sm.players()[1].name, "Bob");
    assert!(sm.players()[1].connected);
}

#[test]
fn register_sets_ready_false() {
    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    assert!(
        !sm.players()[0].ready,
        "newly registered player must have ready=false"
    );
}

// ── Accessibility eligibility side-map (issue #1103) ─────────────────

#[test]
fn eligibility_defaults_true_for_unknown_token() {
    let sm = sm();
    assert!(
        sm.is_eligible("nobody", &StationId("helm".into())),
        "an unreported token must default to eligible"
    );
}

#[test]
fn set_and_query_ineligible_station() {
    let mut sm = sm();
    let helm = StationId("helm".into());
    let tactical = StationId("tactical".into());
    sm.set_eligibility("t1", std::collections::HashSet::from([helm.clone()]));
    assert!(
        !sm.is_eligible("t1", &helm),
        "reported station is ineligible"
    );
    assert!(
        sm.is_eligible("t1", &tactical),
        "an unreported station stays eligible even for a reporting token"
    );
}

#[test]
fn set_eligibility_replaces_prior_report() {
    let mut sm = sm();
    let helm = StationId("helm".into());
    let tactical = StationId("tactical".into());
    sm.set_eligibility("t1", std::collections::HashSet::from([helm.clone()]));
    // Re-send with a different set (profile changed): the old one is replaced.
    sm.set_eligibility("t1", std::collections::HashSet::from([tactical.clone()]));
    assert!(sm.is_eligible("t1", &helm), "old report cleared");
    assert!(!sm.is_eligible("t1", &tactical), "new report applied");
}

#[test]
fn clear_all_eligibility_resets_to_default_true() {
    let mut sm = sm();
    let helm = StationId("helm".into());
    sm.set_eligibility("t1", std::collections::HashSet::from([helm.clone()]));
    assert!(!sm.is_eligible("t1", &helm));
    sm.clear_all_eligibility();
    assert!(
        sm.is_eligible("t1", &helm),
        "after ReturnToLobby every token is eligible again"
    );
}

/// Eligibility lives OFF `Player`: it must never appear in a serialized,
/// broadcast Player. Set an ineligible station for a token that holds a
/// seat, then encode the `PlayerJoined` broadcast and assert the ineligible
/// station id is nowhere in the wire form.
#[test]
fn eligibility_is_absent_from_the_serialized_player() {
    use crate::core::codec::JsonCodec;
    use crate::core::messages::ServerMessage;

    let mut sm = sm();
    sm.register("t1".into(), "Alice".into()).unwrap();
    sm.set_station("t1", Some(StationId("captain".into())));
    // t1 is ineligible for "science" — a station it does NOT hold.
    sm.set_eligibility(
        "t1",
        std::collections::HashSet::from([StationId("science".into())]),
    );

    let player = sm.players()[0].clone();
    let wire = JsonCodec
        .encode_server(&ServerMessage::PlayerJoined { player })
        .expect("encode");
    assert!(
        !wire.contains("science"),
        "the ineligible station leaked into the broadcast Player: {wire}"
    );
    assert!(
        !wire.contains("eligib"),
        "no eligibility field may appear on the broadcast Player: {wire}"
    );
}
