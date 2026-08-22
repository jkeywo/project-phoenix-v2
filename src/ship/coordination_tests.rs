use super::*;

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
fn station_for_target_resolves_console_keys_and_ownerless_targets() {
    let config = crate::ship::components::ShipConfigComponent::default().0;

    assert_eq!(
        station_for_target(&config, &crate::ship::system_registry::helm_station_key()),
        config
            .system(&crate::ship::system_registry::helm_steering_system_id())
            .and_then(|system| system.station.clone())
    );
    assert_eq!(
        station_for_target(
            &config,
            &crate::ship::system_registry::tactical_station_key()
        ),
        config.weapons_station()
    );
    assert_eq!(
        station_for_target(&config, &SystemId("not-a-real-target".into())),
        None
    );
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
/// `process_coordination_lag` (src/ship_plugin.rs).
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

// ── Human-seeking systems (issue #984) ─────────────────────────────────

/// OWNER-FIRST IS LOAD-BEARING: an earlier-authored human seat must not
/// steal the console from the system's own (later-authored) human owner.
#[test]
fn owner_human_wins_over_earlier_authored_human() {
    let seats = vec![
        seat("captain", ControlSource::Human, Some("alice")),
        seat("comms", ControlSource::Human, Some("bob")),
    ];
    let owner = StationId("comms".into());

    let found = seek_human_host(Some(&owner), &seats).unwrap();

    assert_eq!(found.station, StationId("comms".into()));
    assert_eq!(found.holder.as_deref(), Some("bob"));
}

/// No owner (an ownerless/NPC-style seek, or a caller that has not
/// resolved one) falls back to the first human in authored order.
#[test]
fn no_owner_falls_back_to_first_authored_human() {
    let seats = vec![
        seat("captain", ControlSource::Ai, None),
        seat("helm", ControlSource::Human, Some("hikaru")),
        seat("comms", ControlSource::Human, Some("bob")),
    ];

    let found = seek_human_host(None, &seats).unwrap();

    assert_eq!(found.station, StationId("helm".into()));
}

#[test]
fn no_humans_anywhere_yields_none() {
    let seats = vec![
        seat("captain", ControlSource::Ai, None),
        seat("helm", ControlSource::Ai, None),
    ];
    let owner = StationId("comms".into());

    assert!(seek_human_host(Some(&owner), &seats).is_none());
    assert!(seek_human_host(None, &seats).is_none());
}

/// A human-held station with nobody browser-connected is not a seek
/// candidate — same "control source vs. connected holder" distinction
/// `broadcast_to_ship` already draws.
#[test]
fn disconnected_human_seat_is_skipped() {
    let seats = vec![
        seat("captain", ControlSource::Human, None),
        seat("helm", ControlSource::Human, Some("hikaru")),
    ];

    let found = seek_human_host(None, &seats).unwrap();

    assert_eq!(found.station, StationId("helm".into()));
}

/// The owner seat is backfilled (AI) — not human-and-connected — so the
/// seek continues past it to the next human in authored order.
#[test]
fn owner_is_ai_falls_through_to_later_human() {
    let seats = vec![
        seat("comms", ControlSource::Ai, Some("stale-token")),
        seat("captain", ControlSource::Human, Some("alice")),
    ];
    let owner = StationId("comms".into());

    let found = seek_human_host(Some(&owner), &seats).unwrap();

    assert_eq!(found.station, StationId("captain".into()));
    assert_eq!(found.holder.as_deref(), Some("alice"));
}

// ── Authored seek order (issue #984) ──────────────────────────────────

/// The destroyer's shape: a bridge where the system's owner (Tactical) is
/// empty and three other seats are crewed. The DERIVED order would hand the
/// system to the earliest authored seat; the AUTHORED order hands it to
/// Engineering.
fn destroyer_bridge() -> Vec<ShipSeat> {
    vec![
        seat("captain", ControlSource::Human, Some("alice")),
        seat("helm", ControlSource::Human, Some("hikaru")),
        seat("tactical", ControlSource::Ai, None),
        seat("engineering", ControlSource::Human, Some("scotty")),
    ]
}

fn order(ids: &[&str]) -> Vec<StationId> {
    ids.iter().map(|s| StationId((*s).into())).collect()
}

#[test]
fn an_authored_order_is_walked_instead_of_the_derived_one() {
    let seats = destroyer_bridge();
    let owner = StationId("tactical".into());

    let derived = seek_human_host(Some(&owner), &seats).unwrap();
    assert_eq!(
        derived.station,
        StationId("captain".into()),
        "the derived walk takes the earliest authored human"
    );

    let authored = seek_human_host_in(
        Some(&owner),
        &order(&["tactical", "engineering", "captain", "helm"]),
        &seats,
    )
    .unwrap();
    assert_eq!(
        authored.station,
        StationId("engineering".into()),
        "John's ruling: Engineering is promoted ahead of the Captain"
    );
}

/// The authored order starts at the owner, so a crewed owner still wins —
/// the promotion only decides who gets it when Tactical is empty.
#[test]
fn an_authored_order_still_prefers_a_crewed_owner() {
    let mut seats = destroyer_bridge();
    seats[2] = seat("tactical", ControlSource::Human, Some("chang"));

    let found = seek_human_host_in(
        Some(&StationId("tactical".into())),
        &order(&["tactical", "engineering", "captain", "helm"]),
        &seats,
    )
    .unwrap();

    assert_eq!(found.station, StationId("tactical".into()));
}

/// The authored walk skips non-human seats exactly as the derived one does:
/// an order chooses among humans, it never conjures one.
#[test]
fn an_authored_order_with_no_humans_yields_none() {
    let seats = vec![
        seat("captain", ControlSource::Ai, Some("alice")),
        seat("tactical", ControlSource::Offline, None),
        seat("engineering", ControlSource::Human, None),
    ];

    assert!(
        seek_human_host_in(
            Some(&StationId("tactical".into())),
            &order(&["tactical", "engineering", "captain"]),
            &seats,
        )
        .is_none(),
        "backfilled, offline and disconnected seats are all skipped"
    );
}

/// THE DEFAULT IS THE OLD PATH, not a re-implementation of it. Every seat
/// arrangement a four-station hull can be in, against every owner it can
/// name: an empty `seek_order` must choose the identical seat.
#[test]
fn an_empty_seek_order_chooses_exactly_what_the_derived_walk_chooses() {
    let stations = ["captain", "helm", "tactical", "engineering"];
    let controls = [
        (ControlSource::Human, Some("crew")),
        (ControlSource::Human, None),
        (ControlSource::Ai, Some("crew")),
        (ControlSource::Offline, None),
    ];

    // Base 4 over 4 seats: 256 bridges, each tried against all four owners
    // and against no owner at all.
    for combo in 0..256u32 {
        let seats: Vec<ShipSeat> = stations
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let (control, holder) = controls[((combo >> (2 * i)) & 0b11) as usize];
                seat(id, control, holder)
            })
            .collect();

        let owners: Vec<Option<StationId>> = std::iter::once(None)
            .chain(stations.iter().map(|s| Some(StationId((*s).into()))))
            .collect();
        for owner in &owners {
            assert_eq!(
                seek_human_host_in(owner.as_ref(), &[], &seats),
                seek_human_host(owner.as_ref(), &seats),
                "bridge {combo:#010b}, owner {owner:?}"
            );
        }
    }
}

/// A name no seat answers to is stepped over, not treated as the end of the
/// walk. `ShipConfig::validate` makes this unreachable from parsed TOML;
/// the property is here so the pure function has no cliff for a caller that
/// assembles the two halves by hand.
#[test]
fn an_authored_order_steps_over_a_station_that_is_not_seated() {
    let seats = vec![
        seat("captain", ControlSource::Human, Some("alice")),
        seat("tactical", ControlSource::Ai, None),
    ];

    let found = seek_human_host_in(
        Some(&StationId("tactical".into())),
        &order(&["tactical", "science", "captain"]),
        &seats,
    )
    .unwrap();

    assert_eq!(found.station, StationId("captain".into()));
}

// ── seeking_seats (issue #984) ────────────────────────────────────────

/// A hull shaped like the destroyer's Tactical seat: one station owning a
/// mixed roster, plus a bare station whose ONLY system is human-seeking
/// (the battleship's `comms`/`navigation` shape).
fn seeking_config() -> crate::ship::config::ShipConfig {
    crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."

[[station]]
id = "tactical"
name = "Tactical"
description = "Guns."
rank = "Ltn."

[[station]]
id = "comms"
name = "Comms"
description = "Chatter."
rank = "Ens."

[[system]]
id = "captain"
kind = "captain"
station = "captain"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"

[[system]]
id = "navigation"
kind = "navigation"
station = "tactical"
human_seeking = true

[[system]]
id = "comms"
kind = "comms"
station = "comms"
human_seeking = true
"#,
        &["captain", "tactical_radar", "navigation", "comms"],
    )
    .unwrap()
}

fn ratings(pairs: &[(&str, &str)]) -> HashMap<StationId, String> {
    pairs
        .iter()
        .map(|(s, r)| (StationId((*s).into()), (*r).to_string()))
        .collect()
}

/// The fixpoint guard: a seeking system's own `ControlSource` — which this
/// very resolution WROTE last tick — must not be readable as evidence that
/// its host station is crewed. Here `navigation` lives on Tactical and is
/// `Human` because the seek put it there; Tactical's real roster is
/// backfilled, so the seat must still read `Ai`.
#[test]
fn a_seeking_systems_own_source_is_not_evidence_about_its_host_seat() {
    let config = seeking_config();
    let mut resolver = ControlSourceResolver::new();
    resolver.set(SystemId("tactical-radar".into()), ControlSource::Ai);
    resolver.set(SystemId("navigation".into()), ControlSource::Human);
    resolver.set(SystemId("comms".into()), ControlSource::Human);

    let seats = seeking_seats(&config, &resolver, &ratings(&[]), |_| {
        Some("someone".to_string())
    });

    let tactical = seats
        .iter()
        .find(|s| s.station == StationId("tactical".into()))
        .unwrap();
    assert_eq!(
        tactical.control,
        ControlSource::Ai,
        "only `tactical-radar` counts toward the Tactical seat; folding the \
         seek's own write back in would latch the console to itself"
    );
}

/// A station whose entire roster is human-seeking has no rating evidence
/// left to reduce, so the seat is decided by its ACTIVE RATING. This is the
/// battleship's `comms` and `navigation` stations exactly.
#[test]
fn a_station_owning_only_seeking_systems_is_decided_by_its_rating() {
    let config = seeking_config();
    let resolver = ControlSourceResolver::new();
    let comms = StationId("comms".into());

    let manned = seeking_seats(
        &config,
        &resolver,
        &ratings(&[("comms", "Standard")]),
        |_| Some("bob".to_string()),
    );
    assert_eq!(
        manned.iter().find(|s| s.station == comms).unwrap().control,
        ControlSource::Human,
        "a rated, connected Comms officer must be able to host their own console"
    );

    let backfilled = seeking_seats(
        &config,
        &resolver,
        &ratings(&[("comms", crate::ship::rating::BACKFILL_RATING)]),
        |_| Some("bob".to_string()),
    );
    assert_eq!(
        backfilled
            .iter()
            .find(|s| s.station == comms)
            .unwrap()
            .control,
        ControlSource::Ai,
        "an officer who asked the AI to run their station does not get the \
         console pushed back at them"
    );

    let unrated = seeking_seats(&config, &resolver, &ratings(&[]), |_| {
        Some("bob".to_string())
    });
    assert_eq!(
        unrated.iter().find(|s| s.station == comms).unwrap().control,
        ControlSource::Human,
        "an explicit Backfill is the ONLY thing that reads as automation — a \
         station with no rating recorded and a person sitting at it is a human seat"
    );

    let empty_seat = seeking_seats(&config, &resolver, &ratings(&[]), |_| None);
    assert_eq!(
        empty_seat
            .iter()
            .find(|s| s.station == comms)
            .unwrap()
            .control,
        ControlSource::Offline,
        "…and with nobody connected it is nobody's seat at all"
    );
}

/// Issue #1104 AC3: when a directly-held seat's holder steps AFK, their
/// Station is Backfilled — every owned System delegated to AI. `seeking_seats`
/// must then read that seat as `Ai`, not `Human`, so a human-seeking System's
/// seek walks past it exactly as it walks past a disconnected seat. No AFK
/// parameter is needed here: the Backfill the AFK entry applies is precisely
/// what excludes the seat, and this pins that it does even while the AFK
/// player stays connected and keeps the seat.
#[test]
fn an_afk_backfilled_seat_is_not_offered_as_a_human_host() {
    let config = seeking_config();
    let mut resolver = ControlSourceResolver::new();
    // AFK entry Backfilled the captain seat: its owned `captain` system is Ai.
    resolver.set(SystemId("captain".into()), ControlSource::Ai);

    let seats = seeking_seats(
        &config,
        &resolver,
        &ratings(&[("captain", crate::ship::rating::BACKFILL_RATING)]),
        // The AFK player is still CONNECTED and still holds the seat.
        |station| (station.0 == "captain").then(|| "afk-captain".to_string()),
    );

    let captain = seats
        .iter()
        .find(|s| s.station == StationId("captain".into()))
        .unwrap();
    assert_eq!(
        captain.control,
        ControlSource::Ai,
        "a Backfilled (AFK) seat reads Ai — a human-seeking seek skips it"
    );
    assert_eq!(
        captain.holder.as_deref(),
        Some("afk-captain"),
        "AFK retains the seat even though it now hosts nobody"
    );
}

#[test]
fn a_directly_held_human_seeking_station_can_host_a_legacy_seek_without_nesting() {
    let mut config = seeking_config();
    let comms = config
        .stations
        .iter_mut()
        .find(|station| station.id.0 == "comms")
        .expect("fixture has a Comms Station");
    comms.human_seeking = true;

    let connected = seeking_seats(
        &config,
        &ControlSourceResolver::new(),
        &ratings(&[("comms", "Standard")]),
        |station| (station.0 == "comms").then(|| "bob".to_string()),
    );
    assert_eq!(
        connected
            .iter()
            .find(|seat| seat.station.0 == "comms")
            .unwrap()
            .control,
        ControlSource::Human,
        "a connected direct holder is an eligible legacy-seek destination"
    );

    let visiting_only = seeking_seats(
        &config,
        &ControlSourceResolver::new(),
        &ratings(&[("comms", "Standard")]),
        |_| None,
    );
    assert_eq!(
        visiting_only
            .iter()
            .find(|seat| seat.station.0 == "comms")
            .unwrap()
            .control,
        ControlSource::Offline,
        "presentation as a visiting tab supplies no direct Session holder and must not nest"
    );

    let delegated = seeking_seats(
        &config,
        &ControlSourceResolver::new(),
        &ratings(&[("comms", crate::ship::rating::BACKFILL_RATING)]),
        |station| (station.0 == "comms").then(|| "bob".to_string()),
    );
    assert_eq!(
        delegated
            .iter()
            .find(|seat| seat.station.0 == "comms")
            .unwrap()
            .control,
        ControlSource::Offline,
        "an AI-operated human-seeking Station is not a human host"
    );
}

/// Seats come back in AUTHORED station order — the property `seek_human_host`
/// and two lockstep peers both depend on.
#[test]
fn seeking_seats_are_returned_in_authored_station_order() {
    let config = seeking_config();
    let resolver = ControlSourceResolver::new();
    let seats = seeking_seats(&config, &resolver, &ratings(&[]), |_| None);
    assert_eq!(
        seats
            .iter()
            .map(|s| s.station.0.as_str())
            .collect::<Vec<_>>(),
        vec!["captain", "tactical", "comms"],
    );
}

// ── Lag queue ─────────────────────────────────────────────────────────

#[test]
fn queue_starts_empty() {
    let queue = CoordinationLagQueue::new();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}

#[test]
fn enqueue_adds_message() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        sender_label: String::new(),
        due_time: 10.0,
    });
    assert_eq!(queue.len(), 1);
    assert!(!queue.is_empty());
}

#[test]
fn due_messages_returns_nothing_before_deadline() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        sender_label: String::new(),
        due_time: 10.0,
    });
    let due = queue.due_messages(5.0);
    assert!(due.is_empty());
    assert_eq!(queue.len(), 1);
}

#[test]
fn due_messages_returns_ready_at_deadline() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        sender_label: String::new(),
        due_time: 10.0,
    });
    let due = queue.due_messages(10.0);
    assert_eq!(due.len(), 1);
    assert!(queue.is_empty());
}

#[test]
fn due_messages_returns_ready_after_deadline() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "test".into(),
        },
        sender_label: String::new(),
        due_time: 10.0,
    });
    let due = queue.due_messages(15.0);
    assert_eq!(due.len(), 1);
    assert!(queue.is_empty());
}

#[test]
fn due_messages_respects_per_message_deadlines() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "first".into(),
        },
        sender_label: String::new(),
        due_time: 1.0,
    });
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "early".into(),
        },
        sender_label: String::new(),
        due_time: 5.0,
    });
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Advisory {
            message: "late".into(),
        },
        sender_label: String::new(),
        due_time: 15.0,
    });

    let due = queue.due_messages(10.0);
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
fn sender_origin_is_captured_at_enqueue_time() {
    let mut queue = CoordinationLagQueue::new();
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: SystemId("helm".into()),
        payload: CoordinationPayload::Alert {
            title: "AI Alert".into(),
            body: "Incoming threat detected.".into(),
        },
        sender_label: "AI Tactical".into(),
        due_time: 5.0,
    });
    let due = queue.due_messages(10.0);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].sender_origin, ControlSource::Ai);
    assert_eq!(due[0].sender_label, "AI Tactical");
}

#[test]
fn target_is_resolved_live_at_delivery_time() {
    let mut queue = CoordinationLagQueue::new();
    let target = SystemId("red-alert".into());
    queue.enqueue(QueuedCoordination {
        sender_origin: ControlSource::Ai,
        target: target.clone(),
        payload: CoordinationPayload::Alert {
            title: "test".into(),
            body: "body".into(),
        },
        sender_label: String::new(),
        due_time: 1.0,
    });
    let due = queue.due_messages(2.0);
    assert_eq!(due[0].target, target);
}

/// Channel-3 addresses crew by STATION, not by the fine system: a message
/// whose target is the `navigation` SYSTEM must name the station that OWNS
/// navigation (here `tactical`), rendered as that station's display id — the
/// station ALONE, never a `station.tactical.navigation` composite, and never
/// the bare `chatter.sender.navigation` system label (Task 2/3).
#[test]
fn station_addressee_names_the_owning_station_not_the_system() {
    let config = seeking_config();

    // A system that belongs to a station → that station's display id.
    assert_eq!(
        station_addressee_label(
            &config,
            &crate::ship::system_registry::navigation_system_id()
        ),
        "station.tactical.name",
        "navigation is owned by the tactical station on this hull, so it \
         addresses as the STATION alone"
    );

    // A station-level target key → the same station id, resolved once.
    assert_eq!(
        station_addressee_label(&config, &SystemId("comms".into())),
        "station.comms.name"
    );

    // An ownerless / unknown target → the ship's Core, never a bare system.
    assert_eq!(
        station_addressee_label(&config, &SystemId("repair".into())),
        CHATTER_ADDRESSEE_CORE,
        "a target no station owns is addressed to Core, not to a system name"
    );

    // The resolved id carries no system suffix in either shape.
    for id in [
        station_addressee_label(
            &config,
            &crate::ship::system_registry::navigation_system_id(),
        ),
        station_addressee_label(&config, &SystemId("comms".into())),
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
human_seeking = true
host_order = ["shields", "repair"]
visiting_rating = "Visit"
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
human_seeking = true
host_order = ["sensors"]
visiting_rating = "Visit"
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
station = "power"
[[system]]
id = "shields-system"
kind = "test"
station = "shields"
"#,
        &["test"],
    )
    .unwrap()
}

#[test]
fn complete_station_prefers_direct_then_ordered_fallback_then_ai() {
    let config = visiting_config();
    let power = config.station(&StationId("power".into())).unwrap();
    let no_floor = std::collections::BTreeSet::new();
    let direct = resolve_visiting_station(
        &config,
        power,
        |id| id.0 == "power" || id.0 == "repair",
        &no_floor,
    );
    assert_eq!(direct.host, Some(StationId("power".into())));

    let visiting = resolve_visiting_station(
        &config,
        power,
        |id| id.0 == "repair" || id.0 == "sensors",
        &no_floor,
    );
    assert_eq!(visiting.host, Some(StationId("repair".into())));

    let exhausted = resolve_visiting_station(&config, power, |id| id.0 == "sensors", &no_floor);
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
    let power = config.station(&StationId("power".into())).unwrap();
    let no_floor = std::collections::BTreeSet::new();

    let held = |id: &StationId| id.0 == "shields" || id.0 == "repair";
    let eligible_for_power = |id: &StationId| id.0 != "shields";
    let assignment = resolve_visiting_station(
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
    let none_eligible = resolve_visiting_station(&config, power, |_id| false, &no_floor);
    assert_eq!(none_eligible.host, None);
    assert_eq!(none_eligible.rating, crate::ship::rating::BACKFILL_RATING);
}

#[test]
fn generic_power_repair_and_shields_sensors_chains_need_no_band_d_hull() {
    let config = visiting_config();
    let none = std::collections::BTreeSet::new();
    for (visitor, expected) in [("power", "repair"), ("shields", "sensors")] {
        let station = config.station(&StationId(visitor.into())).unwrap();
        let assignment = resolve_visiting_station(&config, station, |id| id.0 == expected, &none);
        assert_eq!(assignment.host, Some(StationId(expected.into())));
    }
}

#[test]
fn scenario_floor_raises_the_authored_visiting_rating() {
    let config = visiting_config();
    let power = config.station(&StationId("power".into())).unwrap();
    let floor = std::iter::once(SystemId("power-system".into())).collect();
    let assignment = resolve_visiting_station(&config, power, |id| id.0 == "repair", &floor);
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
human_seeking = true
host_order = ["captain"]
visiting_rating = "Simplified"
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
station = "navigation"
"#,
        &["navigation"],
    )
    .unwrap();
    let navigation = config.station(&StationId("navigation".into())).unwrap();
    let floor = std::iter::once(SystemId("navigation".into())).collect();
    let assignment = resolve_visiting_station(&config, navigation, |id| id.0 == "captain", &floor);
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
    let power = config.station(&StationId("power".into())).unwrap();
    let floor = std::iter::once(SystemId("power-system".into())).collect();
    let assignment = resolve_visiting_station(&config, power, |id| id.0 == "repair", &floor);
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
    let power = config.station(&StationId("power".into())).unwrap();
    let directly_held = resolve_visiting_station(
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

    let shields_is_only_visiting = resolve_visiting_station(
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
    let power = config.station(&StationId("power".into())).unwrap();
    let none = std::collections::BTreeSet::new();

    // Shields officer seated: power visits shields' authored host order
    // — host_order = ["shields", "repair"], shields first and held.
    let before = resolve_visiting_station(
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
    let after = resolve_visiting_station(&config, power, |id| id.0 == "repair", &none);
    assert_eq!(
        after.host,
        Some(StationId("repair".into())),
        "losing the current host relocates the visiting Station to the next authored one",
    );
}
