use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use vellum_save::Store;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeOperation {
    Read,
    Write,
    Remove,
    Slots,
}

#[derive(Clone, Debug)]
struct FakeFailure {
    operation: FakeOperation,
    slot: String,
    detail: String,
}

#[derive(Clone, Debug)]
struct FakeStoreError(String);

impl fmt::Display for FakeStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Default)]
struct FakeStore {
    values: RefCell<BTreeMap<String, String>>,
    advertised: RefCell<Vec<String>>,
    failures: RefCell<Vec<FakeFailure>>,
}

impl FakeStore {
    fn put(&self, slot: &str, value: impl Into<String>) {
        self.values
            .borrow_mut()
            .insert(slot.to_string(), value.into());
    }

    fn get(&self, slot: &str) -> Option<String> {
        self.values.borrow().get(slot).cloned()
    }

    fn contains(&self, slot: &str) -> bool {
        self.values.borrow().contains_key(slot)
    }

    fn advertise(&self, slot: &str) {
        self.advertised.borrow_mut().push(slot.to_string());
    }

    fn fail_next(&self, operation: FakeOperation, slot: &str, detail: &str) {
        self.failures.borrow_mut().push(FakeFailure {
            operation,
            slot: slot.to_string(),
            detail: detail.to_string(),
        });
    }

    fn maybe_fail(&self, operation: FakeOperation, slot: &str) -> Result<(), FakeStoreError> {
        let mut failures = self.failures.borrow_mut();
        let Some(index) = failures
            .iter()
            .position(|failure| failure.operation == operation && failure.slot == slot)
        else {
            return Ok(());
        };
        Err(FakeStoreError(failures.remove(index).detail))
    }
}

impl Store for FakeStore {
    type Error = FakeStoreError;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        self.maybe_fail(FakeOperation::Read, slot)?;
        Ok(self.get(slot))
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        self.maybe_fail(FakeOperation::Write, slot)?;
        self.put(slot, contents);
        Ok(())
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.maybe_fail(FakeOperation::Remove, slot)?;
        self.values.borrow_mut().remove(slot);
        Ok(())
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        self.maybe_fail(FakeOperation::Slots, "")?;
        let mut slots: Vec<_> = self.values.borrow().keys().cloned().collect();
        slots.extend(self.advertised.borrow().iter().cloned());
        // Deliberately non-canonical: ordering belongs to the wrapper.
        slots.reverse();
        Ok(slots)
    }
}

const SLOT_A: &str = "00000000-0000-4000-8000-000000000001";
const SLOT_B: &str = "00000000-0000-4000-8000-000000000002";
const SLOT_C: &str = "00000000-0000-4000-8000-000000000003";
const SLOT_D: &str = "00000000-0000-4000-8000-000000000004";
const SLOT_E: &str = "00000000-0000-4000-8000-000000000005";
const SLOT_F: &str = "00000000-0000-4000-8000-000000000006";

fn current_versions() -> vellum_save::Versions {
    vellum_save::Versions::new(7, "rules-a", 0x1234)
}

fn stored_run(
    tick: u64,
    scenario: &str,
    seed: u64,
    versions: vellum_save::Versions,
) -> crate::snapshot::StoredRun {
    crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot {
            tick,
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
        tick.wrapping_mul(17),
        seed,
        scenario,
        versions,
    )
}

fn put_run(store: &FakeStore, slot: &str, run: &crate::snapshot::StoredRun) {
    crate::snapshot::save_to(store, slot, run).expect("fake Store write succeeds");
}

fn boot_identity_mut(run: &mut crate::snapshot::StoredRun) -> &mut crate::snapshot::BootIdentity {
    run.snapshot
        .as_mut()
        .and_then(|snapshot| snapshot.state.boot_identity.as_mut())
        .expect("stored-run fixture carries boot identity")
}

fn manual_entry<'a>(entries: &'a [SaveSlotEntry], slot: &str) -> &'a SaveSlotEntry {
    entries
        .iter()
        .find(|entry| entry.slot_id == slot)
        .expect("manual row is listed")
}

fn step(
    schedule: &mut SaveSchedule,
    tick: u64,
    phase: SavePhase,
    interval: u64,
) -> Vec<CaptureDecision> {
    schedule.step(tick, phase, interval, std::iter::empty::<String>())
}

#[test]
fn current_format_refuses_malformed_game_start_identity_maps() {
    let versions = current_versions();
    let base = stored_run(9, "scenario", 17, versions.clone());
    let mut malformed = Vec::new();

    let mut missing = base.clone();
    boot_identity_mut(&mut missing)
        .game_start_entity_uuids
        .clear();
    malformed.push(missing);

    let mut out_of_order = base.clone();
    boot_identity_mut(&mut out_of_order).game_start_entity_uuids = vec![
        crate::snapshot::GameStartEntityUuid {
            authored_index: 2,
            entity_uuid: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, 2)
                .render(),
        },
        crate::snapshot::GameStartEntityUuid {
            authored_index: 1,
            entity_uuid: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, 1)
                .render(),
        },
    ];
    malformed.push(out_of_order);

    let mut repeated = base.clone();
    let repeated_uuid = boot_identity_mut(&mut repeated).game_start_entity_uuids[0]
        .entity_uuid
        .clone();
    boot_identity_mut(&mut repeated)
        .game_start_entity_uuids
        .push(crate::snapshot::GameStartEntityUuid {
            authored_index: 1,
            entity_uuid: repeated_uuid,
        });
    malformed.push(repeated);

    let mut wrong_namespace = base.clone();
    boot_identity_mut(&mut wrong_namespace).game_start_entity_uuids[0].entity_uuid =
        crate::world_id::WorldId::new(crate::world_id::IdNamespace::Message, 0, 0).render();
    malformed.push(wrong_namespace);

    let mut repeated_snapshot_row = base.clone();
    let repeated_snapshot_uuid = boot_identity_mut(&mut repeated_snapshot_row)
        .game_start_entity_uuids[0]
        .entity_uuid
        .clone();
    repeated_snapshot_row
        .snapshot
        .as_mut()
        .unwrap()
        .state
        .entities = vec![
        crate::snapshot::EntityState {
            uuid: repeated_snapshot_uuid.clone(),
            ..Default::default()
        },
        crate::snapshot::EntityState {
            uuid: repeated_snapshot_uuid,
            ..Default::default()
        },
    ];
    malformed.push(repeated_snapshot_row);

    let mut noncanonical = base;
    boot_identity_mut(&mut noncanonical).game_start_entity_uuids[0].entity_uuid =
        "00000000-0000-8000-8000-00000000000A".into();
    malformed.push(noncanonical);

    for (index, run) in malformed.iter().enumerate() {
        let store = FakeStore::default();
        put_run(&store, SLOT_A, run);
        assert!(
            matches!(
                load_slot(&store, SLOT_A, &versions),
                Err(crate::snapshot::LoadRefusal::Unparsable(_))
            ),
            "malformed GameStart identity case {index} must be refused"
        );
    }
}

/// A stationless GM peer records no hull of its own (issue #1445): `?gm=1`
/// inserts no `SelectedShipResource`, so its identity is the replicated
/// roster it participates in. That relaxation belongs ONLY to a peer with no
/// roster ship — a run that claims a hull must still be in a fleet.
#[test]
fn a_stationless_identity_is_admitted_while_a_hull_claim_still_needs_its_fleet() {
    use crate::command_admission::log::HostSlot;
    let versions = current_versions();

    let mut gm = stored_run(9, "scenario", 17, versions.clone());
    {
        let boot = boot_identity_mut(&mut gm);
        boot.selected_ship = String::new();
        boot.fleet = crate::lockstep::FleetRoster::with_participants(
            vec![crate::lockstep::FleetShip {
                host: HostSlot(1),
                ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                authored_slot_id: None,
                crew: Vec::new(),
            }],
            vec![HostSlot(1), HostSlot(2)],
            HostSlot(2),
            HostSlot(1),
        )
        .expect("a GM peer beside one ship is a valid topology");
    }
    let store = FakeStore::default();
    put_run(&store, SLOT_A, &gm);
    assert!(
        load_slot(&store, SLOT_A, &versions).is_ok(),
        "a GM peer's own capture is readable"
    );

    // The same empty hull from a peer that DOES own its roster slot is the
    // old malformed case and stays refused.
    let mut lost_hull = stored_run(9, "scenario", 17, versions.clone());
    boot_identity_mut(&mut lost_hull).selected_ship = String::new();
    // ...as is a hull claimed from outside any fleet at all.
    let mut fleetless = stored_run(9, "scenario", 17, versions.clone());
    boot_identity_mut(&mut fleetless).fleet = crate::lockstep::FleetRoster::with_participants(
        Vec::new(),
        vec![HostSlot::SOLO],
        HostSlot::SOLO,
        HostSlot::SOLO,
    )
    .expect("a participant-only topology is representable");
    for (index, run) in [lost_hull, fleetless].iter().enumerate() {
        let store = FakeStore::default();
        put_run(&store, SLOT_B, run);
        assert!(
            matches!(
                load_slot(&store, SLOT_B, &versions),
                Err(crate::snapshot::LoadRefusal::Unparsable(_))
            ),
            "malformed boot identity case {index} must be refused"
        );
    }
}

#[test]
fn automatic_captures_cover_start_periodic_and_final_boundaries_once() {
    let mut schedule = SaveSchedule::default();

    assert!(step(&mut schedule, 8, SavePhase::BeforeRun, 10).is_empty());
    assert_eq!(
        step(&mut schedule, 10, SavePhase::InProgress, 10),
        vec![CaptureDecision {
            tick: 10,
            slot: CaptureSlot::RollingAutosave,
            reason: CaptureReason::RunStarted,
        }]
    );
    assert!(step(&mut schedule, 19, SavePhase::InProgress, 10).is_empty());
    assert_eq!(
        step(&mut schedule, 20, SavePhase::InProgress, 10),
        vec![CaptureDecision {
            tick: 20,
            slot: CaptureSlot::RollingAutosave,
            reason: CaptureReason::Periodic,
        }]
    );
    assert!(step(&mut schedule, 20, SavePhase::InProgress, 10).is_empty());
    assert_eq!(
        step(&mut schedule, 21, SavePhase::GameOver, 10),
        vec![CaptureDecision {
            tick: 21,
            slot: CaptureSlot::RollingAutosave,
            reason: CaptureReason::GameOver,
        }]
    );
    assert!(step(&mut schedule, 22, SavePhase::GameOver, 10).is_empty());
}

#[test]
fn periodic_ticks_are_relative_to_the_run_start_not_absolute_tick_zero() {
    let mut schedule = SaveSchedule::default();
    step(&mut schedule, 7, SavePhase::InProgress, 5);

    assert!(step(&mut schedule, 10, SavePhase::InProgress, 5).is_empty());
    assert_eq!(
        step(&mut schedule, 12, SavePhase::InProgress, 5)[0].reason,
        CaptureReason::Periodic
    );
    assert_eq!(
        step(&mut schedule, 17, SavePhase::InProgress, 5)[0].reason,
        CaptureReason::Periodic
    );
}

#[test]
fn every_manual_request_survives_and_fires_on_the_next_live_tick_in_fifo_order() {
    let mut schedule = SaveSchedule::default();
    assert!(schedule
        .step(40, SavePhase::InProgress, 10, ["alpha", "alpha", "beta"])
        .iter()
        .all(|decision| decision.reason == CaptureReason::RunStarted));

    assert_eq!(
        step(&mut schedule, 41, SavePhase::InProgress, 10),
        vec![
            CaptureDecision {
                tick: 41,
                slot: CaptureSlot::Manual("alpha".into()),
                reason: CaptureReason::Manual,
            },
            CaptureDecision {
                tick: 41,
                slot: CaptureSlot::Manual("alpha".into()),
                reason: CaptureReason::Manual,
            },
            CaptureDecision {
                tick: 41,
                slot: CaptureSlot::Manual("beta".into()),
                reason: CaptureReason::Manual,
            },
        ]
    );
}

#[test]
fn manual_requests_due_after_terminal_or_lobby_boundaries_are_dropped() {
    for boundary in [SavePhase::GameOver, SavePhase::BeforeRun] {
        let mut schedule = SaveSchedule::default();
        step(&mut schedule, 40, SavePhase::InProgress, 10);
        assert!(schedule
            .step(40, SavePhase::InProgress, 10, ["alpha", "beta"])
            .is_empty());

        let output = schedule.step_with_outcomes(41, boundary, 10, std::iter::empty::<String>());
        assert!(output
            .decisions
            .iter()
            .all(|decision| decision.reason != CaptureReason::Manual));
        assert_eq!(
            output.refused_manual,
            ["alpha", "beta"]
                .into_iter()
                .map(|slot_id| RefusedManualSave {
                    tick: 41,
                    slot_id: slot_id.into(),
                    reason: ManualSaveRefusalReason::PhaseChanged { phase: boundary },
                })
                .collect::<Vec<_>>()
        );

        // A refused request is consumed at its due boundary. It must not
        // appear later when another run becomes capturable.
        let restarted = step(&mut schedule, 50, SavePhase::InProgress, 10);
        assert!(restarted
            .iter()
            .all(|decision| decision.reason != CaptureReason::Manual));
    }
}

#[test]
fn game_over_still_emits_only_the_automatic_final_capture() {
    let mut schedule = SaveSchedule::default();
    step(&mut schedule, 8, SavePhase::InProgress, 10);
    assert!(schedule
        .step(8, SavePhase::InProgress, 10, ["manual"])
        .is_empty());

    assert_eq!(
        step(&mut schedule, 9, SavePhase::GameOver, 10),
        vec![CaptureDecision {
            tick: 9,
            slot: CaptureSlot::RollingAutosave,
            reason: CaptureReason::GameOver,
        }]
    );
}

#[test]
fn automatic_decision_precedes_manual_decisions_due_on_the_same_tick() {
    let mut schedule = SaveSchedule::default();
    step(&mut schedule, 0, SavePhase::InProgress, 5);
    schedule.step(4, SavePhase::InProgress, 5, ["manual-1", "manual-2"]);

    let decisions = step(&mut schedule, 5, SavePhase::InProgress, 5);
    assert_eq!(
        decisions
            .iter()
            .map(|decision| decision.reason)
            .collect::<Vec<_>>(),
        vec![
            CaptureReason::Periodic,
            CaptureReason::Manual,
            CaptureReason::Manual
        ]
    );
}

#[test]
fn returning_before_run_resets_the_automatic_run_boundary() {
    let mut schedule = SaveSchedule::default();
    step(&mut schedule, 3, SavePhase::InProgress, 10);
    step(&mut schedule, 4, SavePhase::GameOver, 10);
    step(&mut schedule, 5, SavePhase::BeforeRun, 10);

    assert_eq!(
        step(&mut schedule, 30, SavePhase::InProgress, 10)[0].reason,
        CaptureReason::RunStarted
    );
    assert_eq!(
        step(&mut schedule, 40, SavePhase::InProgress, 10)[0].reason,
        CaptureReason::Periodic
    );
}

#[test]
fn restored_continuation_rebases_periodic_cadence_without_run_start() {
    let mut schedule = SaveSchedule::default();
    step(&mut schedule, 0, SavePhase::InProgress, 15);
    schedule.queue_manual_for_tick(61, ["pre-restore"]);
    assert_eq!(
        schedule.refuse_pending_manual(61, ManualSaveRefusalReason::StartupRestorePending),
        vec![RefusedManualSave {
            tick: 61,
            slot_id: "pre-restore".into(),
            reason: ManualSaveRefusalReason::StartupRestorePending,
        }]
    );

    schedule.rebase_after_restore(61, SavePhase::InProgress);
    for tick in 61..76 {
        assert!(step(&mut schedule, tick, SavePhase::InProgress, 15).is_empty());
    }
    assert_eq!(
        step(&mut schedule, 76, SavePhase::InProgress, 15),
        vec![CaptureDecision {
            tick: 76,
            slot: CaptureSlot::RollingAutosave,
            reason: CaptureReason::Periodic,
        }]
    );

    schedule.rebase_after_restore(90, SavePhase::GameOver);
    assert!(step(&mut schedule, 90, SavePhase::GameOver, 15).is_empty());
}

#[test]
fn manual_ids_are_store_safe_and_display_names_never_become_keys() {
    let store = FakeStore::default();
    let run = stored_run(12, "scenario-a", 41, current_versions());
    let display_name = "Same / name 🌌\nwith punctuation";

    let first = create_manual_save(&store, display_name, &run).expect("first manual save");
    let second = create_manual_save(&store, display_name, &run).expect("second manual save");

    assert_ne!(first, second);
    for slot in [&first, &second] {
        let parsed = uuid::Uuid::parse_str(slot).expect("internal id is UUID-shaped");
        assert_eq!(parsed.to_string(), *slot);
        assert!(vellum_save::is_slot(slot));
        assert!(store.contains(slot));
        assert!(store.contains(&metadata_slot(slot)));
    }
    assert!(!store.contains(display_name));

    let entries = list_slots(&store, &current_versions()).expect("catalogue lists");
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .all(|entry| entry.display_name == display_name));
}

#[test]
fn stores_are_isolated_and_autosave_rolls_without_touching_manual_rows() {
    let first = FakeStore::default();
    let second = FakeStore::default();
    let old = stored_run(4, "first", 1, current_versions());
    let new = stored_run(8, "first", 2, current_versions());
    let other = stored_run(6, "second", 3, current_versions());

    write_autosave(&first, &old).expect("first autosave");
    write_manual_save(&first, SLOT_A, "manual", &old).expect("first manual");
    write_autosave(&second, &other).expect("second autosave");
    write_autosave(&first, &new).expect("rolling replacement");

    assert_eq!(
        load_slot(&first, AUTOSAVE_SLOT, &current_versions())
            .expect("first loads")
            .seed,
        2
    );
    assert_eq!(
        load_slot(&second, AUTOSAVE_SLOT, &current_versions())
            .expect("second loads")
            .seed,
        3
    );
    assert!(first.contains(SLOT_A));
    assert!(!second.contains(SLOT_A));

    delete_slot(&first, SLOT_A).expect("delete is local");
    assert!(!first.contains(SLOT_A));
    assert_eq!(list_slots(&second, &current_versions()).unwrap().len(), 1);
}

#[test]
fn listing_filters_sidecars_and_is_stably_ordered_from_run_ticks() {
    let store = FakeStore::default();
    write_autosave(&store, &stored_run(3, "auto", 30, current_versions())).unwrap();
    write_manual_save(
        &store,
        SLOT_A,
        "older",
        &stored_run(10, "older-scenario", 10, current_versions()),
    )
    .unwrap();
    write_manual_save(
        &store,
        SLOT_C,
        "same tick c",
        &stored_run(20, "new-scenario", 31, current_versions()),
    )
    .unwrap();
    write_manual_save(
        &store,
        SLOT_B,
        "same tick b",
        &stored_run(20, "new-scenario", 32, current_versions()),
    )
    .unwrap();
    store.put(SLOT_D, "not a run");
    store.put("metadata-orphan", "ignored");
    store.put("another_subsystem", "ignored");

    let entries = list_slots(&store, &current_versions()).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.slot_id.as_str())
            .collect::<Vec<_>>(),
        vec![AUTOSAVE_SLOT, SLOT_B, SLOT_C, SLOT_A, SLOT_D]
    );
    let newest = manual_entry(&entries, SLOT_B);
    assert_eq!(
        newest.record,
        Some(SaveRecordSummary {
            scenario: "new-scenario".into(),
            seed: 32,
            capture_tick: 20,
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            versions: current_versions(),
        })
    );
    assert!(newest.can_start());
    assert!(matches!(
        manual_entry(&entries, SLOT_D).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(_))
    ));
}

#[test]
fn missing_and_corrupt_metadata_fall_back_to_internal_id_and_can_be_renamed() {
    let store = FakeStore::default();
    let run = stored_run(5, "scenario", 9, current_versions());
    put_run(&store, SLOT_A, &run);
    put_run(&store, SLOT_B, &run);
    store.put(&metadata_slot(SLOT_B), "damaged-sidecar");

    let entries = list_slots(&store, &current_versions()).unwrap();
    let missing = manual_entry(&entries, SLOT_A);
    assert_eq!(missing.display_name, SLOT_A);
    assert_eq!(missing.metadata, MetadataStatus::Missing);
    let corrupt = manual_entry(&entries, SLOT_B);
    assert_eq!(corrupt.display_name, SLOT_B);
    assert_eq!(corrupt.metadata, MetadataStatus::Corrupt);

    rename_slot(&store, SLOT_B, "Recovered 🛰️").expect("rename repairs sidecar");
    let entries = list_slots(&store, &current_versions()).unwrap();
    let renamed = manual_entry(&entries, SLOT_B);
    assert_eq!(renamed.display_name, "Recovered 🛰️");
    assert_eq!(renamed.metadata, MetadataStatus::Present);
    assert_eq!(renamed.slot_id, SLOT_B);
}

#[test]
fn every_version_refusal_and_bad_record_remains_a_deletable_row() {
    let store = FakeStore::default();
    let current = current_versions();
    put_run(&store, SLOT_A, &stored_run(1, "ready", 1, current.clone()));
    put_run(
        &store,
        SLOT_B,
        &stored_run(
            2,
            "format",
            2,
            vellum_save::Versions::new(8, "rules-a", 0x1234),
        ),
    );
    put_run(
        &store,
        SLOT_C,
        &stored_run(
            3,
            "rules",
            3,
            vellum_save::Versions::new(7, "rules-b", 0x1234),
        ),
    );
    put_run(
        &store,
        SLOT_D,
        &stored_run(
            4,
            "content",
            4,
            vellum_save::Versions::new(7, "rules-a", 0x9999),
        ),
    );
    store.put(SLOT_E, "corrupt run");
    put_run(
        &store,
        SLOT_F,
        &stored_run(6, "unreadable", 6, current.clone()),
    );
    store.fail_next(FakeOperation::Read, SLOT_F, "device unavailable");

    let entries = list_slots(&store, &current).unwrap();
    assert_eq!(manual_entry(&entries, SLOT_A).start, StartState::Ready);
    assert!(matches!(
        manual_entry(&entries, SLOT_B).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Format { .. }
        ))
    ));
    assert!(matches!(
        manual_entry(&entries, SLOT_C).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Rules { .. }
        ))
    ));
    assert!(matches!(
        manual_entry(&entries, SLOT_D).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Content { .. }
        ))
    ));
    assert!(matches!(
        manual_entry(&entries, SLOT_E).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(_))
    ));
    assert_eq!(
        manual_entry(&entries, SLOT_F).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(
            "device unavailable".into()
        ))
    );

    assert!(matches!(
        load_slot(&store, SLOT_B, &current),
        Err(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Format { .. }
        ))
    ));
    for slot in [SLOT_B, SLOT_C, SLOT_D, SLOT_E, SLOT_F] {
        delete_slot(&store, slot).expect("refused rows remain deletable");
        assert!(!store.contains(slot));
    }
}

#[test]
fn pre_contact_override_scenario_ron_keeps_metadata_and_reaches_format_refusal() {
    let store = FakeStore::default();
    let current = vellum_save::Versions::new(crate::snapshot::SNAPSHOT_FORMAT, "rules-a", 0x1234);
    let mut old = stored_run(
        42,
        "historical-scenario",
        1309,
        vellum_save::Versions::new(27, "rules-a", 0x1234),
    );
    old.snapshot.as_mut().unwrap().state.scenario = Some(crate::snapshot::ScenarioState::default());
    let mut ron = old.to_ron().unwrap();
    // Remove the new field itself: merely editing the version number leaves
    // a current-shaped payload and cannot exercise historical deserialization.
    let start = ron
        .find("contact_overrides:")
        .expect("current scenario writes its contact map");
    let end = start + ron[start..].find('}').unwrap() + 1;
    assert!(ron[start..end].ends_with("{}"));
    let comma = end + ron[end..].find(',').unwrap();
    assert!(ron[end..comma].trim().is_empty());
    ron.replace_range(start..=comma, "");
    assert!(!ron.contains("contact_overrides"));
    store.put(SLOT_A, &ron);
    let parsed = crate::snapshot::StoredRun::from_ron(&ron).unwrap();
    assert!(parsed
        .snapshot
        .unwrap()
        .state
        .scenario
        .unwrap()
        .contact_overrides
        .is_empty());
    let entries = list_slots(&store, &current).unwrap();
    let entry = manual_entry(&entries, SLOT_A);
    let summary = entry
        .record
        .as_ref()
        .expect("historical metadata survives parsing");
    assert_eq!(summary.scenario, "historical-scenario");
    assert_eq!(summary.seed, 1309);
    assert_eq!(summary.capture_tick, 42);
    assert!(matches!(
        &entry.start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Format { .. }
        ))
    ));
    assert!(matches!(
        load_slot(&store, SLOT_A, &current),
        Err(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Format { .. }
        ))
    ));
    assert!(matches!(
        crate::snapshot::load_from(&store, SLOT_A, &current),
        Err(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Format { .. }
        ))
    ));
}

#[test]
fn unfrozen_catalogue_defers_only_content_and_full_load_still_refuses_it() {
    let store = FakeStore::default();
    let current = current_versions();
    put_run(&store, SLOT_A, &stored_run(1, "ready", 1, current.clone()));
    put_run(
        &store,
        SLOT_B,
        &stored_run(
            2,
            "format",
            2,
            vellum_save::Versions::new(8, "rules-a", 0x1234),
        ),
    );
    put_run(
        &store,
        SLOT_C,
        &stored_run(
            3,
            "rules",
            3,
            vellum_save::Versions::new(7, "rules-b", 0x1234),
        ),
    );
    put_run(
        &store,
        SLOT_D,
        &stored_run(
            4,
            "content",
            4,
            vellum_save::Versions::new(7, "rules-a", 0x9999),
        ),
    );
    store.put(SLOT_E, "corrupt run");

    let entries = list_slots_with_content_check(&store, &current, ContentCheck::Deferred)
        .expect("unfrozen catalogue lists");
    assert_eq!(manual_entry(&entries, SLOT_A).start, StartState::Ready);
    assert_eq!(
        manual_entry(&entries, SLOT_D).start,
        StartState::ContentDeferred
    );
    assert!(manual_entry(&entries, SLOT_A).can_start());
    assert!(manual_entry(&entries, SLOT_D).can_start());
    for slot in [SLOT_B, SLOT_C, SLOT_E] {
        assert!(!manual_entry(&entries, slot).can_start());
    }

    assert!(matches!(
        manual_entry(&entries, SLOT_B).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Format { .. }
        ))
    ));
    assert!(matches!(
        manual_entry(&entries, SLOT_C).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Rules { .. }
        ))
    ));
    assert!(matches!(
        manual_entry(&entries, SLOT_E).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(_))
    ));

    // Deferred is presentation/readiness only. The selected row still
    // crosses the existing full Versions::check after its world loads.
    assert!(matches!(
        load_slot(&store, SLOT_D, &current),
        Err(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Content { .. }
        ))
    ));
    assert!(matches!(
        manual_entry(
            &list_slots_with_content_check(&store, &current, ContentCheck::Full).unwrap(),
            SLOT_D
        )
        .start,
        StartState::Refused(crate::snapshot::LoadRefusal::Moved(
            vellum_save::Moved::Content { .. }
        ))
    ));
}

#[test]
fn an_advertised_but_missing_run_is_empty_and_still_deletable() {
    let store = FakeStore::default();
    store.advertise(SLOT_A);
    let entries = list_slots(&store, &current_versions()).unwrap();
    assert_eq!(
        manual_entry(&entries, SLOT_A).start,
        StartState::Refused(crate::snapshot::LoadRefusal::Empty)
    );
    delete_slot(&store, SLOT_A).expect("Store removal is idempotent");
}

#[test]
fn rename_and_delete_failures_preserve_a_visible_retry_path() {
    let rename_store = FakeStore::default();
    let run = stored_run(5, "scenario", 7, current_versions());
    write_manual_save(&rename_store, SLOT_A, "before", &run).unwrap();
    let sidecar = metadata_slot(SLOT_A);
    let before = rename_store.get(&sidecar).unwrap();
    rename_store.fail_next(FakeOperation::Write, &sidecar, "write refused");
    assert!(matches!(
        rename_slot(&rename_store, SLOT_A, "after"),
        Err(CatalogueError::Store {
            operation: CatalogueOperation::WriteMetadata,
            ..
        })
    ));
    assert_eq!(rename_store.get(&sidecar).unwrap(), before);

    let metadata_failure = FakeStore::default();
    write_manual_save(&metadata_failure, SLOT_B, "name", &run).unwrap();
    let sidecar = metadata_slot(SLOT_B);
    metadata_failure.fail_next(FakeOperation::Remove, &sidecar, "locked metadata");
    assert!(matches!(
        delete_slot(&metadata_failure, SLOT_B),
        Err(CatalogueError::Store {
            operation: CatalogueOperation::RemoveMetadata,
            ..
        })
    ));
    assert!(metadata_failure.contains(SLOT_B));
    assert!(metadata_failure.contains(&sidecar));

    let run_failure = FakeStore::default();
    write_manual_save(&run_failure, SLOT_C, "name", &run).unwrap();
    run_failure.fail_next(FakeOperation::Remove, SLOT_C, "locked run");
    assert!(matches!(
        delete_slot(&run_failure, SLOT_C),
        Err(CatalogueError::Partial {
            operation: CatalogueOperation::RemoveRun,
            rollback_detail: None,
            ..
        })
    ));
    assert!(run_failure.contains(SLOT_C));
    assert!(!run_failure.contains(&metadata_slot(SLOT_C)));
    assert_eq!(
        manual_entry(
            &list_slots(&run_failure, &current_versions()).unwrap(),
            SLOT_C
        )
        .metadata,
        MetadataStatus::Missing
    );
    delete_slot(&run_failure, SLOT_C).expect("retry removes visible fallback row");
}

#[test]
fn failed_metadata_create_rolls_back_or_reports_the_surviving_run() {
    let run = stored_run(7, "scenario", 11, current_versions());
    let rolled_back = FakeStore::default();
    rolled_back.fail_next(
        FakeOperation::Write,
        &metadata_slot(SLOT_A),
        "metadata full",
    );
    assert!(matches!(
        write_manual_save(&rolled_back, SLOT_A, "name", &run),
        Err(CatalogueError::Store {
            operation: CatalogueOperation::WriteMetadata,
            ..
        })
    ));
    assert!(!rolled_back.contains(SLOT_A));

    let partial = FakeStore::default();
    partial.fail_next(
        FakeOperation::Write,
        &metadata_slot(SLOT_B),
        "metadata full",
    );
    partial.fail_next(FakeOperation::Remove, SLOT_B, "rollback locked");
    assert!(matches!(
        write_manual_save(&partial, SLOT_B, "name", &run),
        Err(CatalogueError::Partial {
            operation: CatalogueOperation::WriteMetadata,
            rollback_detail: Some(_),
            ..
        })
    ));
    assert!(partial.contains(SLOT_B));
    let entries = list_slots(&partial, &current_versions()).unwrap();
    assert_eq!(
        manual_entry(&entries, SLOT_B).metadata,
        MetadataStatus::Missing
    );
}

#[test]
fn selected_slot_export_uses_the_canonical_run_and_does_not_add_a_gate() {
    let store = FakeStore::default();
    let old_versions = vellum_save::Versions::new(6, "rules-old", 0x7777);
    let selected = stored_run(44, "selected", 88, old_versions);
    let other = stored_run(45, "other", 99, current_versions());
    put_run(&store, SLOT_A, &selected);
    put_run(&store, SLOT_B, &other);

    let text = export_slot(&store, SLOT_A).expect("incompatible saves remain exportable");
    assert_eq!(
        crate::snapshot::StoredRun::from_ron(&text).expect("export is a StoredRun"),
        selected
    );
    assert!(matches!(
        load_slot(&store, SLOT_A, &current_versions()),
        Err(crate::snapshot::LoadRefusal::Moved(_))
    ));
    assert_eq!(
        load_slot(&store, SLOT_B, &current_versions())
            .expect("selected other run remains compatible")
            .seed,
        99
    );

    store.put(SLOT_C, "broken");
    assert!(matches!(
        export_slot(&store, SLOT_C),
        Err(crate::snapshot::LoadRefusal::Unparsable(_))
    ));
    assert_eq!(
        export_slot(&store, SLOT_D),
        Err(crate::snapshot::LoadRefusal::Empty)
    );
}

#[test]
fn list_errors_and_reserved_or_invalid_mutations_are_structural() {
    let store = FakeStore::default();
    store.fail_next(FakeOperation::Slots, "", "catalogue unavailable");
    assert!(matches!(
        list_slots(&store, &current_versions()),
        Err(CatalogueError::Store {
            operation: CatalogueOperation::List,
            ..
        })
    ));
    assert_eq!(
        rename_slot(&store, AUTOSAVE_SLOT, "name"),
        Err(CatalogueError::ReservedAutosave)
    );
    assert_eq!(
        delete_slot(&store, "../escape"),
        Err(CatalogueError::InvalidSlot("../escape".into()))
    );
}
