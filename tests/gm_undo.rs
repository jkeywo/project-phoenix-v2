//! The typed inverse path for NPC doctrine and faction relations (issue #1442).
//!
//! Everything here drives the production path: a real headless world load, the
//! real `apply_due_actions` reducer over real canonical grants, the real
//! `NpcDoctrineControl`/faction registry it mutates, real
//! `snapshot::capture`/`restore`, the real `sim_digest::world_digest`, and the
//! real `publish_session_projection` message the GM journal panel reads.
//! Nothing here constructs a projection or a result by hand.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::HostSlot,
    console_bridge::GmSessionChanged,
    entities::config_cache::FactionRegistryResource,
    entities::spawner::EntityUuid,
    gm_action::*,
    gm_faction::GmFactionOverrides,
    gm_journal::{GmJournalEntry, GmJournalProjection},
    gm_npc::NpcDoctrineState,
    sim_tick::SimTick,
};
use project_phoenix as phoenix;

const WORLD: &str = "tests/fixtures/worlds/gm_npc_doctrine.toml";
const COURIER: &str = "Directive courier";

fn boot() -> App {
    let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1442),
        deterministic: true,
        ..Default::default()
    })
    .unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..400 {
        app.update();
    }
    app
}

/// The live uuid of the authored courier, which is what a typed action names.
fn courier_uuid(app: &mut App) -> String {
    app.world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid[COURIER]
        .clone()
}

fn grant(sequence: u64, tick: u64, operator: &str, action: GmAction) -> GmActionGrant {
    let from = HostSlot(7);
    GmActionGrant {
        from,
        sequenced_by: HostSlot(1),
        operator_id: operator.into(),
        correlation: GmActionId::new(format!("act-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(from, sequence),
        action,
    }
}

/// Insert one canonical grant and run the real reducer over it.
fn apply(app: &mut App, sequence: u64, operator: &str, action: GmAction) {
    let tick = app.world().resource::<SimTick>().0;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, tick, operator, action))
        .expect("canonical grant is admitted");
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}

/// The journal exactly as the GM page receives it, through the production
/// publisher and its Host-Channel message.
fn published(app: &mut App) -> GmJournalProjection {
    app.world_mut()
        .resource_mut::<LastGmSessionProjection>()
        .clear_for_republish();
    app.world_mut()
        .run_system_once(publish_session_projection)
        .unwrap();
    app.world_mut()
        .resource_mut::<Messages<GmSessionChanged>>()
        .drain()
        .last()
        .expect("the session projection republishes after a journal change")
        .payload
        .journal
}

fn row(app: &mut App, correlation: &str) -> GmJournalEntry {
    published(app)
        .entries
        .into_iter()
        .find(|entry| entry.correlation == correlation)
        .unwrap_or_else(|| panic!("journal has a row for {correlation}"))
}

fn outcome(app: &mut App, correlation: &str) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
    let entry = row(app, correlation);
    (entry.outcome, entry.reason)
}

fn doctrine(app: &mut App, uuid: &str) -> Option<String> {
    app.world_mut()
        .query::<(&EntityUuid, &NpcDoctrineState)>()
        .iter(app.world())
        .find(|(id, _)| id.0 == uuid)
        .and_then(|(_, state)| state.0.as_ref().map(|state| state.id.clone()))
}

fn hostile(app: &App, faction: &str, enemy: &str) -> bool {
    let registry = &app.world().resource::<FactionRegistryResource>().0;
    let faction = registry.uuid_by_name(faction).expect("authored faction");
    let enemy = registry.uuid_by_name(enemy).expect("authored faction");
    phoenix::ai::faction::is_enemy(Some(faction), Some(enemy), registry)
}

fn set_doctrine(uuid: &str, id: &str) -> GmAction {
    GmAction::SetNpcDoctrine {
        target: uuid.into(),
        doctrine: id.into(),
    }
}

fn undo(original_sequence: u64, original_operator: &str, expected: GmAffectedField) -> GmAction {
    GmAction::UndoGmAction {
        original: GmActionId::new(format!("act-{original_sequence}")).unwrap(),
        original_operator: original_operator.into(),
        original_sequence,
        expected,
    }
}

// ── NPC doctrine ────────────────────────────────────────────────────────────

/// An equal GM reverses another operator's applied doctrine. Both operators are
/// in the ONE saved journal, the entity is really back on its authored
/// doctrine, and the original row now says it has been inverted.
#[test]
fn any_equal_gm_reverses_an_applied_doctrine_and_both_operators_are_recorded() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    assert_eq!(doctrine(&mut app, &uuid).as_deref(), Some("north"));

    let original = row(&mut app, "act-1");
    assert_eq!(original.outcome, GmActionOutcome::Applied);
    // The exact affected field, with a `None` before that means "its own
    // authored doctrine" rather than "unknown".
    assert_eq!(
        original.affected,
        Some(GmAffectedField::NpcDoctrine {
            entity: uuid.clone(),
            before: None,
            after: Some("north".into()),
        })
    );
    assert!(!original.inverted);

    // A DIFFERENT operator, echoing back the facts the projection published.
    apply(
        &mut app,
        2,
        "gm-blake",
        undo(1, "gm-alex", original.affected.clone().unwrap()),
    );
    assert_eq!(doctrine(&mut app, &uuid), None);

    let inverse = row(&mut app, "act-2");
    assert_eq!(inverse.outcome, GmActionOutcome::Applied);
    assert_eq!(inverse.operator_id, "gm-blake");
    let reference = inverse.undo_of.expect("an inverse names its original");
    assert_eq!(reference.operator_id, "gm-alex");
    assert_eq!(reference.correlation.as_str(), "act-1");
    assert_eq!(reference.sequence, 1);
    // The inverse's own fact says what IT changed: the original pair, reversed.
    assert_eq!(
        inverse.affected,
        Some(GmAffectedField::NpcDoctrine {
            entity: uuid.clone(),
            before: Some("north".into()),
            after: None,
        })
    );
    // The original entry is untouched and merely reported as inverted; nothing
    // rewrote or removed it.
    let original_now = row(&mut app, "act-1");
    assert!(original_now.inverted);
    assert_eq!(original_now.outcome, GmActionOutcome::Applied);
    assert_eq!(original_now.operator_id, "gm-alex");
}

/// A change to the same affected field between the action and its inverse is
/// refused with its own reason. Nothing is overwritten.
#[test]
fn an_intervening_change_to_the_affected_field_refuses_the_inverse() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    let facts = row(&mut app, "act-1").affected.unwrap();
    apply(&mut app, 2, "gm-blake", set_doctrine(&uuid, "east"));
    apply(&mut app, 3, "gm-cass", undo(1, "gm-alex", facts));
    assert_eq!(
        outcome(&mut app, "act-3"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::AffectedStateChanged)
        )
    );
    // The newer decision stands.
    assert_eq!(doctrine(&mut app, &uuid).as_deref(), Some("east"));
}

/// Two GMs racing to undo one action get one undo and one truthful refusal.
#[test]
fn a_second_inverse_of_one_original_is_refused_rather_than_applied_twice() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    let facts = row(&mut app, "act-1").affected.unwrap();
    apply(&mut app, 2, "gm-blake", undo(1, "gm-alex", facts.clone()));
    apply(&mut app, 3, "gm-cass", undo(1, "gm-alex", facts));
    assert_eq!(outcome(&mut app, "act-2").0, GmActionOutcome::Applied);
    assert_eq!(
        outcome(&mut app, "act-3"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::AlreadyInverted)
        )
    );
    assert_eq!(doctrine(&mut app, &uuid), None);
}

/// An exact retry of one inverse is the same canonical grant, not a second one.
#[test]
fn an_exact_duplicate_inverse_request_keeps_one_row_and_one_effect() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    let facts = row(&mut app, "act-1").affected.unwrap();
    let tick = app.world().resource::<SimTick>().0;
    let inverse = grant(2, tick, "gm-blake", undo(1, "gm-alex", facts));
    for _ in 0..2 {
        assert!(matches!(
            app.world_mut()
                .resource_mut::<GmActionJournal>()
                .insert(inverse.clone()),
            Ok(GmActionInsert::Duplicate) | Ok(GmActionInsert::Inserted)
        ));
        app.world_mut().run_system_once(apply_due_actions).unwrap();
    }
    let projection = published(&mut app);
    assert_eq!(
        projection
            .entries
            .iter()
            .filter(|entry| entry.correlation == "act-2")
            .count(),
        1
    );
    assert_eq!(doctrine(&mut app, &uuid), None);
}

/// The facts a GM asks against must be the ones the journal recorded.
#[test]
fn facts_that_do_not_match_the_canonical_record_are_refused_as_a_stale_reading() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    apply(
        &mut app,
        2,
        "gm-blake",
        undo(
            1,
            "gm-alex",
            GmAffectedField::NpcDoctrine {
                entity: uuid.clone(),
                before: Some("east".into()),
                after: Some("north".into()),
            },
        ),
    );
    assert_eq!(
        outcome(&mut app, "act-2"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::InverseFactsMismatch)
        )
    );
    assert_eq!(doctrine(&mut app, &uuid).as_deref(), Some("north"));
}

/// Nothing to reverse, and a family that records no affected field, are both
/// refused rather than guessed at.
#[test]
fn a_no_op_original_and_an_unknown_original_are_both_refused_with_their_own_reason() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    // Same doctrine again: accepted, changed nothing, so it recorded no pair.
    apply(&mut app, 2, "gm-alex", set_doctrine(&uuid, "north"));
    assert_eq!(outcome(&mut app, "act-2").0, GmActionOutcome::NoOp);
    assert_eq!(row(&mut app, "act-2").affected, None);

    let facts = GmAffectedField::NpcDoctrine {
        entity: uuid.clone(),
        before: Some("north".into()),
        after: Some("east".into()),
    };
    apply(&mut app, 3, "gm-blake", undo(2, "gm-alex", facts.clone()));
    assert_eq!(
        outcome(&mut app, "act-3"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::InverseUnsupported)
        )
    );
    // A sequence nothing in the journal carries.
    apply(&mut app, 4, "gm-blake", undo(97, "gm-alex", facts));
    assert_eq!(
        outcome(&mut app, "act-4"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::UnknownGmAction)
        )
    );
    assert_eq!(doctrine(&mut app, &uuid).as_deref(), Some("north"));
}

// ── Faction relations ───────────────────────────────────────────────────────

fn hostility(faction: &str, enemy: &str, hostile: bool) -> GmAction {
    GmAction::SetFactionHostility {
        faction: faction.into(),
        enemy: enemy.into(),
        hostile,
    }
}

/// The typed faction adapter moves the authored registry through its own
/// vocabulary, records the exact pair it moved, and is reversible.
#[test]
fn a_gm_faction_hostility_is_attributed_recorded_and_reversible() {
    let mut app = boot();
    assert!(!hostile(&app, "Alliance", "Harrow"));
    apply(
        &mut app,
        1,
        "gm-alex",
        hostility("Alliance", "Harrow", true),
    );
    assert!(hostile(&app, "Alliance", "Harrow"));
    // Asymmetric by construction: the other direction was never asked for.
    assert!(!hostile(&app, "Harrow", "Alliance"));

    let original = row(&mut app, "act-1");
    assert_eq!(original.outcome, GmActionOutcome::Applied);
    assert_eq!(original.target.as_deref(), Some("Alliance"));
    assert_eq!(
        original.affected,
        Some(GmAffectedField::FactionHostility {
            faction: "Alliance".into(),
            enemy: "Harrow".into(),
            before: false,
            after: true,
        })
    );

    apply(
        &mut app,
        2,
        "gm-blake",
        undo(1, "gm-alex", original.affected.clone().unwrap()),
    );
    assert_eq!(outcome(&mut app, "act-2").0, GmActionOutcome::Applied);
    assert!(!hostile(&app, "Alliance", "Harrow"));
    assert!(row(&mut app, "act-1").inverted);
    // The pre-GM baseline survives every later change to the same pair, so a
    // restore of a save that predates the whole exchange still reverts to it.
    let overrides = app.world().resource::<GmFactionOverrides>();
    let entry = overrides
        .entries()
        .iter()
        .find(|entry| entry.faction == "Alliance" && entry.enemy == "Harrow")
        .expect("the moved pair is recorded");
    assert!(!entry.before);
    assert!(!entry.current);
}

/// A change to an UNRELATED relation does not block the inverse; a change to
/// the affected pair itself does.
#[test]
fn unrelated_relation_changes_are_allowed_while_the_affected_pair_is_guarded() {
    let mut app = boot();
    apply(
        &mut app,
        1,
        "gm-alex",
        hostility("Alliance", "Harrow", true),
    );
    let facts = row(&mut app, "act-1").affected.unwrap();
    // A different ordered pair entirely, between the action and its inverse.
    apply(
        &mut app,
        2,
        "gm-blake",
        hostility("Harrow", "Requiem", true),
    );
    apply(&mut app, 3, "gm-cass", undo(1, "gm-alex", facts.clone()));
    assert_eq!(outcome(&mut app, "act-3").0, GmActionOutcome::Applied);
    assert!(!hostile(&app, "Alliance", "Harrow"));
    // The unrelated change stands.
    assert!(hostile(&app, "Harrow", "Requiem"));

    // Now move the affected pair itself and try to reverse the same original.
    apply(
        &mut app,
        4,
        "gm-blake",
        hostility("Alliance", "Harrow", true),
    );
    apply(&mut app, 5, "gm-cass", undo(1, "gm-alex", facts));
    assert_eq!(
        outcome(&mut app, "act-5"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::AlreadyInverted)
        )
    );
}

/// A faction name the setting never authored is refused; nothing is created.
#[test]
fn an_unauthored_faction_name_is_refused_rather_than_registered() {
    let mut app = boot();
    apply(
        &mut app,
        1,
        "gm-alex",
        hostility("Alliance", "Not A Faction", true),
    );
    assert_eq!(
        outcome(&mut app, "act-1"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::UnknownFaction)
        )
    );
    assert!(app.world().resource::<GmFactionOverrides>().is_empty());
    assert!(app
        .world()
        .resource::<FactionRegistryResource>()
        .0
        .uuid_by_name("Not A Faction")
        .is_none());
}

// ── Identity of an original ─────────────────────────────────────────────────

/// Insert one canonical grant whose idempotency key is CHOSEN rather than
/// derived from its sequence, and run the real reducer over it.
///
/// A correlation is scoped to the operator that minted it, so two GMs may
/// legitimately be running the same opaque key at once; only a helper that can
/// spell that can test it.
fn apply_as(app: &mut App, sequence: u64, operator: &str, correlation: &str, action: GmAction) {
    let tick = app.world().resource::<SimTick>().0;
    let mut grant = grant(sequence, tick, operator, action);
    grant.correlation = GmActionId::new(correlation).unwrap();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant)
        .expect("canonical grant is admitted");
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}

/// The published row for ONE operator's use of a correlation.
fn row_of(app: &mut App, operator: &str, correlation: &str) -> GmJournalEntry {
    published(app)
        .entries
        .into_iter()
        .find(|entry| entry.correlation == correlation && entry.operator_id == operator)
        .unwrap_or_else(|| panic!("journal has {operator}'s row for {correlation}"))
}

fn undo_of(
    original_correlation: &str,
    original_sequence: u64,
    original_operator: &str,
    expected: GmAffectedField,
) -> GmAction {
    GmAction::UndoGmAction {
        original: GmActionId::new(original_correlation).unwrap(),
        original_operator: original_operator.into(),
        original_sequence,
        expected,
    }
}

/// Two operators may hold the same correlation, and an undo reverses exactly
/// one of them.
///
/// The whole identity of an original is (operator, correlation, sequence). On
/// the bare correlation, one GM's undo would mark every other operator's
/// same-named action as reversed — hiding its Undo control on the journal panel
/// while the reducer went on refusing a second inverse of the OTHER one — so
/// this asserts both halves: what the panel shows and what the reducer allows.
#[test]
fn one_correlation_held_by_two_operators_inverts_only_the_action_that_was_reversed() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply_as(
        &mut app,
        1,
        "gm-alex",
        "shared",
        set_doctrine(&uuid, "north"),
    );
    apply_as(
        &mut app,
        2,
        "gm-blake",
        "shared",
        hostility("Alliance", "Harrow", true),
    );
    let alex = row_of(&mut app, "gm-alex", "shared");
    let blake = row_of(&mut app, "gm-blake", "shared");
    assert_eq!(alex.outcome, GmActionOutcome::Applied);
    assert_eq!(blake.outcome, GmActionOutcome::Applied);

    apply_as(
        &mut app,
        3,
        "gm-cass",
        "undo-alex",
        undo_of("shared", 1, "gm-alex", alex.affected.clone().unwrap()),
    );
    assert_eq!(
        row_of(&mut app, "gm-cass", "undo-alex").outcome,
        GmActionOutcome::Applied
    );
    assert_eq!(doctrine(&mut app, &uuid), None);
    // Only the action that was actually reversed says so. gm-blake's, which
    // merely shares the key, is untouched and still offers its Undo control.
    assert!(row_of(&mut app, "gm-alex", "shared").inverted);
    assert!(!row_of(&mut app, "gm-blake", "shared").inverted);
    assert!(hostile(&app, "Alliance", "Harrow"));

    // ...and it really is still reversible: the guard must not refuse it as
    // already inverted on the strength of the other operator's key.
    apply_as(
        &mut app,
        4,
        "gm-cass",
        "undo-blake",
        undo_of("shared", 2, "gm-blake", blake.affected.clone().unwrap()),
    );
    assert_eq!(
        row_of(&mut app, "gm-cass", "undo-blake").outcome,
        GmActionOutcome::Applied
    );
    assert!(!hostile(&app, "Alliance", "Harrow"));
    assert!(row_of(&mut app, "gm-blake", "shared").inverted);
}

/// An undo is not itself reversible, in the reducer and not merely on the page.
///
/// `gui/gm-inverse-preview.js` already declines to offer the control over an
/// `action-undo` row. The reducer has to agree: an inverse records its own
/// affected pair, so without this refusal a stale page or a replaying peer
/// could walk a chain of reversals no GM was ever offered. Append-only: to put
/// the doctrine back, ask for it again as its own attributed action.
#[test]
fn an_undo_is_not_itself_reversible() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    let facts = row(&mut app, "act-1").affected.unwrap();
    apply(&mut app, 2, "gm-blake", undo(1, "gm-alex", facts));
    let inverse = row(&mut app, "act-2");
    assert_eq!(inverse.outcome, GmActionOutcome::Applied);
    // The inverse's own recorded pair — exactly what a chain would ask against.
    let inverse_facts = inverse
        .affected
        .clone()
        .expect("an applied inverse records its pair");

    apply(&mut app, 3, "gm-cass", undo(2, "gm-blake", inverse_facts));
    assert_eq!(
        outcome(&mut app, "act-3"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::InverseUnsupported)
        )
    );
    // Nothing moved, and the inverse is not reported as itself inverted.
    assert_eq!(doctrine(&mut app, &uuid), None);
    assert!(!row(&mut app, "act-2").inverted);
}

// ── Snapshot, digest and replay ─────────────────────────────────────────────

/// Capture, act further, restore: the inverse records and the world they
/// describe come back exactly as the save had them, and later entries are gone.
#[test]
fn capture_and_restore_return_the_saves_inverse_records_and_the_state_they_describe() {
    let mut app = boot();
    let uuid = courier_uuid(&mut app);
    apply(&mut app, 1, "gm-alex", set_doctrine(&uuid, "north"));
    apply(
        &mut app,
        2,
        "gm-alex",
        hostility("Alliance", "Harrow", true),
    );
    let doctrine_facts = row(&mut app, "act-1").affected.unwrap();
    apply(
        &mut app,
        3,
        "gm-blake",
        undo(1, "gm-alex", doctrine_facts.clone()),
    );

    let saved = phoenix::snapshot::capture(app.world());
    assert_eq!(
        saved.gm_faction_overrides.current("Alliance", "Harrow"),
        Some(true)
    );

    // Work that happens AFTER the save and must not survive the restore.
    let faction_facts = row(&mut app, "act-2").affected.unwrap();
    apply(&mut app, 4, "gm-cass", undo(2, "gm-alex", faction_facts));
    apply(&mut app, 5, "gm-cass", set_doctrine(&uuid, "east"));
    assert!(!hostile(&app, "Alliance", "Harrow"));
    assert_eq!(doctrine(&mut app, &uuid).as_deref(), Some("east"));

    phoenix::snapshot::restore(app.world_mut(), &saved);
    let restored = published(&mut app);
    assert_eq!(
        restored
            .entries
            .iter()
            .map(|entry| entry.correlation.as_str())
            .collect::<Vec<_>>(),
        vec!["act-1", "act-2", "act-3"]
    );
    // The inverse record itself survived, with both operators intact.
    let inverse = row(&mut app, "act-3");
    assert_eq!(inverse.operator_id, "gm-blake");
    assert_eq!(
        inverse.undo_of.map(|undo| undo.operator_id),
        Some("gm-alex".into())
    );
    assert!(row(&mut app, "act-1").inverted);
    // And the world the save described: the hostility is back, and the undone
    // faction change of the abandoned branch is gone with its journal entry.
    assert!(hostile(&app, "Alliance", "Harrow"));
    assert_eq!(
        app.world()
            .resource::<GmFactionOverrides>()
            .current("Alliance", "Harrow"),
        Some(true)
    );
}

/// A restore that predates a GM faction change reverts it to the authored
/// baseline rather than leaving it standing under an older journal.
#[test]
fn restoring_a_save_from_before_a_faction_change_reverts_that_relation() {
    let mut app = boot();
    let clean = phoenix::snapshot::capture(app.world());
    assert!(clean.gm_faction_overrides.is_empty());
    apply(
        &mut app,
        1,
        "gm-alex",
        hostility("Alliance", "Harrow", true),
    );
    assert!(hostile(&app, "Alliance", "Harrow"));
    phoenix::snapshot::restore(app.world_mut(), &clean);
    assert!(!hostile(&app, "Alliance", "Harrow"));
    assert!(app.world().resource::<GmFactionOverrides>().is_empty());
}

/// The GM's faction decisions are part of the authoritative fold, so two peers
/// that disagree about one are caught by the ordinary digest comparator — and a
/// run that never uses the surface is folded exactly as it was before.
#[test]
fn gm_faction_overrides_join_the_digest_only_once_a_gm_has_moved_a_relation() {
    let mut untouched = boot();
    let mut moved = boot();
    let baseline = phoenix::sim_digest::world_digest(untouched.world());
    assert_eq!(
        baseline,
        phoenix::sim_digest::world_digest(moved.world()),
        "two identical boots agree before any GM acts"
    );

    // The empty resource is present on both and folds nothing.
    assert!(untouched
        .world()
        .resource::<GmFactionOverrides>()
        .is_empty());

    apply(
        &mut moved,
        1,
        "gm-alex",
        hostility("Alliance", "Harrow", true),
    );
    assert_ne!(
        phoenix::sim_digest::world_digest(moved.world()),
        phoenix::sim_digest::world_digest(untouched.world()),
        "a GM faction decision is a divergence the mesh comparator can see"
    );

    // The same decision on the other peer converges them again: the fold is
    // over the decision, not over the order a peer happened to learn it in.
    apply(
        &mut untouched,
        1,
        "gm-alex",
        hostility("Alliance", "Harrow", true),
    );
    assert_eq!(
        phoenix::sim_digest::world_digest(moved.world()),
        phoenix::sim_digest::world_digest(untouched.world())
    );
}

/// Replaying one peer's exact canonical grant sequence — original AND inverse —
/// on a second peer reproduces the same authoritative world.
///
/// This is the replay half of the inverse contract: an inverse is an ordinary
/// journal grant, not a local edit, so a peer that only ever saw the journal
/// lands on the same digest as the peer whose GM pressed the buttons.
#[test]
fn replaying_the_journal_reproduces_the_same_world_on_a_second_peer() {
    let mut acting = boot();
    let mut replaying = boot();
    let uuid = courier_uuid(&mut acting);
    assert_eq!(uuid, courier_uuid(&mut replaying));

    let script: Vec<(u64, &str, GmAction)> = vec![
        (1, "gm-alex", set_doctrine(&uuid, "north")),
        (2, "gm-alex", hostility("Alliance", "Harrow", true)),
    ];
    for (sequence, operator, action) in &script {
        apply(&mut acting, *sequence, operator, action.clone());
        apply(&mut replaying, *sequence, operator, action.clone());
    }
    let doctrine_facts = row(&mut acting, "act-1").affected.unwrap();
    let faction_facts = row(&mut acting, "act-2").affected.unwrap();
    for (sequence, facts) in [(1u64, doctrine_facts), (2, faction_facts)] {
        let inverse = undo(sequence, "gm-alex", facts);
        apply(&mut acting, sequence + 2, "gm-blake", inverse.clone());
        apply(&mut replaying, sequence + 2, "gm-blake", inverse);
    }

    assert_eq!(doctrine(&mut acting, &uuid), None);
    assert!(!hostile(&acting, "Alliance", "Harrow"));
    assert_eq!(
        phoenix::sim_digest::world_digest(acting.world()),
        phoenix::sim_digest::world_digest(replaying.world()),
        "the inverse records fold identically on both peers"
    );
    // And the saved history both peers hold is the same one, inverses included.
    let left = published(&mut acting).entries;
    let right = published(&mut replaying).entries;
    assert_eq!(left, right);
    assert_eq!(left.len(), 4);
    assert!(left[0].inverted && left[1].inverted);
}
