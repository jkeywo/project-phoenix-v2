use super::*;
use crate::ship::control_source::ControlSourceResolver;
use std::collections::HashMap;

fn station_address(id: &str) -> CoordinationAddress {
    CoordinationAddress::Station(StationId(id.to_string()))
}

// ── Routing matrix ────────────────────────────────────────────────────

#[test]
fn target_ai_always_consumes_regardless_of_sender() {
    assert_eq!(
        route_coordination(ControlSource::Ai, ControlSource::Ai),
        DeliverAction::Consume
    );
    assert_eq!(
        route_coordination(ControlSource::Human, ControlSource::Ai),
        DeliverAction::Consume
    );
}

#[test]
fn target_human_sender_human_suppresses() {
    assert_eq!(
        route_coordination(ControlSource::Human, ControlSource::Human),
        DeliverAction::Suppress
    );
}

#[test]
fn address_for_system_resolves_owning_station_and_never_widens_unknown_to_ship() {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_battleship.toml")
            .expect("the shipped battleship composes")
            .ship_config
            .expect("the shipped battleship declares a ship config");

    assert_eq!(
        address_for_system(
            &config,
            &crate::ship::system_registry::helm_steering_system_id()
        ),
        config
            .system(&crate::ship::system_registry::helm_steering_system_id())
            .and_then(|system| config.system_home_station(system).cloned())
            .map(CoordinationAddress::Station)
    );
    assert_eq!(
        address_for_system(
            &config,
            &crate::ship::system_registry::tactical_radar_system_id()
        ),
        config.weapons_station().map(CoordinationAddress::Station)
    );
    assert_eq!(
        address_for_system(&config, &SystemId("not-a-real-system".into())),
        None
    );

    let shields_station = config
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
        .and_then(|system| config.system_home_station(system).cloned())
        .expect("the default player hull stations its shield arcs");
    assert_eq!(
        address_for_system_kind(&config, crate::ship::system_registry::SHIELD_ARC_KIND),
        Some(CoordinationAddress::Station(shields_station))
    );
    assert_eq!(address_for_system_kind(&config, "not_a_kind"), None);

    let mut split_kind = config.clone();
    let second_arc = split_kind
        .systems
        .iter_mut()
        .filter(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
        .nth(1)
        .expect("the default hull has multiple shield arcs");
    second_arc.console = Some(phoenix_model::identity::ConsoleId("helm".into()));
    assert_eq!(
        address_for_system_kind(&split_kind, crate::ship::system_registry::SHIELD_ARC_KIND),
        None,
        "an ambiguous kind owner is rejected, never widened to the Ship"
    );
}

#[test]
fn shipped_hulls_resolve_coordination_from_authored_topology_not_literal_station_names() {
    for stem in [
        "alliance_battleship",
        "alliance_cruiser",
        "alliance_destroyer",
        "alliance_courier",
    ] {
        let config =
            crate::test_fixtures::load_entity_config(&format!("assets/entities/{stem}.toml"))
                .unwrap_or_else(|error| panic!("{stem} must compose: {error}"))
                .ship_config
                .unwrap_or_else(|| panic!("{stem} must declare a ship config"));

        assert_eq!(
            address_for_system(
                &config,
                &crate::ship::system_registry::tactical_radar_system_id()
            ),
            config.weapons_station().map(CoordinationAddress::Station),
            "{stem}: Tactical address follows the Station that owns the radar"
        );
        assert_eq!(
            address_for_system(
                &config,
                &crate::ship::system_registry::helm_steering_system_id()
            ),
            config
                .system(&crate::ship::system_registry::helm_steering_system_id())
                .and_then(|system| config.system_home_station(system).cloned())
                .map(CoordinationAddress::Station),
            "{stem}: Helm address follows authored ownership"
        );

        let shield_owner = config
            .systems
            .iter()
            .find(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
            .and_then(|system| config.system_home_station(system).cloned());
        assert_eq!(
            address_for_system_kind(&config, crate::ship::system_registry::SHIELD_ARC_KIND),
            shield_owner.map(CoordinationAddress::Station),
            "{stem}: multi-arc Shields resolves its real Station (including composite Stations)"
        );
    }
}

#[test]
fn target_human_sender_ai_shows_popup() {
    assert_eq!(
        route_coordination(ControlSource::Ai, ControlSource::Human),
        DeliverAction::Popup
    );
}

/// #673: a Channel-3 message targeting a damage-disabled human console
/// must resolve via `policy_for` (which honours `offline_systems`), not
/// the raw `source_for`. A damage-disabled console can neither operate
/// AI nor accept human input, so delivery must be treated as Consume,
/// never Popup — mirroring the resolution logic in
/// `process_coordination_lag` (crates/phoenix-simulation/src/ship_plugin.rs).
#[test]
fn target_damage_disabled_human_console_is_not_a_popup_candidate() {
    let mut resolver = ControlSourceResolver::new();
    let helm = SystemId("helm".into());

    // Station rating says Human, but damage has taken the console offline.
    resolver.set(helm.clone(), ControlSource::Human);
    resolver.set_offline(helm.clone(), true);

    let target_policy = resolver.policy_for(&helm);
    assert!(!target_policy.operate_ai && !target_policy.accept_human_input);

    // The caller must treat this as Consume rather than routing through
    // route_coordination with the raw (stale) ControlSource::Human, which
    // would incorrectly produce a Popup.
    let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
        DeliverAction::Consume
    } else {
        route_coordination(ControlSource::Ai, resolver.source_for(&helm))
    };
    assert_eq!(action, DeliverAction::Consume);
}

// ── Ship-wide broadcast (issue #879) ──────────────────────────────────

fn seat(station: &str, control: ControlSource, holder: Option<&str>) -> ShipSeat {
    ShipSeat {
        station: StationId(station.into()),
        control,
        holder: holder.map(|h| h.to_string()),
    }
}

fn human_policy() -> ControlTickPolicy {
    crate::ship::control_source::control_tick_policy(ControlSource::Human)
}

fn ai_policy() -> ControlTickPolicy {
    crate::ship::control_source::control_tick_policy(ControlSource::Ai)
}

fn offline_policy() -> ControlTickPolicy {
    crate::ship::control_source::control_tick_policy(ControlSource::Offline)
}

#[test]
fn a_station_with_no_systems_is_an_offline_seat() {
    assert_eq!(seat_control_source(&[]), ControlSource::Offline);
}

#[test]
fn a_fully_backfilled_station_is_an_ai_seat() {
    assert_eq!(
        seat_control_source(&[ai_policy(), ai_policy(), ai_policy()]),
        ControlSource::Ai
    );
}

/// One live human console on the station is enough: the question the router
/// asks is "can a person read this".
#[test]
fn one_human_system_makes_the_whole_seat_human() {
    assert_eq!(
        seat_control_source(&[ai_policy(), human_policy(), offline_policy()]),
        ControlSource::Human
    );
}

#[test]
fn a_damage_disabled_station_is_an_offline_seat() {
    assert_eq!(
        seat_control_source(&[offline_policy(), offline_policy()]),
        ControlSource::Offline
    );
}

/// AC: a backfilled seat's advisory reaches EVERY human seat on the source
/// ship, and no AI or offline seat.
#[test]
fn a_backfilled_senders_advisory_reaches_every_human_seat_and_no_other() {
    let seats = vec![
        seat("captain", ControlSource::Human, Some("alice")),
        seat("helm", ControlSource::Human, Some("hikaru")),
        seat("tactical", ControlSource::Ai, None),
        seat("shields", ControlSource::Ai, Some("stale-token")),
        seat("power", ControlSource::Offline, Some("scotty")),
    ];

    let recipients = broadcast_to_ship(ControlSource::Ai, &seats);

    let stations: Vec<&str> = recipients.iter().map(|s| s.station.0.as_str()).collect();
    assert_eq!(
        stations,
        vec!["captain", "helm"],
        "every human seat, in authored order — and neither the backfilled \
         Tactical/Shields nor the offline Power, whatever token they carry"
    );
    let tokens: Vec<&str> = recipients
        .iter()
        .map(|s| s.holder.as_deref().unwrap())
        .collect();
    assert_eq!(tokens, vec!["alice", "hikaru"]);
}

/// The seat semantics are about the CONTROL SOURCE, not about who happens
/// to be logged in: a human-held station whose holder has dropped is still
/// a human seat, it simply has nobody to deliver to.
#[test]
fn a_human_seat_with_no_connected_holder_receives_nothing() {
    let seats = vec![seat("helm", ControlSource::Human, None)];
    assert!(broadcast_to_ship(ControlSource::Ai, &seats).is_empty());
}

/// A complete auxiliary/visiting Station is presented inside its host's
/// already-connected console; it is not a second Session seat. Ship fan-out
/// therefore reaches the host once and does not duplicate the popup for the
/// holder-less visiting identity.
#[test]
fn a_visiting_station_does_not_duplicate_its_hosts_ship_popup() {
    let seats = vec![
        seat("comms", ControlSource::Human, Some("uhura")),
        seat("navigation", ControlSource::Human, None),
    ];

    let recipients = broadcast_to_ship(ControlSource::Ai, &seats);

    assert_eq!(recipients.len(), 1);
    assert_eq!(recipients[0].station, StationId("comms".into()));
    assert_eq!(recipients[0].holder.as_deref(), Some("uhura"));
}

/// The broadcast is `route_coordination` applied per seat, so it inherits
/// the human→human `Suppress`: two officers on the same bridge do not need
/// popups about each other.
#[test]
fn a_human_senders_broadcast_is_suppressed_at_every_seat() {
    let seats = vec![
        seat("captain", ControlSource::Human, Some("alice")),
        seat("helm", ControlSource::Human, Some("hikaru")),
    ];
    assert!(
        broadcast_to_ship(ControlSource::Human, &seats).is_empty(),
        "human sender + human seats is Suppress, as it has always been"
    );
}

#[test]
fn a_ship_with_no_human_seats_broadcasts_to_nobody() {
    let seats = vec![
        seat("captain", ControlSource::Ai, None),
        seat("helm", ControlSource::Ai, None),
    ];
    assert!(broadcast_to_ship(ControlSource::Ai, &seats).is_empty());
}

// ── Lag queue ─────────────────────────────────────────────────────────

fn test_presentation() -> crate::core::messages::CoordinationPresentation {
    crate::core::messages::CoordinationPresentation::titled("test.coordination.title")
}

#[test]
fn queue_starts_empty() {
    let queue = CoordinationLagQueue::new();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}

#[test]
fn authored_coordination_lag_converts_to_the_expected_whole_ticks() {
    assert_eq!(coordination_lag_ticks(0.0, 60.0), 0, "zero is same-tick");
    assert_eq!(
        coordination_lag_ticks(0.001, 60.0),
        1,
        "a positive sub-tick delay waits one tick"
    );
    assert_eq!(coordination_lag_ticks(0.6, 30.0), 18);
    assert_eq!(coordination_lag_ticks(0.6, 60.0), 36);
    assert_eq!(coordination_lag_ticks(2.0, 30.0), 60);
    assert_eq!(coordination_lag_ticks(2.0, 60.0), 120);
    assert_eq!(
        coordination_lag_ticks(0.61, 60.0),
        37,
        "a non-integral delay rounds up to the first observable tick"
    );
}

#[test]
fn enqueue_adds_message() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 10,
    });
    assert_eq!(queue.len(), 1);
    assert!(!queue.is_empty());
}

#[test]
fn due_messages_returns_nothing_before_deadline() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 10,
    });
    let due = queue.due_messages(5);
    assert!(due.is_empty());
    assert_eq!(queue.len(), 1);
}

#[test]
fn due_messages_returns_ready_at_deadline() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 10,
    });
    let due = queue.due_messages(10);
    assert_eq!(due.len(), 1);
    assert!(queue.is_empty());
}

#[test]
fn due_messages_returns_ready_after_deadline() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 10,
    });
    let due = queue.due_messages(15);
    assert_eq!(due.len(), 1);
    assert!(queue.is_empty());
}

#[test]
fn due_messages_respects_per_message_deadlines() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "first".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 1,
    });
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "early".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 5,
    });
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Advisory {
            message: "late".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 15,
    });

    let due = queue.due_messages(10);
    assert_eq!(due.len(), 2);
    assert_eq!(
        due[0].payload,
        CoordinationPayload::Advisory {
            message: "first".into()
        }
    );
    assert_eq!(
        due[1].payload,
        CoordinationPayload::Advisory {
            message: "early".into()
        }
    );
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue.0[0].payload,
        CoordinationPayload::Advisory {
            message: "late".into()
        }
    );
}

#[test]
fn equal_tick_messages_are_delivered_in_enqueue_order() {
    let mut queue = CoordinationLagQueue::new();
    for message in ["first", "second"] {
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            address: station_address("helm"),
            payload: CoordinationPayload::Advisory {
                message: message.into(),
            },
            presentation: test_presentation(),
            sender_label: String::new(),
            due_tick: 5,
        });
    }

    let due = queue.due_messages(5);
    assert_eq!(
        due.iter()
            .map(|message| match &message.payload {
                CoordinationPayload::Advisory { message } => message.as_str(),
                _ => unreachable!("fixture queues Advisory payloads"),
            })
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn sender_origin_is_captured_at_enqueue_time() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: station_address("helm"),
        payload: CoordinationPayload::Alert {
            title: "AI Alert".into(),
            body: "Incoming threat detected.".into(),
        },
        presentation: test_presentation(),
        sender_label: "AI Tactical".into(),
        due_tick: 5,
    });
    let due = queue.due_messages(10);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].sender_origin, ControlSource::Ai);
    assert_eq!(due[0].sender_label, "AI Tactical");
}

#[test]
fn explicit_address_survives_the_delay_unchanged() {
    let mut queue = CoordinationLagQueue::new();
    let address = station_address("captain");
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        address: address.clone(),
        payload: CoordinationPayload::Alert {
            title: "test".into(),
            body: "body".into(),
        },
        presentation: test_presentation(),
        sender_label: String::new(),
        due_tick: 1,
    });
    let due = queue.due_messages(2);
    assert_eq!(due[0].address, address);
}

/// A producer resolves its fine System once, then the explicit address alone
/// supplies the delayed message's addressee label.
#[test]
fn coordination_addressee_names_station_or_ship_from_the_explicit_address() {
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "tactical"
name = "Tactical"
description = ""
rank = ""
[[console]]
id = "navigation-screen"
home_station = "tactical"
screen = "fixture.html"
[[system]]
id = "navigation"
kind = "navigation"
console = "navigation-screen"
"#,
        &["navigation"],
    )
    .unwrap();
    let navigation_address = address_for_system(
        &config,
        &crate::ship::system_registry::navigation_system_id(),
    )
    .expect("navigation has an owning Station");

    assert_eq!(
        coordination_addressee_label(&navigation_address),
        "station.tactical.name",
        "navigation resolves to its owning Station before enqueue"
    );
    assert_eq!(
        coordination_addressee_label(&station_address("comms")),
        "station.comms.name"
    );
    assert_eq!(
        coordination_addressee_label(&CoordinationAddress::Ship),
        CHATTER_ADDRESSEE_SHIP
    );

    for id in [
        coordination_addressee_label(&navigation_address),
        coordination_addressee_label(&station_address("comms")),
    ] {
        assert!(
            id.starts_with("station.") && id.ends_with(".name"),
            "a station addressee is a `station.<id>.name` id with no system \
             half, got {id}"
        );
    }
}

fn visiting_config() -> crate::ship::config::ShipConfig {
    crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "power"
name = "Power"
description = ""
rank = ""
[[station.rating]]
name = "Floor"
automated_systems = []
[[station.rating]]
name = "Visit"
automated_systems = ["power-system"]

[[station]]
id = "repair"
name = "Repair"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "shields"
name = "Shields"
description = ""
rank = ""
[[station.rating]]
name = "Visit"
automated_systems = []

[[station]]
id = "sensors"
name = "Sensors"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "power-system"
kind = "test"
console = "power"
[[system]]
id = "shields-system"
kind = "test"
console = "shields"

[[console]]
id = "power"
home_station = "power"
screen = "fixture.html"

human_seeking = true
host_order = ["shields", "repair"]
visiting_rating = "Visit"

[[console]]
id = "shields"
home_station = "shields"
screen = "fixture.html"

human_seeking = true
host_order = ["sensors"]
visiting_rating = "Visit"
"#,
        &["test"],
    )
    .unwrap()
}

#[test]
fn complete_station_prefers_direct_then_ordered_fallback_then_ai() {
    let config = visiting_config();
    let power = config
        .console(&crate::core::messages::ConsoleId("power".into()))
        .unwrap();
    let no_floor = std::collections::BTreeSet::new();
    let direct = resolve_console_for_test(
        &config,
        power,
        |id| id.0 == "power" || id.0 == "repair",
        &no_floor,
    );
    assert_eq!(direct.host, Some(StationId("power".into())));

    let visiting = resolve_console_for_test(
        &config,
        power,
        |id| id.0 == "repair" || id.0 == "sensors",
        &no_floor,
    );
    assert_eq!(visiting.host, Some(StationId("repair".into())));

    let exhausted = resolve_console_for_test(&config, power, |id| id.0 == "sensors", &no_floor);
    assert_eq!(
        exhausted.host, None,
        "an unrelated human is not an implicit fallback"
    );
    assert_eq!(exhausted.rating, crate::ship::rating::BACKFILL_RATING);
}

#[test]
fn an_ineligible_host_is_skipped_to_the_next_eligible_one() {
    // AC2 (issue #1103): the Bevy adapter passes `held && eligible` as
    // `is_directly_held`. Model shields as held-but-INELIGIBLE for the
    // visiting power station (composed predicate false) while repair is held
    // AND eligible: the walk skips shields — first in power's host_order —
    // and lands on repair, exactly as if shields were unheld.
    let config = visiting_config();
    let power = config
        .console(&crate::core::messages::ConsoleId("power".into()))
        .unwrap();
    let no_floor = std::collections::BTreeSet::new();

    let held = |id: &StationId| id.0 == "shields" || id.0 == "repair";
    let eligible_for_power = |id: &StationId| id.0 != "shields";
    let assignment = resolve_console_for_test(
        &config,
        power,
        |id| held(id) && eligible_for_power(id),
        &no_floor,
    );
    assert_eq!(
        assignment.host,
        Some(StationId("repair".into())),
        "an ineligible first host is skipped; the walk continues to the next eligible one"
    );

    // With every held candidate ineligible the composed predicate is false
    // everywhere, so the walk falls through to AI exactly as it would with no
    // human host at all — reason/settings never enter the resolver.
    let none_eligible = resolve_console_for_test(&config, power, |_id| false, &no_floor);
    assert_eq!(none_eligible.host, None);
    assert_eq!(none_eligible.rating, crate::ship::rating::BACKFILL_RATING);
}

#[test]
fn generic_power_repair_and_shields_sensors_chains_need_no_band_d_hull() {
    let config = visiting_config();
    let none = std::collections::BTreeSet::new();
    for (visitor, expected) in [("power", "repair"), ("shields", "sensors")] {
        let station = config
            .console(&crate::core::messages::ConsoleId(visitor.into()))
            .unwrap();
        let assignment = resolve_console_for_test(&config, station, |id| id.0 == expected, &none);
        assert_eq!(assignment.host, Some(StationId(expected.into())));
    }
}

#[test]
fn scenario_floor_raises_the_authored_visiting_rating() {
    let config = visiting_config();
    let power = config
        .console(&crate::core::messages::ConsoleId("power".into()))
        .unwrap();
    let floor = std::iter::once(SystemId("power-system".into())).collect();
    let assignment = resolve_console_for_test(&config, power, |id| id.0 == "repair", &floor);
    assert_eq!(assignment.rating, "Floor");
}

#[test]
fn scenario_floor_raises_simplified_toward_earlier_std_rung() {
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "navigation"
name = "Navigation"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []
[[station.rating]]
name = "Simplified"
automated_systems = ["navigation"]

[[station]]
id = "captain"
name = "Captain"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "navigation"
kind = "navigation"
console = "navigation"

[[console]]
id = "navigation"
home_station = "navigation"
screen = "fixture.html"

human_seeking = true
host_order = ["captain"]
visiting_rating = "Simplified"
"#,
        &["navigation"],
    )
    .unwrap();
    let navigation = config
        .console(&crate::core::messages::ConsoleId("navigation".into()))
        .unwrap();
    let floor = std::iter::once(SystemId("navigation".into())).collect();
    let assignment = resolve_console_for_test(&config, navigation, |id| id.0 == "captain", &floor);
    assert_eq!(assignment.host, Some(StationId("captain".into())));
    assert_eq!(assignment.rating, "Std");
}

#[test]
fn impossible_scenario_floor_refuses_human_host_and_falls_back_to_ai() {
    let mut config = visiting_config();
    let power = config
        .stations
        .iter_mut()
        .find(|station| station.id.0 == "power")
        .unwrap();
    for rating in &mut power.ratings {
        rating.automated_systems = vec![SystemId("power-system".into())];
    }
    let power = config
        .console(&crate::core::messages::ConsoleId("power".into()))
        .unwrap();
    let floor = std::iter::once(SystemId("power-system".into())).collect();
    let assignment = resolve_console_for_test(&config, power, |id| id.0 == "repair", &floor);
    assert_eq!(assignment.host, None);
    assert_eq!(assignment.rating, crate::ship::rating::BACKFILL_RATING);
}

#[test]
fn scenario_floor_vocabulary_resolves_station_families_and_system_kinds_per_hull() {
    let config = visiting_config();
    let selectors = vec!["power".to_string(), "shields".to_string()];
    let resolved = resolve_scenario_detail_floor(&config, &selectors);
    assert!(resolved.contains(&SystemId("power-system".into())));
    assert!(resolved.contains(&SystemId("shields-system".into())));
    assert!(!resolved.contains(&SystemId("repair-system".into())));
}

#[test]
fn directly_held_human_seeking_station_may_host_without_enabling_nesting() {
    let config = visiting_config();
    let power = config
        .console(&crate::core::messages::ConsoleId("power".into()))
        .unwrap();
    let directly_held = resolve_console_for_test(
        &config,
        power,
        |id| id.0 == "shields",
        &std::collections::BTreeSet::new(),
    );
    assert_eq!(
        directly_held.host,
        Some(StationId("shields".into())),
        "Station type does not disqualify an active direct holder"
    );

    let shields_is_only_visiting = resolve_console_for_test(
        &config,
        power,
        |id| id.0 == "repair",
        &std::collections::BTreeSet::new(),
    );
    assert_eq!(
        shields_is_only_visiting.host,
        Some(StationId("repair".into())),
        "a non-direct visiting candidate is skipped; resolution stays finite"
    );
}

/// Issue #1099 AC3: a disconnect relocates a human-seeking Station. The
/// resolver is a pure per-tick recompute keyed on which seats are directly
/// held; a holder dropping off (its seat no longer `is_directly_held`) moves
/// the visiting Station on to the next authored host without any state of
/// its own being reset. `holder_for_station` returning `None` for a
/// disconnected holder (see `lobby::session`) is what flips the input here.
#[test]
fn a_disconnect_relocates_a_human_seeking_station_to_the_next_host() {
    let config = visiting_config();
    let power = config
        .console(&crate::core::messages::ConsoleId("power".into()))
        .unwrap();
    let none = std::collections::BTreeSet::new();

    // Shields officer seated: power visits shields' authored host order
    // — host_order = ["shields", "repair"], shields first and held.
    let before = resolve_console_for_test(
        &config,
        power,
        |id| id.0 == "shields" || id.0 == "repair",
        &none,
    );
    assert_eq!(before.host, Some(StationId("shields".into())));

    // The shields holder disconnects: its seat is no longer directly held,
    // so the very same recompute relocates power on to `repair`, the next
    // authored host that is still held. Nothing about power's own state is
    // touched — the resolver reads the seat map and nothing else.
    let after = resolve_console_for_test(&config, power, |id| id.0 == "repair", &none);
    assert_eq!(
        after.host,
        Some(StationId("repair".into())),
        "losing the current host relocates the visiting Station to the next authored one",
    );
}

// Exercise the production Console resolver with no directly selected rating.
fn resolve_console_for_test(
    config: &crate::ship::config::ShipConfig,
    console: &phoenix_model::console::ConsoleConfig,
    available: impl Fn(&StationId) -> bool,
    floor: &std::collections::BTreeSet<SystemId>,
) -> phoenix_model::console::ConsoleHostSnapshot {
    crate::ship::console_ownership::resolve_assignment(
        config,
        console,
        available,
        &HashMap::new(),
        floor,
    )
}

#[test]
fn detached_shipped_navigation_keeps_its_localized_sender_when_addressing_helm() {
    for hull in ["alliance_destroyer", "alliance_cruiser"] {
        let config =
            crate::test_fixtures::load_entity_config(&format!("assets/entities/{hull}.toml"))
                .unwrap()
                .ship_config
                .unwrap();
        let navigation = crate::ship::system_registry::navigation_system_id();
        assert!(config
            .console_for_system(&navigation)
            .unwrap()
            .home_station
            .is_none());
        assert_eq!(
            system_addressee_label(&config, &navigation),
            "station.navigation.name"
        );
        let address = address_for_system(
            &config,
            &crate::ship::system_registry::helm_steering_system_id(),
        )
        .unwrap();
        assert_eq!(coordination_addressee_label(&address), "station.helm.name");
    }
}

#[test]
fn detached_sender_uses_authored_name_or_console_id_and_only_unowned_is_core() {
    let mut config: crate::ship::config::ShipConfig = toml::from_str(
        r#"
[[console]]
id="custom-display"
name="custom.localized.name"
[[system]]
id="capability"
kind="test"
console="custom-display"
"#,
    )
    .unwrap();
    let id = SystemId("capability".into());
    assert_eq!(
        system_addressee_label(&config, &id),
        "custom.localized.name"
    );
    config.consoles[0].name.clear();
    assert_eq!(system_addressee_label(&config, &id), "custom-display");
    config.systems[0].console = None;
    assert_eq!(system_addressee_label(&config, &id), CHATTER_ADDRESSEE_CORE);
    assert_eq!(
        system_addressee_label(&config, &SystemId("missing".into())),
        CHATTER_ADDRESSEE_CORE
    );
}

#[test]
fn movable_and_detached_console_addresses_survive_host_changes() {
    let mut config = visiting_config();
    let system = SystemId("power-system".into());
    let owner = config.console_for_system(&system).unwrap().id.clone();
    let expected = CoordinationAddress::Console(owner.clone());
    assert_eq!(address_for_system(&config, &system), Some(expected.clone()));
    let console = config
        .consoles
        .iter_mut()
        .find(|console| console.id == owner)
        .unwrap();
    console.home_station = None;
    assert_eq!(address_for_system(&config, &system), Some(expected.clone()));
    assert_eq!(
        address_for_system(&config, &SystemId("missing".into())),
        None
    );
    let encoded = serde_json::to_string(&expected).unwrap();
    assert_eq!(
        serde_json::from_str::<CoordinationAddress>(&encoded).unwrap(),
        expected
    );
}
