use super::{
    apply_gm_roster_replacement, apply_instagib_toggles, defer_unloaded_scenario_content,
    host_channels, import_artifact_into_catalogue, import_resume_after_scenario,
    legacy_force_start_allowed, load_resume_after_scenario, queue_fleet_lobby_input_bounded,
    rebind_fleet_lobby_projections, save_slot_start_projection, scoped_browser_save_namespace,
    BoundedFifo, BrowserResumeRefusal, ImportSlotRefusal, PendingBrowserSaves,
    MAX_PENDING_BROWSER_SAVES,
};
use crate::console::navigation::server::apply_teleport_to_waypoint;
use crate::console::navigation::{NavigationWaypoint, WaypointMode};
use crate::server_app::Instagib;
use crate::ship::state::ShipPhysics;
use bevy::prelude::{App, Messages};
use std::fmt;

#[derive(Debug)]
struct ReadOnlyStoreError;

impl fmt::Display for ReadOnlyStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("read-only test store")
    }
}

struct ReadOnlyStore {
    slot: String,
    text: String,
}

impl vellum_save::Store for ReadOnlyStore {
    type Error = ReadOnlyStoreError;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok((slot == self.slot).then(|| self.text.clone()))
    }

    fn write(&self, _slot: &str, _contents: &str) -> Result<(), Self::Error> {
        Err(ReadOnlyStoreError)
    }

    fn remove(&self, _slot: &str) -> Result<(), Self::Error> {
        Err(ReadOnlyStoreError)
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(vec![self.slot.clone()])
    }
}

/// A Store that actually takes writes, for the paths that make a row.
#[derive(Default)]
struct MapStore {
    slots: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
}

impl vellum_save::Store for MapStore {
    type Error = ReadOnlyStoreError;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.slots.borrow().get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        self.slots
            .borrow_mut()
            .insert(slot.to_string(), contents.to_string());
        Ok(())
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.slots.borrow_mut().remove(slot);
        Ok(())
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(self.slots.borrow().keys().cloned().collect())
    }
}

/// A minimal portable artifact of `scenario`, as a file a host would pick.
fn portable_artifact(scenario: &str, versions: &vellum_save::Versions) -> String {
    crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot {
            tick: 42,
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            ..Default::default()
        },
        0xfeed,
        17,
        scenario,
        versions.clone(),
    )
    .to_ron()
    .expect("portable artifact must encode")
}

fn one_game_start_world() -> crate::world::config::WorldConfig {
    crate::world::config::parse_world(
        r#"
[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
"#,
    )
    .expect("browser resume world fixture must parse")
}

fn content_refusal(saved_content: u64, current_content: u64) -> crate::save_slots::StartState {
    let saved = vellum_save::Versions::new(7, "rules", saved_content);
    let current = vellum_save::Versions::new(7, "rules", current_content);
    let moved = saved.check(&current).expect_err("content must move");
    assert!(matches!(moved, vellum_save::Moved::Content { .. }));
    crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(moved))
}

fn catalogue_row(
    scenario: &str,
    start: crate::save_slots::StartState,
) -> crate::save_slots::SaveSlotEntry {
    crate::save_slots::SaveSlotEntry {
        slot_id: crate::save_slots::AUTOSAVE_SLOT.to_string(),
        kind: crate::save_slots::SaveSlotKind::Autosave,
        display_name: crate::save_slots::AUTOSAVE_SLOT.to_string(),
        metadata: crate::save_slots::MetadataStatus::NotApplicable,
        record: Some(crate::save_slots::SaveRecordSummary {
            scenario: scenario.to_string(),
            seed: 17,
            capture_tick: 42,
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            versions: vellum_save::Versions::new(7, "rules", 0x1234),
        }),
        start,
    }
}

#[test]
fn gm_roster_replacement_broadcasts_only_when_canonical_contents_change() {
    use crate::core::messages::{DeliveryClass, ServerMessage};
    use crate::gm_roster::{GmOperator, GmRoster};
    use crate::lobby::{OutboundMessage, Target};

    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .init_resource::<GmRoster>();
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();
    let first = GmRoster::try_new(vec![
        GmOperator {
            id: "gm-2".into(),
            name: String::new(),
            connected: false,
            ready: false,
        },
        GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: true,
        },
    ])
    .unwrap();

    assert!(apply_gm_roster_replacement(app.world_mut(), first.clone()));
    assert!(
        !apply_gm_roster_replacement(app.world_mut(), first),
        "the same canonical full replacement is a no-op"
    );

    let messages: Vec<_> = cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .collect();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].target, Target::All);
    assert_eq!(messages[0].delivery, DeliveryClass::Reliable);
    assert!(matches!(
        &messages[0].msg,
        ServerMessage::GmRosterChanged { gms }
            if gms.iter().map(|gm| gm.id.as_str()).collect::<Vec<_>>()
                == vec!["gm-1", "gm-2"]
    ));
}

#[test]
fn gm_roster_replacement_clears_ready_on_reconnect() {
    use crate::gm_roster::{GmOperator, GmRoster};
    use crate::lobby::OutboundMessage;

    let mut app = App::new();
    app.add_message::<OutboundMessage>().insert_resource(
        GmRoster::try_new(vec![GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: false,
            ready: false,
        }])
        .unwrap(),
    );
    let replacement = GmRoster::try_new(vec![GmOperator {
        id: "gm-1".into(),
        name: "Morgan".into(),
        connected: true,
        ready: true,
    }])
    .unwrap();

    assert!(apply_gm_roster_replacement(app.world_mut(), replacement));
    let gm = &app.world().resource::<GmRoster>().operators()[0];
    assert!(gm.connected);
    assert!(!gm.ready, "a reconnect always returns unready");
}

/// The host page publishes the crew-public GM roster as the projection of a
/// FLEET, so a session with no fleet publishes the empty array — at boot,
/// before `wasm_init` has even bound anything, and again whenever a fleet
/// closes. That replacement used to unseat a standalone game master's own
/// bound presence, which left every control on its desk admitted by identity
/// and refused for absence.
#[test]
fn a_standalone_game_masters_own_presence_survives_the_pages_empty_roster() {
    use crate::gm_roster::GmRoster;
    use crate::lobby::OutboundMessage;

    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .insert_resource(crate::lockstep::FleetRoster::default())
        .init_resource::<GmRoster>();
    crate::gm_solo::bind_standalone_game_master(app.world_mut()).expect("a solo peer binds");

    apply_gm_roster_replacement(app.world_mut(), GmRoster::default());

    assert!(app
        .world()
        .resource::<GmRoster>()
        .is_connected(crate::gm_solo::SOLO_GM_OPERATOR_ID));
    assert!(crate::gm_solo::local_gm_operator(app.world()).is_some());
}

/// The same replacement on a FLEET peer is applied exactly as published —
/// the page's projection is the roster there, and an emptied one is a real
/// fact about the fleet.
#[test]
fn a_fleet_peers_empty_roster_is_applied_unchanged() {
    use crate::command_admission::HostSlot;
    use crate::gm_roster::{GmOperator, GmRoster};
    use crate::lobby::OutboundMessage;

    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .insert_resource(
            crate::lockstep::FleetRoster::with_participants_and_gms(
                Vec::new(),
                vec![HostSlot(1), HostSlot(2)],
                vec![crate::lockstep::FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-2".into(),
                }],
                HostSlot(2),
                HostSlot(1),
            )
            .expect("a stationless fleet GM participant"),
        )
        .insert_resource(
            GmRoster::try_new(vec![GmOperator::new("gm-2".into(), "Morgan".into(), true)]).unwrap(),
        );

    assert!(apply_gm_roster_replacement(
        app.world_mut(),
        GmRoster::default()
    ));
    assert!(app.world().resource::<GmRoster>().is_empty());
}

#[test]
fn legacy_ai_force_start_is_refused_while_fleet_managed() {
    let mut managed = crate::lobby::FleetManagedLobby::default();
    assert!(legacy_force_start_allowed(
        &managed,
        &crate::core::messages::GamePhase::Lobby
    ));
    managed.set_enabled(true);
    assert!(!legacy_force_start_allowed(
        &managed,
        &crate::core::messages::GamePhase::Lobby
    ));
    managed.set_enabled(false);
    assert!(!legacy_force_start_allowed(
        &managed,
        &crate::core::messages::GamePhase::InProgress
    ));
}

#[test]
fn fleet_lobby_queue_coalesces_absolute_samples_without_losing_edges() {
    use crate::lobby::FleetLobbyInput;
    use std::collections::VecDeque;

    let mut pending = VecDeque::new();
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        8,
        FleetLobbyInput::Managed(false),
        4,
    ));
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        8,
        FleetLobbyInput::Managed(true),
        4,
    ));
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        8,
        FleetLobbyInput::Validation(false),
        4,
    ));
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        8,
        FleetLobbyInput::Validation(true),
        4,
    ));
    assert_eq!(
        pending.len(),
        3,
        "only the consecutive validation coalesces"
    );
    assert!(matches!(
        pending.pop_front().map(|row| row.input),
        Some(FleetLobbyInput::Managed(false))
    ));
    assert!(matches!(
        pending.pop_front().map(|row| row.input),
        Some(FleetLobbyInput::Managed(true))
    ));
    assert!(matches!(
        pending.pop_front().map(|row| row.input),
        Some(FleetLobbyInput::Validation(true))
    ));
}

#[test]
fn fleet_lobby_queue_refuses_rather_than_displacing_a_generation_edge() {
    use crate::lobby::FleetLobbyInput;
    use std::collections::VecDeque;

    let mut pending = VecDeque::new();
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        2,
        FleetLobbyInput::Managed(false),
        2,
    ));
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        2,
        FleetLobbyInput::Managed(true),
        2,
    ));
    assert!(!queue_fleet_lobby_input_bounded(
        &mut pending,
        2,
        FleetLobbyInput::Validation(true),
        2,
    ));
    assert_eq!(pending.len(), 2);
    assert!(matches!(pending[0].input, FleetLobbyInput::Managed(false)));
    assert!(matches!(pending[1].input, FleetLobbyInput::Managed(true)));
}

#[test]
fn prejoin_lobby_projections_rebind_before_the_new_generations_grant() {
    use crate::lobby::start_policy::{StartGrant, StartGrantMode};
    use crate::lobby::FleetLobbyInput;
    use std::collections::VecDeque;

    let mut pending = VecDeque::new();
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        0,
        FleetLobbyInput::Managed(true),
        64,
    ));
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        0,
        FleetLobbyInput::Validation(true),
        64,
    ));

    rebind_fleet_lobby_projections(&mut pending, 1, Some(true), Some(true));
    assert!(queue_fleet_lobby_input_bounded(
        &mut pending,
        1,
        FleetLobbyInput::Grant(StartGrant {
            id: "start-1".into(),
            mode: StartGrantMode::Automatic,
            operator_id: None,
            apply_tick: 0,
        }),
        64,
    ));
    assert_eq!(
        pending.iter().map(|row| row.generation).collect::<Vec<_>>(),
        vec![1, 1, 1]
    );

    let mut inputs = pending.into_iter().map(|row| row.input).collect();
    let mut managed = crate::lobby::FleetManagedLobby::default();
    let mut grants = crate::lobby::PendingStartGrants::default();
    let mut tracker = crate::lobby::server::StartGrantTracker::default();
    let mut results = crate::lobby::StartGrantResults::default();
    crate::lobby::apply_fleet_lobby_inputs(
        &mut inputs,
        &mut managed,
        &mut grants,
        &mut tracker,
        &mut results,
    );
    assert!(inputs.is_empty());
    assert!(managed.enabled);
    assert!(managed.validation_passed);
    assert_eq!(grants.len(), 1);
}

#[test]
fn save_slot_projection_keeps_only_deferred_content_startable() {
    use crate::save_slots::StartState;

    let ready = save_slot_start_projection(&StartState::Ready);
    assert!(ready.compatible);
    assert!(ready.startable);
    assert_eq!(ready.refusal_kind, None);

    let deferred = save_slot_start_projection(&StartState::ContentDeferred);
    assert!(!deferred.compatible);
    assert!(deferred.startable);
    assert_eq!(deferred.refusal_kind, Some("content-pending"));
    assert_eq!(deferred.refusal, None);

    let saved = vellum_save::Versions::new(7, "rules-before", 0x1234);
    let current = vellum_save::Versions::new(7, "rules-now", 0x1234);
    let rules = saved.check(&current).expect_err("rules moved");
    let refused = save_slot_start_projection(&StartState::Refused(
        crate::snapshot::LoadRefusal::Moved(rules),
    ));
    assert!(!refused.compatible);
    assert!(!refused.startable);
    assert_eq!(refused.refusal_kind, Some("rules"));

    let corrupt = save_slot_start_projection(&StartState::Refused(
        crate::snapshot::LoadRefusal::Unparsable("damaged run".into()),
    ));
    assert!(!corrupt.startable);
    assert_eq!(corrupt.refusal_kind, Some("unparsable"));
}

#[test]
fn save_catalogue_defers_content_on_fresh_boot() {
    let mut entries = [catalogue_row(
        "assets/worlds/scenario-a.toml",
        crate::save_slots::StartState::Ready,
    )];

    defer_unloaded_scenario_content(&mut entries, None);

    assert_eq!(
        entries[0].start,
        crate::save_slots::StartState::ContentDeferred,
        "even an accidental digest match is not proof before a scenario is loaded"
    );
}

#[test]
fn save_catalogue_keeps_same_scenario_content_mismatch_refused() {
    let scenario = "assets/worlds/scenario-a.toml";
    let mut entries = [catalogue_row(scenario, content_refusal(0x1111, 0x2222))];

    defer_unloaded_scenario_content(&mut entries, Some(scenario));

    assert!(matches!(
        entries[0].start,
        crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Content { .. }
        ))
    ));
}

#[test]
fn save_catalogue_defers_other_scenario_content_but_not_hard_failures() {
    let saved_rules = vellum_save::Versions::new(7, "rules-before", 0x1111);
    let current_rules = vellum_save::Versions::new(7, "rules-now", 0x1111);
    let rules_moved = saved_rules
        .check(&current_rules)
        .expect_err("rules must move");
    let mut entries = [
        catalogue_row(
            "assets/worlds/scenario-b.toml",
            content_refusal(0x1111, 0x2222),
        ),
        catalogue_row(
            "assets/worlds/scenario-b.toml",
            crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                rules_moved,
            )),
        ),
        catalogue_row(
            "assets/worlds/scenario-b.toml",
            crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(
                "backend unavailable".into(),
            )),
        ),
    ];

    defer_unloaded_scenario_content(&mut entries, Some("assets/worlds/scenario-a.toml"));

    assert_eq!(
        entries[0].start,
        crate::save_slots::StartState::ContentDeferred
    );
    assert!(matches!(
        entries[1].start,
        crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Rules { .. }
        ))
    ));
    assert!(matches!(
        entries[2].start,
        crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(_))
    ));
}

#[test]
fn resume_gate_refuses_content_after_selected_scenario_loads() {
    let scenario = "assets/worlds/scenario-b.toml";
    let saved_versions = vellum_save::Versions::new(7, "rules", 0x1111);
    let loaded_versions = vellum_save::Versions::new(7, "rules", 0x2222);
    let mut catalogue = [catalogue_row(scenario, content_refusal(0x1111, 0xaaaa))];
    defer_unloaded_scenario_content(&mut catalogue, Some("assets/worlds/scenario-a.toml"));
    assert_eq!(
        catalogue[0].start,
        crate::save_slots::StartState::ContentDeferred
    );

    let run = crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot::default(),
        0,
        17,
        scenario,
        saved_versions,
    );
    let store = ReadOnlyStore {
        slot: "selected-slot".into(),
        text: run.to_ron().expect("test run must encode"),
    };

    let refusal = load_resume_after_scenario(
        &store,
        "selected-slot",
        &loaded_versions,
        "assets/entities/alliance_cruiser.toml",
        &one_game_start_world(),
    )
    .expect_err("the full post-load content gate must refuse the run");
    assert!(matches!(
        refusal,
        BrowserResumeRefusal::Load(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Content { .. }
        ))
    ));
}

#[test]
fn browser_resume_refuses_a_different_selected_hull_before_staging() {
    let current = vellum_save::Versions::new(7, "rules", 0x1234);
    let run = crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot {
            tick: 42,
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: "assets/entities/alliance_destroyer.toml".into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            ..Default::default()
        },
        0xfeed,
        17,
        "assets/worlds/default.toml",
        current.clone(),
    );
    let store = ReadOnlyStore {
        slot: "selected-slot".into(),
        text: run.to_ron().expect("test run must encode"),
    };

    let refusal = load_resume_after_scenario(
        &store,
        "selected-slot",
        &current,
        "assets/entities/alliance_cruiser.toml",
        &one_game_start_world(),
    )
    .expect_err("a different hull must never receive the saved component state");
    assert_eq!(
        refusal,
        BrowserResumeRefusal::WrongSelectedShip {
            saved: "assets/entities/alliance_destroyer.toml".into(),
            loaded: "assets/entities/alliance_cruiser.toml".into(),
        }
    );
}

#[test]
fn an_imported_artifact_becomes_a_row_of_the_ordinary_catalogue() {
    // Issue #1363's AC2. The importer now sits in the save catalogue's
    // header, which makes importing an action ON that list — and an action
    // on a list that leaves the list unchanged is a control in the wrong
    // panel. So the file becomes a manual slot like any other, listed by
    // the same `list_slots` the catalogue reads, under the name the
    // operator's file had.
    let current = vellum_save::Versions::new(7, "rules", 0x1234);
    let artifact = portable_artifact("assets/worlds/default.toml", &current);
    let store = MapStore::default();

    let slot_id = import_artifact_into_catalogue(&store, &artifact, "away-team.ron")
        .expect("an intact artifact must enter the catalogue");

    let listed = crate::save_slots::list_slots(&store, &current)
        .expect("the catalogue must list what was just written");
    let row = listed
        .iter()
        .find(|entry| entry.slot_id == slot_id)
        .expect("the imported save must be a row of the ordinary catalogue");
    assert_eq!(row.kind, crate::save_slots::SaveSlotKind::Manual);
    assert_eq!(row.display_name, "away-team.ron");
    assert_eq!(
        row.record
            .as_ref()
            .expect("an imported row carries the run's own summary")
            .scenario,
        "assets/worlds/default.toml"
    );
    // ...and it is startable the moment it lands, which is what makes it a
    // row of this list rather than a private shelf beside it.
    assert!(matches!(row.start, crate::save_slots::StartState::Ready));
}

#[test]
fn a_file_that_is_not_a_run_never_becomes_a_row() {
    // The one rule this path must not break, and the reason the helper is
    // pure: a catalogue row is a promise that something can be read back.
    let store = MapStore::default();
    let refusal = import_artifact_into_catalogue(&store, "not a save at all", "junk.txt")
        .expect_err("an unparsable file must not become a row");
    assert!(matches!(refusal, ImportSlotRefusal::Damaged(_)));
    assert!(refusal.to_string().starts_with("damaged\t"));
    assert!(
        vellum_save::Store::slots(&store)
            .expect("the fake store lists")
            .is_empty(),
        "a refused import must leave the Store untouched"
    );
}

#[test]
fn a_store_that_will_not_take_it_is_a_different_answer_from_a_damaged_file() {
    // Two classes because they send a host to two different places: pick
    // another file, against make room. The page is not left to infer which
    // from an English sentence it may not paraphrase.
    let current = vellum_save::Versions::new(7, "rules", 0x1234);
    let artifact = portable_artifact("assets/worlds/default.toml", &current);
    let store = ReadOnlyStore {
        slot: "occupied".into(),
        text: String::new(),
    };
    let refusal = import_artifact_into_catalogue(&store, &artifact, "away-team.ron")
        .expect_err("a Store that refuses writes cannot make a row");
    assert!(matches!(refusal, ImportSlotRefusal::NotStored(_)));
    assert!(refusal.to_string().starts_with("not-stored\t"));
}

#[test]
fn an_incompatible_artifact_still_enters_the_catalogue_and_is_refused_on_its_row() {
    // Compatibility is not an ENTRY gate, exactly as it is not a copy gate
    // on the way out (`wasm_export_save_slot`). It could not be one here:
    // the content dimension is a digest over the world the save names, and
    // a host standing at the catalogue has loaded no world. The refusal
    // arrives where #1363's AC3 asks for it — on the row — and the gate
    // that AC4 is about still runs before anything is restored.
    let saved = vellum_save::Versions::new(6, "rules", 0x1234);
    let current = vellum_save::Versions::new(7, "rules", 0x1234);
    let artifact = portable_artifact("assets/worlds/default.toml", &saved);
    let store = MapStore::default();

    let slot_id = import_artifact_into_catalogue(&store, &artifact, "older.ron")
        .expect("an intact artifact from another build still enters the list");
    let listed =
        crate::save_slots::list_slots(&store, &current).expect("the catalogue must list it");
    let row = listed
        .iter()
        .find(|entry| entry.slot_id == slot_id)
        .expect("the imported save is a row");
    assert!(
        matches!(
            row.start,
            crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Moved(_))
        ),
        "the row says it cannot be started, and names the dimension that moved"
    );
    assert!(!row.can_start());
}

#[test]
fn portable_import_applies_the_non_default_hull_gate_before_staging() {
    let current = vellum_save::Versions::new(7, "rules", 0x1234);
    let run = crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot {
            tick: 42,
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            ..Default::default()
        },
        0xfeed,
        17,
        "assets/worlds/default.toml",
        current.clone(),
    );
    let artifact = run.to_ron().expect("portable artifact must encode");
    let world = one_game_start_world();

    let accepted = import_resume_after_scenario(
        &artifact,
        &current,
        "assets/entities/alliance_cruiser.toml",
        &world,
    )
    .expect("the artifact's non-default saved hull must pass unchanged");
    assert_eq!(
        crate::snapshot::required_boot_identity(&accepted)
            .expect("accepted import keeps boot identity")
            .selected_ship,
        "assets/entities/alliance_cruiser.toml"
    );

    let refusal = import_resume_after_scenario(
        &artifact,
        &current,
        "assets/entities/alliance_destroyer.toml",
        &world,
    )
    .expect_err("a portable import must not bypass the selected-hull gate");
    assert_eq!(
        refusal,
        BrowserResumeRefusal::WrongSelectedShip {
            saved: "assets/entities/alliance_cruiser.toml".into(),
            loaded: "assets/entities/alliance_destroyer.toml".into(),
        }
    );

    let mut world_with_unmapped_npc = world.clone();
    let mut npc_row = world_with_unmapped_npc.entities[0].clone();
    npc_row.id = Some("game-start-npc".into());
    world_with_unmapped_npc.entities.push(npc_row);
    assert!(matches!(
        import_resume_after_scenario(
            &artifact,
            &current,
            "assets/entities/alliance_cruiser.toml",
            &world_with_unmapped_npc,
        ),
        Err(BrowserResumeRefusal::Load(
            crate::snapshot::LoadRefusal::Unparsable(_)
        ))
    ));

    let mut out_of_bounds = run;
    out_of_bounds
        .snapshot
        .as_mut()
        .unwrap()
        .state
        .boot_identity
        .as_mut()
        .unwrap()
        .game_start_entity_uuids[0]
        .authored_index = 1;
    let artifact = out_of_bounds.to_ron().unwrap();
    assert!(matches!(
        import_resume_after_scenario(
            &artifact,
            &current,
            "assets/entities/alliance_cruiser.toml",
            &world,
        ),
        Err(BrowserResumeRefusal::Load(
            crate::snapshot::LoadRefusal::Unparsable(_)
        ))
    ));
}

/// Teleport onto a Free waypoint sets `x`/`z` and leaves `y` unchanged.
#[test]
fn teleport_sets_xz_and_preserves_y() {
    let mut physics = ShipPhysics {
        x: 1.0,
        y: 42.0,
        z: 2.0,
        ..Default::default()
    };
    let waypoint = NavigationWaypoint::new(WaypointMode::Free { x: 120.0, z: -45.0 });

    let teleported = apply_teleport_to_waypoint(&mut physics, &waypoint);

    assert!(teleported, "a waypoint exists, so a teleport should happen");
    assert_eq!(physics.x, 120.0);
    assert_eq!(physics.z, -45.0);
    assert_eq!(physics.y, 42.0, "altitude must be left unchanged");
}

/// An Anchored waypoint teleports to its live-cached x/z.
#[test]
fn teleport_uses_anchored_snapshot_position() {
    let mut physics = ShipPhysics::default();
    let waypoint = NavigationWaypoint::new(WaypointMode::Anchored {
        source_uuid: "target-1".into(),
        last_x: 75.0,
        last_z: -150.0,
    });

    let teleported = apply_teleport_to_waypoint(&mut physics, &waypoint);

    assert!(teleported);
    assert_eq!(physics.x, 75.0);
    assert_eq!(physics.z, -150.0);
}

/// With no waypoint set the teleport is a no-op and reports `false`.
#[test]
fn teleport_without_waypoint_is_a_noop() {
    let mut physics = ShipPhysics {
        x: 7.0,
        y: 3.0,
        z: 9.0,
        ..Default::default()
    };
    let waypoint = NavigationWaypoint::default();

    let teleported = apply_teleport_to_waypoint(&mut physics, &waypoint);

    assert!(!teleported, "no waypoint means nothing to teleport to");
    assert_eq!(physics.x, 7.0);
    assert_eq!(physics.y, 3.0);
    assert_eq!(physics.z, 9.0);
}

/// The Host Channel name table (issue #818) must have no duplicates —
/// the JS dispatcher in `server.html` keys its handlers by these names.
#[test]
fn host_channel_names_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for name in host_channels::ALL {
        assert!(
            seen.insert(name),
            "duplicate host channel name: {name:?} — each name must map to \
                 exactly one JS handler"
        );
    }
}

/// Every named channel const is present in `ALL` (and nothing else is) —
/// `flush_host_channels` and the JS dispatcher both key off these.
#[test]
fn host_channel_all_covers_every_const() {
    assert_eq!(
        host_channels::ALL,
        [
            host_channels::HUD,
            host_channels::LOBBY,
            host_channels::CHATTER,
            host_channels::AUDIO_CONFIG,
            host_channels::AUDIO_CUE,
            host_channels::AUDIO_LIFECYCLE,
            host_channels::SHAKE,
            host_channels::AUDIO_LEVEL,
            host_channels::GM_ENTITY,
            host_channels::GM_ACTIVITY,
            host_channels::GM_STATION,
            host_channels::GM_SESSION,
            host_channels::GM_MISSION,
            host_channels::GM_SPAWN,
            host_channels::GM_COMMS,
            host_channels::GM_ATTENTION,
            host_channels::GM_HEALTH,
            host_channels::GM_WORKLOAD,
        ]
    );
}

/// A connected phone's bridge batch reaches the same catalogue adapter as
/// the host route, while Pause stays untouched and duplicate diagnostic
/// toggles still collapse to one flip.
#[cfg(not(phoenix_demo_build))]
#[test]
fn phone_bridge_batch_applies_the_named_surface_and_not_pause() {
    use crate::core::debug_surface::DebugSurface;
    use crate::core::messages::ClientMessage;
    use crate::lobby::{InboundMessage, Sessions};
    use bevy::prelude::{App, Messages, Update};

    let mut app = App::new();
    app.add_message::<InboundMessage>();
    app.init_resource::<crate::debug_overlay::DebugRegionsEnabled>();
    app.init_resource::<crate::debug_overlay::DebugOverlayEnabled>();
    app.init_resource::<crate::debug_overlay::SimulationPaused>();
    app.init_resource::<crate::debug_overlay::DebugDamageEnabled>();
    app.init_resource::<crate::debug_overlay::DebugEntitiesEnabled>();
    app.init_resource::<crate::debug_overlay::DebugEntityInspectorEnabled>();
    app.init_resource::<crate::debug::DebugStationActivityEnabled>();
    app.init_resource::<crate::debug::DebugAiDoctrineEnabled>();
    app.init_resource::<crate::debug::DebugScenarioStateEnabled>();
    app.init_resource::<crate::debug::DebugConsoleLatencyEnabled>();
    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions
        .register("phone".into(), "Tester".into())
        .expect("register connected phone");
    app.insert_resource(Sessions(sessions));
    app.add_systems(Update, super::drain_client_debug_flags);
    for _ in 0..2 {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: "phone".into(),
                msg: ClientMessage::ToggleDebugFlag {
                    flag: DebugSurface::Damage,
                },
            });
    }
    app.update();

    assert!(
        app.world()
            .resource::<crate::debug_overlay::DebugDamageEnabled>()
            .0
    );
    assert!(
        !app.world()
            .resource::<crate::debug_overlay::SimulationPaused>()
            .0
    );
}

// ── Instagib queue-drain semantics (issue #1181) ────────────────────────

/// Draining an empty instagib queue leaves the flag untouched — the frame
/// after a drain, with nothing queued, must not re-flip.
#[test]
fn draining_no_instagib_toggles_leaves_the_flag() {
    let mut on = false;
    apply_instagib_toggles(0, &mut on);
    assert!(!on, "no queued toggles must not flip");

    let mut already_on = true;
    apply_instagib_toggles(0, &mut already_on);
    assert!(already_on, "no queued toggles must preserve an on flag");
}

/// One queued toggle flips the flag exactly once.
#[test]
fn one_instagib_toggle_flips_once() {
    let mut on = false;
    apply_instagib_toggles(1, &mut on);
    assert!(on, "one toggle: false -> true");

    apply_instagib_toggles(1, &mut on);
    assert!(!on, "one toggle again: true -> false");
}

/// The queue is a COUNT, so its parity decides the net flip — two clicks in
/// one frame cancel, three land as one, matching what the same clicks spread
/// over separate ticks would do (the God Mode drain's contract).
#[test]
fn instagib_toggle_count_applies_by_parity() {
    let mut on = false;
    apply_instagib_toggles(2, &mut on);
    assert!(!on, "two toggles in one frame cancel");

    apply_instagib_toggles(3, &mut on);
    assert!(on, "three toggles net to one flip");

    apply_instagib_toggles(4, &mut on);
    assert!(on, "four toggles net to no change");
}

/// The drain's shape end to end: the `Instagib` Resource starts off, a queued
/// batch flips it, and a mirror read reflects the Resource — the same round
/// trip `drain_instagib_toggle` + `publish_instagib` perform on wasm, minus
/// the thread-local edge.
#[test]
fn instagib_resource_round_trips_a_drained_batch() {
    let mut instagib = Instagib::default();
    assert!(!instagib.0, "starts off");

    // One frame's queue of a single click.
    apply_instagib_toggles(1, &mut instagib.0);
    assert_eq!(instagib, Instagib(true), "a click turns it on");

    // A frame that queued nothing must leave it on.
    apply_instagib_toggles(0, &mut instagib.0);
    assert_eq!(instagib, Instagib(true), "an empty frame preserves it");
}

#[test]
fn browser_save_queue_is_bounded_fifo_and_rejected_work_never_drains() {
    let mut pending = PendingBrowserSaves::new();
    for index in 0..MAX_PENDING_BROWSER_SAVES {
        assert_eq!(
            pending.try_push(format!("token-{index}"), index),
            Ok(()),
            "every request through the finite capacity is accepted"
        );
    }

    assert_eq!(
        pending.try_push("rejected".to_string(), usize::MAX),
        Err(usize::MAX),
        "the first request beyond the bound is refused"
    );
    let requests = pending.take_requests();
    assert_eq!(requests.len(), MAX_PENDING_BROWSER_SAVES);
    assert_eq!(requests.front().map(String::as_str), Some("token-0"));
    let expected_last = format!("token-{}", MAX_PENDING_BROWSER_SAVES - 1);
    assert_eq!(
        requests.back().map(String::as_str),
        Some(expected_last.as_str()),
        "accepted requests retain FIFO order"
    );
    assert!(
        !requests.iter().any(|token| token == "rejected"),
        "a refused call must never reach the fixed-boundary snapshot drain"
    );

    assert_eq!(
        pending.try_push("still-full".to_string(), usize::MAX),
        Err(usize::MAX),
        "taking ingress does not release an in-flight intent"
    );
    assert_eq!(pending.remove_intent("token-0"), Some(0));
    assert_eq!(
        pending.remove_intent("token-0"),
        None,
        "a fixed-boundary refusal can clear and report an intent only once"
    );
    assert_eq!(
        pending.try_push("recovered".to_string(), usize::MAX),
        Ok(())
    );
    assert_eq!(
        pending.take_requests().into_iter().collect::<Vec<_>>(),
        vec!["recovered"],
        "draining one result releases exactly one request slot"
    );
}

#[test]
fn browser_save_namespace_accepts_only_canonical_peer_identity() {
    let identity = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        scoped_browser_save_namespace(identity).as_deref(),
        Some("phoenix:0123456789abcdef0123456789abcdef")
    );

    for invalid in [
        "",
        "0123456789abcdef0123456789abcde",
        "0123456789abcdef0123456789abcdef0",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef:123456789abcdef",
    ] {
        assert_eq!(
            scoped_browser_save_namespace(invalid),
            None,
            "an unvalidated local value must not choose a Store namespace"
        );
    }
}

#[test]
fn browser_status_outbox_keeps_newest_bound_and_recovers_after_poll() {
    let mut statuses = BoundedFifo::<_, 3>::new();
    statuses.push_back("oldest");
    statuses.push_back("second");
    statuses.push_back("third");
    statuses.push_back("queue full refusal");

    assert_eq!(statuses.len(), 3);
    assert_eq!(statuses.pop_front(), Some("second"));
    assert_eq!(statuses.pop_front(), Some("third"));
    assert_eq!(
        statuses.pop_front(),
        Some("queue full refusal"),
        "the newest overload status remains visible and retained values stay FIFO"
    );
    assert_eq!(statuses.pop_front(), None);

    statuses.push_back("after recovery");
    assert_eq!(statuses.pop_front(), Some("after recovery"));
}
