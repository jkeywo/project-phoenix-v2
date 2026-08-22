use super::*;
use crate::core::messages::*;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
use crate::world::server::tests::ai_trigger_test_app;
use crate::world::server::{broadcast_objective_summary, WorldContentRuntime};

// -- Test app -------------------------------------------------------------

#[derive(Resource, Default)]
pub(crate) struct Outbox(pub(crate) Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

pub(crate) fn comms_test_app() -> App {
    let mut app = App::new();
    // The Comms AI hosts read `AiHostEnv` (issue #1207); a fixture that runs
    // one must register its bare-`Res` context or panic at schedule build.
    crate::ai::host::register_ai_host_env(&mut app);
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .init_resource::<WorldContentRuntime>()
        .init_resource::<CommsRuntime>()
        .init_resource::<CommsInboxRes>()
        .init_resource::<ObjectiveManagerRes>()
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .add_message::<CommsChannel2Event>()
        .add_systems(
            FixedUpdate,
            (
                handle_hail,
                handle_respond_to_message,
                handle_clear_comms,
                handle_comms_channel2,
                update_comms_range_flags,
                broadcast_comms_state,
                broadcast_objective_summary,
            )
                .chain()
                .after(crate::server_app::AdmissionSet),
        )
        .add_systems(PostUpdate, collect);
    // One fixed step per update (issue #895): the chain above joins the
    // admission seam in `FixedUpdate`, and each harness tick steps it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(1),
    );
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        crate::core::messages::AdmittedCommands::default(),
        // The AUTHORED Comms console AI pair every shipped hull carries.
        // Since #885b stage 5d neither host has a synthesised fallback, so a
        // fixture whose subject is the AI answering (or being refused by)
        // the router has to attach the declarations a real hull writes.
        crate::console::comms::server::CommsResponseAiPolicy(
            crate::entities::authored_ai_pins::shipped_policy_toml("comms_response")
                .to_policy()
                .expect("the shipped Comms response policy decodes"),
        ),
        crate::console::comms::server::CommsTargetSelector {
            selector: crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail")
                .to_selector()
                .expect("the shipped Comms hail selector decodes"),
            power_rating: None,
        },
    ));
    app
}

pub(crate) fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

pub(crate) fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
    let mut msgs = app.world().resource::<Outbox>().0.clone();
    for (target, msg) in sim_entries {
        msgs.push(OutboundMessage {
            target,
            msg,
            delivery: crate::core::messages::DeliveryClass::Reliable,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    msgs
}

/// Set up a game in InProgress phase with a comms player and captain.
pub(crate) fn setup_game_with_comms(app: &mut App, station_uuid: &str) {
    // Register captain
    push_msg(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push_msg(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    // Register comms
    push_msg(
        app,
        "comms",
        ClientMessage::Identify {
            token: "comms".into(),
            name: "Uhura".into(),
        },
    );
    tick(app);
    push_msg(
        app,
        "comms",
        ClientMessage::SelectStation {
            station: "Comms".into(),
        },
    );
    tick(app);
    // Start game
    push_msg(app, "captain", ClientMessage::SetReady { ready: true });
    push_msg(app, "comms", ClientMessage::SetReady { ready: true });
    tick(app);

    // Seat the station as a hail contact directly, so these tests are
    // independent of both TOML loading and the entity-derived roster pass.
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .name_to_uuid
        .insert("starbase_alpha".into(), station_uuid.into());
    let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
    comms.contacts.push(CommsContact {
        uuid: station_uuid.into(),
        name: "Starbase Alpha".into(),
        in_range: true,
        is_urgent: false,
    });
    comms.needs_broadcast = true;
}

// -- Slice 7: range-aware comms broadcast ---------------------------------

#[test]
fn comms_state_marks_contact_out_of_range_when_ship_too_far() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "station-uuid-range-far";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    // Spawn the ship close to the station so the initial hail succeeds,
    // then move the station far away to verify the flag flips.
    let ship_entity = app
        .world_mut()
        .spawn((
            Ship,
            crate::server_app::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(100.0),
        ))
        .id();
    let station_entity = app
        .world_mut()
        .spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(100.0),
        ))
        .id();

    // Flush initial broadcast.
    let _ = tick(&mut app);

    // Seat a message from the station while it is in range. (It used to
    // arrive from a `[[comms]] on_hailed` template; issue #985 deleted that
    // front-end, and the stamp under test reads the inbox, not the source.)
    app.world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(CommsMessage::injected(
            "msg-range-far".into(),
            station_uuid.into(),
            "Starbase Alpha".into(),
            "Go ahead, Phoenix.".into(),
            Default::default(),
            vec![],
            "thread-range-far".into(),
            true,
            false,
        ));
    let _ = tick(&mut app);

    // Now move the station far away (combined range 200, distance 1000).
    let _ = ship_entity;
    if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
        e.insert(Transform::from_xyz(1000.0, 0.0, 0.0));
    }
    let out = tick(&mut app);

    let (messages, contacts) = out
        .iter()
        .find_map(|m| {
            if let ServerMessage::CommsState {
                messages, contacts, ..
            } = &m.msg
            {
                Some((messages.clone(), contacts.clone()))
            } else {
                None
            }
        })
        .expect("CommsState must be broadcast after range flip");

    let contact = contacts
        .iter()
        .find(|c| c.uuid == station_uuid)
        .expect("contact present");
    assert!(!contact.in_range, "contact should be out of range");
    assert_eq!(messages.len(), 1, "one hail message expected");
    assert!(
        !messages[0].sender_in_range,
        "sender_in_range must be false when station is far"
    );
}

/// Issue #786: `open_hails` must be pruned alongside `contacts` when the
/// target entity stops existing, or it grows monotonically and retains the
/// UUIDs of despawned entities. A LoadWorld → UnloadWorld → LoadWorld cycle
/// that re-registers the same authored UUID would otherwise leave that
/// contact permanently un-hailable: `candidate_fact(has_open_hail_thread)`
/// would still read 1 for a hail issued in a previous life of the world.
#[test]
fn despawning_a_hailed_entity_prunes_the_open_hail_record() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "station-uuid-open-hail-prune";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(100.0),
    ));
    let station_entity = app
        .world_mut()
        .spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(100.0),
        ))
        .id();
    let _ = tick(&mut app);

    push_msg(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
                target_uuid: station_uuid.into(),
            },
        },
    );
    let _ = tick(&mut app);
    assert!(
        app.world()
            .resource::<CommsRuntime>()
            .open_hails
            .contains(station_uuid),
        "the hail must be recorded while the target is live"
    );

    // The layer unloads (or the station is destroyed): its entity despawns.
    app.world_mut().despawn(station_entity);
    let _ = tick(&mut app);
    assert!(
        !app.world()
            .resource::<CommsRuntime>()
            .open_hails
            .contains(station_uuid),
        "a despawned entity's open-hail record must be pruned alongside its \
         contact and range flag"
    );
}

/// Issue #786 determinism: a `Hail` and a `ClearComms` landing in the SAME
/// tick must resolve the same way every run — clear wins, matching the inbox
/// semantics the two handlers share. `handle_clear_comms` is explicitly
/// `.after(handle_hail)`, so the outcome cannot depend on executor ordering.
#[test]
fn a_same_tick_clear_wins_over_a_hail() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "station-uuid-same-tick-clear";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(100.0),
    ));
    app.world_mut().spawn((
        EntityUuid(station_uuid.into()),
        Transform::from_xyz(50.0, 0.0, 0.0),
        CommsRange(100.0),
    ));
    let _ = tick(&mut app);

    push_msg(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
                target_uuid: station_uuid.into(),
            },
        },
    );
    push_msg(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::ClearComms,
        },
    );
    let _ = tick(&mut app);

    assert!(
        app.world().resource::<CommsRuntime>().open_hails.is_empty(),
        "clear must win over a same-tick hail: `handle_clear_comms` runs \
         after `handle_hail`, deterministically"
    );
}

#[test]
fn comms_state_marks_contact_in_range_when_ship_close() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "station-uuid-range-near";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid(station_uuid.into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        CommsRange(500.0),
    ));

    let _ = tick(&mut app);
    app.world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(CommsMessage::injected(
            "msg-range-near".into(),
            station_uuid.into(),
            "Starbase Alpha".into(),
            "Go ahead, Phoenix.".into(),
            Default::default(),
            vec![],
            "thread-range-near".into(),
            true,
            false,
        ));
    let out = tick(&mut app);

    let (messages, contacts) = out
        .iter()
        .find_map(|m| {
            if let ServerMessage::CommsState {
                messages, contacts, ..
            } = &m.msg
            {
                Some((messages.clone(), contacts.clone()))
            } else {
                None
            }
        })
        .expect("CommsState must be broadcast");

    let contact = contacts
        .iter()
        .find(|c| c.uuid == station_uuid)
        .expect("contact present");
    assert!(contact.in_range, "contact should be in range");
    assert!(
        messages[0].sender_in_range,
        "sender_in_range true when station within range"
    );
}

/// Issue #761 projection: the authored `important` flag and the
/// authoritative `available` (in-range) flag ride onto each wire response.
/// `important` is preserved regardless of range; `available` tracks the
/// sender's reachability and flips false once the station leaves range.
///
/// The message is injected straight into the inbox rather than produced by a
/// hail. It used to come from a `[[comms]] on_hailed` template, and issue
/// #985 deleted that front-end; what is under test here is the BROADCAST
/// stamp, which reads the inbox and the range map and does not care which
/// front-end seated the row.
#[test]
fn comms_state_projects_important_and_available_onto_responses() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456761";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(100.0),
    ));
    let station_entity = app
        .world_mut()
        .spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(100.0),
        ))
        .id();

    let _ = tick(&mut app);
    {
        let mut inbox = app.world_mut().resource_mut::<CommsInboxRes>();
        inbox.0.inject(CommsMessage::injected(
            "msg-important".into(),
            station_uuid.into(),
            "Starbase Alpha".into(),
            "USS Phoenix, please identify yourself.".into(),
            Default::default(),
            vec![CommsResponseView {
                text: "We are on a survey mission.".into(),
                important: true,
                available: true,
            }],
            "thread-important".into(),
            true,
            false,
        ));
    }
    let out = tick(&mut app);
    let messages = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::CommsState { messages, .. } => Some(messages.clone()),
            _ => None,
        })
        .expect("CommsState must be broadcast");
    assert_eq!(messages[0].responses.len(), 1);
    assert!(
        messages[0].responses[0].important,
        "authored important flag must ride the wire"
    );
    assert!(
        messages[0].responses[0].available,
        "response available while sender in range"
    );

    // Move the station far away: the response becomes unavailable while its
    // important flag is unchanged.
    if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
        e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
    }
    let out = tick(&mut app);
    let messages = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::CommsState { messages, .. } => Some(messages.clone()),
            _ => None,
        })
        .expect("CommsState re-broadcast after range flip");
    assert!(
        !messages[0].responses[0].available,
        "response unavailable once sender leaves range"
    );
    assert!(
        messages[0].responses[0].important,
        "important flag is range-independent"
    );
}

// -- Entity-derived hail contacts (#985) ----------------------------------

/// Broadcast contacts, in the order the Comms console receives them.
fn broadcast_contacts(out: &[OutboundMessage]) -> Vec<CommsContact> {
    out.iter()
        .find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else {
                None
            }
        })
        .expect("CommsState must be broadcast")
}

/// A `CommsRange` entity that did NOT opt in stays off the roster. This is
/// the whole reason `hailable` exists: every shipped warship and station
/// declares a range, so range-only derivation would put the entire
/// `combat_test` wave order on the Comms officer's contact list.
#[test]
fn a_comms_range_entity_without_the_opt_in_is_not_a_contact() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::{EntityName, EntityUuid};
    use crate::server_app::Ship;

    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, "declared-station");
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    // The seated contact's own entity, so it survives the prune.
    app.world_mut().spawn((
        EntityUuid("declared-station".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    // A range-bearing enemy that never opted in.
    app.world_mut().spawn((
        EntityUuid("harrow-wave-1".into()),
        EntityName("wave_1".into()),
        Transform::from_xyz(20.0, 0.0, 0.0),
        CommsRange(600.0),
    ));

    let contacts = broadcast_contacts(&tick(&mut app));
    assert_eq!(
        contacts.iter().map(|c| c.uuid.as_str()).collect::<Vec<_>>(),
        vec!["declared-station"],
        "only the seated contact belongs on the roster, got {contacts:?}"
    );
}

/// The opt-in marker puts a live entity on the roster, labelled from its
/// `EntityName` (the world's `name` reference id — the same string a
/// `[[comms]] from` would have carried).
#[test]
fn a_hailable_entity_joins_the_roster_labelled_from_its_entity_name() {
    use crate::comms::{CommsHailable, CommsRange};
    use crate::entities::spawner::{EntityName, EntityUuid};
    use crate::server_app::Ship;

    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, "declared-station");
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid("declared-station".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid("courier-uuid".into()),
        EntityName("world.entity.courier.name".into()),
        Transform::from_xyz(30.0, 0.0, 0.0),
        CommsRange(500.0),
        CommsHailable::default(),
    ));

    let contacts = broadcast_contacts(&tick(&mut app));
    let derived = contacts
        .iter()
        .find(|c| c.uuid == "courier-uuid")
        .expect("the hailable entity must join the roster");
    assert_eq!(derived.name, "world.entity.courier.name");
    assert!(
        derived.in_range,
        "the new contact must carry its real range stamp on the tick it appears"
    );
    // The seated entry is still first — entity-derived contacts are
    // APPENDED, never interleaved.
    assert_eq!(contacts[0].uuid, "declared-station");
}

/// An authored `[comms] display_name` beats the reference id, and the
/// out-of-range stamp reaches an entity-derived contact.
#[test]
fn an_entity_derived_contact_uses_its_authored_display_name_and_range_stamp() {
    use crate::comms::{CommsHailable, CommsRange};
    use crate::entities::spawner::{EntityName, EntityUuid};
    use crate::server_app::Ship;

    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, "declared-station");
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(100.0),
    ));
    app.world_mut().spawn((
        EntityUuid("outpost-uuid".into()),
        EntityName("world.entity.outpost.name".into()),
        Transform::from_xyz(5_000.0, 0.0, 0.0),
        CommsRange(100.0),
        CommsHailable {
            display_name: Some("Relay Outpost".into()),
        },
    ));

    let contacts = broadcast_contacts(&tick(&mut app));
    let derived = contacts
        .iter()
        .find(|c| c.uuid == "outpost-uuid")
        .expect("the hailable entity must join the roster");
    assert_eq!(derived.name, "Relay Outpost");
    assert!(
        !derived.in_range,
        "a distant entity-derived contact must be stamped out of range"
    );
}

/// Lifecycle: a hailable entity that spawns mid-mission joins the roster,
/// and leaves it when it is destroyed. Same live query that drives the
/// range flags, so the two can never disagree.
#[test]
fn an_entity_derived_contact_appears_on_spawn_and_drops_on_despawn() {
    use crate::comms::{CommsHailable, CommsRange};
    use crate::entities::spawner::{EntityName, EntityUuid};
    use crate::server_app::Ship;

    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, "declared-station");
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid("declared-station".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    let before = broadcast_contacts(&tick(&mut app));
    assert!(!before.iter().any(|c| c.uuid == "reinforcement-uuid"));

    // Spawn.
    let reinforcement = app
        .world_mut()
        .spawn((
            EntityUuid("reinforcement-uuid".into()),
            EntityName("world.entity.reinforcement.name".into()),
            Transform::from_xyz(40.0, 0.0, 0.0),
            CommsRange(500.0),
            CommsHailable::default(),
        ))
        .id();
    let after_spawn = broadcast_contacts(&tick(&mut app));
    assert!(
        after_spawn.iter().any(|c| c.uuid == "reinforcement-uuid"),
        "contact must appear when the entity spawns, got {after_spawn:?}"
    );

    // Despawn.
    app.world_mut().entity_mut(reinforcement).despawn();
    let after_despawn = broadcast_contacts(&tick(&mut app));
    assert!(
        !after_despawn.iter().any(|c| c.uuid == "reinforcement-uuid"),
        "contact must drop when the entity is destroyed, got {after_despawn:?}"
    );
    assert!(
        after_despawn.iter().any(|c| c.uuid == "declared-station"),
        "the seated contact must survive the despawn of an unrelated entity"
    );
}

/// A roster row already seated for a UUID and an entity-derived candidate
/// naming the SAME entity collapse to a single row, and the seated row's
/// display metadata is the one that survives.
///
/// The seated row used to be a declarative `[[comms]]` contact, which is
/// what made the rule "declarative wins"; issue #985 deleted that source, so
/// what the rule now protects is idempotency across ticks — the roster is
/// re-merged every tick, and a contact must not lose its label (or its
/// `in_range` / `is_urgent` stamps) to its own re-derivation.
#[test]
fn a_seated_contact_wins_the_uuid_collision_with_its_entity() {
    use crate::comms::{CommsHailable, CommsRange};
    use crate::entities::spawner::{EntityName, EntityUuid};
    use crate::server_app::Ship;

    let mut app = comms_test_app();
    // `setup_game_with_comms` seats the contact named "Starbase Alpha" for
    // this UUID.
    setup_game_with_comms(&mut app, "starbase-uuid");
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid("starbase-uuid".into()),
        EntityName("world.entity.starbase_alpha.name".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
        CommsRange(500.0),
        CommsHailable {
            display_name: Some("Ignored By The Declarative Entry".into()),
        },
    ));

    let contacts = broadcast_contacts(&tick(&mut app));
    assert_eq!(
        contacts.len(),
        1,
        "the two sources must collapse to one row, got {contacts:?}"
    );
    assert_eq!(contacts[0].name, "Starbase Alpha");
}

/// Roster order must not depend on ECS archetype iteration order: several
/// hailable entities land in `(name, uuid)` order no matter what.
#[test]
fn entity_derived_contacts_are_appended_in_deterministic_order() {
    use crate::comms::{CommsHailable, CommsRange};
    use crate::entities::spawner::{EntityName, EntityUuid};
    use crate::server_app::Ship;

    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, "declared-station");
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid("declared-station".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    // Spawned in a deliberately unsorted order.
    for (uuid, name) in [
        ("u-delta", "Delta"),
        ("u-alpha", "Alpha"),
        ("u-charlie", "Charlie"),
        ("u-bravo", "Bravo"),
    ] {
        app.world_mut().spawn((
            EntityUuid(uuid.into()),
            EntityName(name.into()),
            Transform::from_xyz(25.0, 0.0, 0.0),
            CommsRange(500.0),
            CommsHailable::default(),
        ));
    }

    let contacts = broadcast_contacts(&tick(&mut app));
    assert_eq!(
        contacts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Starbase Alpha", "Alpha", "Bravo", "Charlie", "Delta"],
        "seated first, then entity-derived in (name, uuid) order"
    );
}

// -- Review fixes: pruning, server enforcement, despawn handling ----------

/// Contacts whose UUID has no matching `CommsRange`-bearing entity in
/// the world (e.g. the world TOML names a `[[comms]]` template but the
/// referenced entity doesn't declare a `[comms]` block) MUST be pruned
/// before broadcast so they never appear as permanently in-range.
#[test]
fn contact_without_comms_range_entity_is_pruned_from_broadcast() {
    use crate::comms::CommsRange;
    use crate::server_app::Ship;

    let bogus_uuid = "no-such-entity";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, bogus_uuid);

    // Spawn the ship so range tracking activates, but DO NOT spawn an
    // entity with `bogus_uuid` + CommsRange.
    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));

    let out = tick(&mut app);
    let contacts = out
        .iter()
        .find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else {
                None
            }
        })
        .expect("CommsState must be broadcast");

    assert!(
        !contacts.iter().any(|c| c.uuid == bogus_uuid),
        "contact for entity without [comms] block must be pruned, got {contacts:?}"
    );
}

/// When a comms-bearing entity is despawned, its `range_flags` entry
/// must be removed and any inbox message from that sender must be
/// stamped `sender_in_range = false` on the next broadcast.
#[test]
fn entity_despawn_flips_sender_in_range_to_false() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    // Use a real UUID4 so the non-UUID synthetic-sender exception introduced
    // for `_self` / "Starcorp Command" does not suppress the range flip.
    let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456789";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(1000.0),
    ));
    let station_entity = app
        .world_mut()
        .spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(1000.0),
        ))
        .id();
    let _ = tick(&mut app);

    // Hail to populate the inbox while in range.
    push_msg(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
                target_uuid: station_uuid.into(),
            },
        },
    );
    let _ = tick(&mut app);

    // Now despawn the station entity.
    app.world_mut().despawn(station_entity);
    let out = tick(&mut app);

    let messages = out
        .iter()
        .find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else {
                None
            }
        })
        .expect("a broadcast must fire after despawn (range flip)");

    assert!(
        messages.iter().all(|m| !m.sender_in_range),
        "after despawn, all messages from that sender must have sender_in_range=false: {messages:?}"
    );
}

/// Two entities at different distances each get their own flag; flipping
/// only the closer one's range must not affect the farther one's flag.
#[test]
fn multiple_entities_have_independent_range_flags() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let near_uuid = "near-1";
    let far_uuid = "far-1";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, near_uuid);
    // Manually add a second contact.
    {
        let comms = &mut app.world_mut().resource_mut::<CommsRuntime>();
        comms.contacts.push(CommsContact {
            uuid: far_uuid.into(),
            name: "Far".into(),
            in_range: true,
            is_urgent: false,
        });
    }

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid(near_uuid.into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        CommsRange(500.0),
    ));
    app.world_mut().spawn((
        EntityUuid(far_uuid.into()),
        Transform::from_xyz(5000.0, 0.0, 0.0),
        CommsRange(500.0),
    ));

    let out = tick(&mut app);
    let contacts = out
        .iter()
        .find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else {
                None
            }
        })
        .expect("CommsState must be broadcast");

    let near = contacts
        .iter()
        .find(|c| c.uuid == near_uuid)
        .expect("near contact");
    let far = contacts
        .iter()
        .find(|c| c.uuid == far_uuid)
        .expect("far contact");
    assert!(near.in_range, "near contact must be in range");
    assert!(!far.in_range, "far contact must be out of range");
}

/// When a contact flips in_range, a CommsState broadcast must fire even
/// if the inbox itself is clean.
#[test]
fn range_flip_triggers_fresh_broadcast() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "station-flip";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    let ship_entity = app
        .world_mut()
        .spawn((
            Ship,
            crate::server_app::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(station_uuid.into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        CommsRange(500.0),
    ));

    // Drain initial broadcasts.
    let _ = tick(&mut app);
    let _ = tick(&mut app);

    // Move ship far away — this must trigger a fresh broadcast even
    // though the inbox didn't change.
    if let Ok(mut e) = app.world_mut().get_entity_mut(ship_entity) {
        e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
    }
    let out = tick(&mut app);

    let has_broadcast = out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::CommsState { .. }));
    assert!(
        has_broadcast,
        "range flip from in→out must trigger a fresh CommsState broadcast"
    );
}

/// If the player ship is despawned mid-game (hypothetical hull-zero edge
/// case), the server must NOT silently re-enable comms by flipping
/// `range_active` back to false. All tracked flags must be forced to
/// false so the Hail / Respond gates stay closed.
#[test]
fn ship_despawn_mid_game_keeps_gates_closed() {
    use crate::comms::CommsRange;
    use crate::entities::spawner::EntityUuid;
    use crate::server_app::Ship;

    let station_uuid = "station-ship-despawn";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    let ship_entity = app
        .world_mut()
        .spawn((
            Ship,
            crate::server_app::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(1000.0),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(station_uuid.into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        CommsRange(1000.0),
    ));
    let _ = tick(&mut app);

    // Sanity: contact is in range.
    {
        let comms = app.world().resource::<CommsRuntime>();
        assert!(comms.range_active, "range_active must be true with ship");
        assert_eq!(comms.range_flags.get(station_uuid).copied(), Some(true));
    }

    // Despawn the ship.
    app.world_mut().despawn(ship_entity);
    let _ = tick(&mut app);

    let comms = app.world().resource::<CommsRuntime>();
    assert!(
        comms.range_active,
        "range_active must REMAIN true after ship despawn (no back-door)"
    );
    assert_eq!(
        comms.range_flags.get(station_uuid).copied(),
        Some(false),
        "tracked flag must be forced false on ship despawn"
    );
    assert!(
        comms.contacts.iter().all(|c| !c.in_range),
        "all contacts must be out of range after ship despawn"
    );
}

// -- Issue #506: channel-2 routing tests ----------------------------------

/// Scenario content arrives in `CommsInboxRes` via channel-2: a producer
/// writes `CommsChannel2Event`, `handle_comms_channel2` injects it.
///
/// The producer used to be `inject_comms_templates` firing a `[[comms]]`
/// template on `WorldLoaded`; issue #985 deleted it, and the producer is now
/// `open_scripted_comms_threads` (covered in `comms::scripted`). The event is
/// written directly here because the DELIVERY leg is what this test is for.
#[test]
fn scenario_hail_arrives_in_inbox_via_channel2() {
    let mut app = ai_trigger_test_app();

    app.world_mut().write_message(CommsChannel2Event {
        message: CommsMessage::injected(
            "msg-ch2".into(),
            "outpost-uuid-ch2".into(),
            "Outpost Alpha".into(),
            "Channel-2 test message.".into(),
            Default::default(),
            vec![],
            "thread-ch2".into(),
            true,
            false,
        ),
    });

    app.update();

    let messages = app.world().resource::<CommsInboxRes>().0.messages();
    assert_eq!(
        messages.len(),
        1,
        "scenario hail must arrive in inbox after routing through channel-2"
    );
    assert_eq!(messages[0].body, "Channel-2 test message.");
    assert_eq!(messages[0].sender_name, "Outpost Alpha");
}

/// Issue #786: `handle_comms_channel2` is INJECT-ONLY. It used to carry an
/// AI branch that called `inbox.record_response(&id, 0)` directly whenever
/// the comms system was AI-operated — a direct authoritative write that
/// bypassed admission AND `handle_respond_to_message`, so no trigger action
/// ever fired and no follow-up ever advanced. That stub is retired: the AI's
/// answer is now decided by `operate_comms_response_ai` and emitted as an
/// ordinary admitted `RespondToMessage` for the real router (proved by
/// `console::comms::server::tests::comms_ai_response_fires_trigger_actions_through_the_router`).
///
/// This test pins the retirement: with an AI-operated comms system and a
/// message carrying a response, channel-2 delivery injects the message and
/// leaves `selected_response` untouched.
#[test]
fn channel2_injection_never_auto_responds_for_ai_comms() {
    let mut app = ai_trigger_test_app();

    // Spawn a Ship entity with comms system set to AI control.
    {
        let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
        sources.0.set(
            crate::ship::system_registry::comms_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            sources,
        ));
    }

    app.world_mut().write_message(CommsChannel2Event {
        message: CommsMessage::injected(
            "msg-ai-ch2".into(),
            "sector-hq-uuid".into(),
            "sector_hq".into(),
            "AI auto-respond test.".into(),
            Default::default(),
            vec![CommsResponseView {
                text: "Acknowledged.".into(),
                important: false,
                available: true,
            }],
            "thread-ai-ch2".into(),
            true,
            false,
        ),
    });

    app.update();

    let messages = app.world().resource::<CommsInboxRes>().0.messages();
    assert_eq!(messages.len(), 1, "message must be injected into inbox");
    assert_eq!(
        messages[0].selected_response, None,
        "channel-2 delivery must never record a response: the retired stub \
         bypassed admission and the consequence router (issue #786)"
    );
    assert!(
        !messages[0].is_read,
        "channel-2 delivery must not mark an AI-operated ship's message read"
    );
}
