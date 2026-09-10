//! Reversing an allowed GM removal from captured entity state (issue #1444).
//!
//! Every test here drives the PRODUCTION path: a canonical `GmActionGrant` goes
//! into the real `GmActionJournal`, the real `apply_due_actions` reducer runs at
//! the agreed tick, and the real `tick_trigger_pipeline` performs the removal
//! and the rebuild. Nothing calls the capture or the restore directly, because
//! the claim being tested is that a GM pressing Undo gets their ship back — not
//! that two functions compose.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix::command_admission::HostSlot;
use phoenix::entities::spawner::EntityUuid;
use phoenix::gm_action::*;
use phoenix::gm_contact::ContactMode;
use phoenix::sim_tick::SimTick;
use phoenix::world::server::WorldContentRuntime;
use project_phoenix as phoenix;
use std::collections::BTreeSet;

/// The fixture's own three removable shapes — a runtime-spawned hull, a
/// runtime-spawned structure and an authored `[[entity]]` block. See the file.
const WORLD: &str = "tests/fixtures/worlds/gm_removal_undo.toml";

fn seeded() -> App {
    let args = phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1444),
        deterministic: true,
        max_ticks: 400,
        ..Default::default()
    };
    let mut app = phoenix::headless::build_headless_app(&args).unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..60 {
        app.update();
    }
    app
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimTick>().0
}

fn grant(operator: u32, sequence: u64, at: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(operator),
        sequenced_by: HostSlot(1),
        operator_id: format!("gm-{operator}"),
        correlation: GmActionId::new(format!("corr-{operator}-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: at,
        order: GmActionOrder::new(HostSlot(operator), sequence),
        action,
    }
}

fn submit(app: &mut App, request: GmActionGrant) {
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(request)
        .unwrap();
}

/// Run the app until the armed work the grants queued has actually happened.
fn settle(app: &mut App) {
    for _ in 0..12 {
        app.update();
    }
}

fn uuids(app: &mut App) -> BTreeSet<String> {
    app.world_mut()
        .query::<&EntityUuid>()
        .iter(app.world())
        .map(|uuid| uuid.0.clone())
        .collect()
}

fn entity_for(app: &mut App, uuid: &str) -> Option<Entity> {
    app.world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == uuid)
        .map(|(entity, _)| entity)
}

fn facts(app: &App) -> Vec<LoggedGmAction> {
    app.world().resource::<GmActionLog>().entries().to_vec()
}

fn fact<'a>(facts: &'a [LoggedGmAction], correlation: &str) -> &'a LoggedGmAction {
    facts
        .iter()
        .find(|fact| fact.correlation.as_str() == correlation)
        .unwrap_or_else(|| panic!("no terminal fact for {correlation}: {facts:#?}"))
}

/// Place one removable NPC through the ordinary GM spawn palette and return its
/// freshly minted uuid. A RUNTIME spawn, which is what carries the recipe an
/// inverse rebuilds from.
fn place_removable_npc(app: &mut App, sequence: u64) -> String {
    place_removable(app, sequence, "removable-raider").1
}

/// The same placement, with the canonical grant kept so a second peer can
/// replay the identical input.
fn place_removable(app: &mut App, sequence: u64, palette: &str) -> (GmActionGrant, String) {
    let before = uuids(app);
    let request = grant(
        1,
        sequence,
        tick(app) + 2,
        GmAction::SpawnPaletteEntity {
            palette: palette.into(),
            variant: Some("removable".into()),
            position_mm: [4_000_000, 0, -4_000_000],
            heading_mdeg: 0,
        },
    );
    submit(app, request.clone());
    settle(app);
    let placed: Vec<String> = uuids(app).difference(&before).cloned().collect();
    assert_eq!(
        placed.len(),
        1,
        "the palette placement must spawn one entity"
    );
    (request, placed.into_iter().next().unwrap())
}

/// The authored `[[entity]]` NPC, which a fresh boot re-creates from the world
/// file and which therefore carries no spawn recipe at all.
fn authored_npc(app: &mut App) -> String {
    app.world().resource::<WorldContentRuntime>().name_to_uuid["Authored raider"].clone()
}

/// The live fleet ship whose knowledge a contact override belongs to.
///
/// Read from the world rather than written down, because `gm_contact::prune` —
/// the ordinary system that keeps overrides honest — retains a row only while
/// its observer is a live FLEET hull and its target is live. A test that made up
/// an observer id would have its overrides pruned a tick later and would prove
/// nothing about the inverse.
fn fleet_observer(app: &mut App) -> String {
    app.world_mut()
        .query::<(
            &EntityUuid,
            bevy::prelude::Has<phoenix::lockstep::FleetSlotOf>,
        )>()
        .iter(app.world())
        .find(|(_, fleet)| *fleet)
        .map(|(uuid, _)| uuid.0.clone())
        .expect("the probe world seats one fleet ship")
}

fn set_override(app: &mut App, observer: &str, target: &str, mode: ContactMode) {
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .contact_overrides
        .entry(observer.into())
        .or_default()
        .insert(target.into(), mode);
}

fn contact_override(app: &App, observer: &str, target: &str) -> Option<ContactMode> {
    app.world()
        .resource::<WorldContentRuntime>()
        .contact_overrides
        .get(observer)
        .and_then(|rows| rows.get(target))
        .copied()
}

fn despawn(app: &mut App, operator: u32, sequence: u64, target: &str) -> GmActionGrant {
    let request = grant(
        operator,
        sequence,
        tick(app) + 2,
        GmAction::DespawnEntity {
            target: target.into(),
        },
    );
    submit(app, request.clone());
    settle(app);
    request
}

/// The inverse request a GM's page composes: the recorded pair echoed back
/// verbatim, beside the original's public identity.
fn undo_of(app: &mut App, operator: u32, sequence: u64, original: &GmActionGrant) -> GmActionGrant {
    let recorded = fact(&facts(app), original.correlation.as_str())
        .affected
        .clone()
        .expect("an applied removal records the pair its inverse is built on");
    grant(
        operator,
        sequence,
        tick(app) + 2,
        GmAction::UndoGmAction {
            original: original.correlation.clone(),
            original_operator: original.operator_id.clone(),
            original_sequence: original.order.sequence,
            expected: recorded,
        },
    )
}

#[test]
fn an_allowed_removal_records_the_presence_pair_and_retains_a_rebuildable_capture() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let observer = fleet_observer(&mut app);
    set_override(&mut app, &observer, &npc, ContactMode::Reveal);
    let removal = despawn(&mut app, 1, 2, &npc);

    let log = facts(&app);
    let removed = fact(&log, removal.correlation.as_str());
    assert_eq!(removed.outcome, GmActionOutcome::Applied);
    assert_eq!(
        removed.affected,
        Some(GmAffectedField::EntityPresence {
            entity: npc.clone(),
            before: true,
            after: false,
        }),
        "the removal's own durable fact says exactly what it changed"
    );
    assert!(entity_for(&mut app, &npc).is_none());
    // The reference cleanup really ran, and the capture really holds it.
    assert_eq!(contact_override(&app, &observer, &npc), None);
    let captures = app
        .world()
        .resource::<WorldContentRuntime>()
        .gm_despawn_captures
        .clone();
    assert!(
        captures.is_restorable(
            &removal.operator_id,
            &removal.correlation,
            removal.order.sequence
        ),
        "an allowed removal of a runtime spawn retains a rebuildable capture"
    );
    // And the page is told the control is real.
    let projection = phoenix::gm_journal::journal_projection(
        app.world().resource::<GmActionLog>(),
        None,
        60.0,
        Some(&captures),
    );
    let row = projection
        .entries
        .iter()
        .find(|row| row.correlation == removal.correlation.as_str())
        .expect("the removal is on the published journal");
    assert!(!row.capture_lost);
    assert!(!row.inverted);
}

#[test]
fn the_inverse_restores_the_same_identity_and_state_at_the_current_tick() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let observer = fleet_observer(&mut app);
    set_override(&mut app, &observer, &npc, ContactMode::Reveal);

    // Give the ship a state a bare rebuild from the template could not invent.
    let entity = entity_for(&mut app, &npc).expect("the placement is live");
    {
        let mut physics = app
            .world_mut()
            .get_mut::<phoenix::ship::state::ShipPhysics>(entity)
            .expect("a placed hull has physics");
        physics.x = 5_500.0;
        physics.z = -3_250.0;
    }

    let removal = despawn(&mut app, 1, 2, &npc);
    let removed_at = tick(&app);
    let request = undo_of(&mut app, 2, 3, &removal);
    submit(&mut app, request.clone());
    settle(&mut app);

    let restored = entity_for(&mut app, &npc).expect("the inverse rebuilt the removed identity");
    let physics = app
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(restored)
        .expect("the rebuilt hull carries its captured physics");
    // Where it was when it was removed, not where the palette places a fresh
    // one (4000, -4000). The hull keeps flying between the write above and the
    // removal, so this is a neighbourhood rather than an equality: what it rules
    // out is a rebuild from the bare template.
    assert!(
        (physics.x - 5_500.0).abs() < 100.0 && (physics.z + 3_250.0).abs() < 100.0,
        "captured state, not a fresh spawn: {:?}",
        (physics.x, physics.z)
    );
    // No absent time was simulated: the world's clock ran on while the entity was
    // gone and the rebuild lands at the tick the GM pressed Undo, not at the tick
    // of the removal.
    assert!(tick(&app) > removed_at);

    let log = facts(&app);
    let undone = fact(&log, request.correlation.as_str());
    assert_eq!(undone.outcome, GmActionOutcome::Applied);
    assert_eq!(undone.action_kind, GmActionKind::ActionUndo);
    assert_eq!(
        undone.affected,
        Some(GmAffectedField::EntityPresence {
            entity: npc.clone(),
            before: false,
            after: true,
        })
    );
    // Both operators are on the ONE saved history.
    assert_eq!(undone.operator_id, "gm-2");
    let reference = undone
        .undo_of
        .clone()
        .expect("an inverse names its original");
    assert_eq!(reference.operator_id, "gm-1");
    assert_eq!(reference.correlation, removal.correlation);
    assert_eq!(reference.sequence, removal.order.sequence);
    // The attributed GM knowledge override the cleanup cleared comes back.
    assert_eq!(
        contact_override(&app, &observer, &npc),
        Some(ContactMode::Reveal)
    );
    // And the capture is consumed, so the row stops offering a second Undo.
    let captures = app
        .world()
        .resource::<WorldContentRuntime>()
        .gm_despawn_captures
        .clone();
    assert!(!captures.is_restorable(
        &removal.operator_id,
        &removal.correlation,
        removal.order.sequence
    ));
}

#[test]
fn a_removal_no_capture_can_rebuild_refuses_its_inverse_and_says_so_on_the_journal() {
    let mut app = seeded();
    let authored = authored_npc(&mut app);
    let removal = despawn(&mut app, 1, 1, &authored);
    assert_eq!(
        fact(&facts(&app), removal.correlation.as_str()).outcome,
        GmActionOutcome::Applied
    );
    assert!(entity_for(&mut app, &authored).is_none());

    // The page is told BEFORE it offers anything.
    let captures = app
        .world()
        .resource::<WorldContentRuntime>()
        .gm_despawn_captures
        .clone();
    let projection = phoenix::gm_journal::journal_projection(
        app.world().resource::<GmActionLog>(),
        None,
        60.0,
        Some(&captures),
    );
    let row = projection
        .entries
        .iter()
        .find(|row| row.correlation == removal.correlation.as_str())
        .expect("the removal is on the published journal");
    assert!(
        row.capture_lost,
        "an authored world entity carries no recipe an inverse could rebuild it from"
    );

    // And a page that asked anyway is refused, not half-served.
    let request = undo_of(&mut app, 2, 2, &removal);
    submit(&mut app, request.clone());
    settle(&mut app);
    let log = facts(&app);
    let undone = fact(&log, request.correlation.as_str());
    assert_eq!(undone.outcome, GmActionOutcome::Refused);
    assert_eq!(
        undone.reason,
        Some(GmActionRefusalReason::InverseUnsupported)
    );
    assert!(entity_for(&mut app, &authored).is_none());
}

#[test]
fn a_reused_identity_refuses_the_inverse_without_overwriting_the_live_entity() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let removal = despawn(&mut app, 1, 2, &npc);

    // Something else now answers to that identity.
    let occupier = app.world_mut().spawn(EntityUuid(npc.clone())).id();
    let request = undo_of(&mut app, 1, 3, &removal);
    submit(&mut app, request.clone());
    settle(&mut app);

    let log = facts(&app);
    let undone = fact(&log, request.correlation.as_str());
    assert_eq!(undone.outcome, GmActionOutcome::Refused);
    assert_eq!(
        undone.reason,
        Some(GmActionRefusalReason::RestoreIdentityOccupied)
    );
    // No partial overwrite: the occupier is untouched and no hull was built.
    assert!(app.world().get_entity(occupier).is_ok());
    assert!(
        app.world()
            .get::<phoenix::ship::state::ShipPhysics>(occupier)
            .is_none(),
        "the refused restore wrote nothing onto the identity it found taken"
    );
    assert_eq!(
        app.world_mut()
            .query::<&EntityUuid>()
            .iter(app.world())
            .filter(|uuid| uuid.0 == npc)
            .count(),
        1,
        "one identity, one claimant"
    );
}

#[test]
fn a_newer_conflicting_reference_refuses_the_inverse_without_a_partial_restore() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let observer = fleet_observer(&mut app);
    set_override(&mut app, &observer, &npc, ContactMode::Reveal);
    let removal = despawn(&mut app, 1, 2, &npc);
    let name = app
        .world()
        .resource::<WorldContentRuntime>()
        .name_to_uuid
        .iter()
        .find(|(_, uuid)| *uuid == &npc)
        .map(|(name, _)| name.clone())
        .expect("a palette placement registers its scenario name");

    // The world has re-bound one of the names the removed entity answered to.
    // Nothing about the entity's own identity has changed; a reference TO it
    // now points somewhere else.
    let successor = place_removable_npc(&mut app, 3);
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .name_to_uuid
        .insert(name.clone(), successor.clone());

    let request = undo_of(&mut app, 2, 4, &removal);
    submit(&mut app, request.clone());
    settle(&mut app);

    let log = facts(&app);
    let undone = fact(&log, request.correlation.as_str());
    assert_eq!(undone.outcome, GmActionOutcome::Refused);
    assert_eq!(
        undone.reason,
        Some(GmActionRefusalReason::RestoreReferenceConflict)
    );
    assert_eq!(
        app.world().resource::<WorldContentRuntime>().name_to_uuid[&name],
        successor,
        "the newer reference is never overwritten"
    );
    assert!(
        entity_for(&mut app, &npc).is_none(),
        "a refused inverse is refused whole: no hull, no half-applied references"
    );
    assert_eq!(
        contact_override(&app, &observer, &npc),
        None,
        "and no reference the restore would have put back was written either"
    );
}

#[test]
fn unrelated_world_changes_do_not_stand_in_the_way_of_the_inverse() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let observer = fleet_observer(&mut app);
    set_override(&mut app, &observer, &npc, ContactMode::Reveal);
    let removal = despawn(&mut app, 1, 2, &npc);

    // Everything here is a real change to the same world, and none of it is the
    // affected reference: a different observer, a different target, and a second
    // entity spawned in the meantime.
    let bystander = place_removable_npc(&mut app, 3);
    set_override(&mut app, &observer, &bystander, ContactMode::Conceal);

    let request = undo_of(&mut app, 1, 4, &removal);
    submit(&mut app, request.clone());
    settle(&mut app);
    assert_eq!(
        fact(&facts(&app), request.correlation.as_str()).outcome,
        GmActionOutcome::Applied
    );
    assert!(entity_for(&mut app, &npc).is_some());
    assert!(entity_for(&mut app, &bystander).is_some());
    assert_eq!(
        contact_override(&app, &observer, &bystander),
        Some(ContactMode::Conceal),
        "an unrelated pair's newer row is left exactly as it was"
    );
}

#[test]
fn two_game_masters_racing_to_reverse_one_removal_get_one_restore_and_one_truth() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let removal = despawn(&mut app, 1, 2, &npc);

    let at = tick(&app) + 3;
    let log = facts(&app);
    let recorded = fact(&log, removal.correlation.as_str())
        .affected
        .clone()
        .unwrap();
    let inverse = |operator: u32, sequence: u64| {
        grant(
            operator,
            sequence,
            at,
            GmAction::UndoGmAction {
                original: removal.correlation.clone(),
                original_operator: removal.operator_id.clone(),
                original_sequence: removal.order.sequence,
                expected: recorded.clone(),
            },
        )
    };
    let first = inverse(1, 3);
    let second = inverse(2, 4);
    submit(&mut app, first.clone());
    submit(&mut app, second.clone());
    settle(&mut app);

    let log = facts(&app);
    assert_eq!(
        fact(&log, first.correlation.as_str()).outcome,
        GmActionOutcome::Applied
    );
    let loser = fact(&log, second.correlation.as_str());
    assert_eq!(loser.outcome, GmActionOutcome::Refused);
    assert_eq!(loser.reason, Some(GmActionRefusalReason::AlreadyInverted));
    assert_eq!(
        app.world_mut()
            .query::<&EntityUuid>()
            .iter(app.world())
            .filter(|uuid| uuid.0 == npc)
            .count(),
        1,
        "two accepted inverses would be two hulls"
    );
}

#[test]
fn a_stale_page_that_echoes_the_wrong_pair_is_refused_rather_than_acted_on() {
    let mut app = seeded();
    let npc = place_removable_npc(&mut app, 1);
    let removal = despawn(&mut app, 1, 2, &npc);
    let request = grant(
        2,
        3,
        tick(&app) + 2,
        GmAction::UndoGmAction {
            original: removal.correlation.clone(),
            original_operator: removal.operator_id.clone(),
            original_sequence: removal.order.sequence,
            // A pair from a page that has drifted onto a different entity.
            expected: GmAffectedField::EntityPresence {
                entity: "some-other-hull".into(),
                before: true,
                after: false,
            },
        },
    );
    submit(&mut app, request.clone());
    settle(&mut app);
    let log = facts(&app);
    let undone = fact(&log, request.correlation.as_str());
    assert_eq!(undone.outcome, GmActionOutcome::Refused);
    assert_eq!(
        undone.reason,
        Some(GmActionRefusalReason::InverseFactsMismatch)
    );
    assert!(entity_for(&mut app, &npc).is_none());
}

#[test]
fn the_capture_survives_a_snapshot_round_trip_and_the_inverse_still_works() {
    let mut source = seeded();
    let npc = place_removable_npc(&mut source, 1);
    let observer = fleet_observer(&mut source);
    set_override(&mut source, &observer, &npc, ContactMode::Reveal);
    let removal = despawn(&mut source, 1, 2, &npc);

    // Round-tripped through the real payload's own serde shape, so a capture
    // that could not survive a save would fail here rather than in a live event.
    let taken = phoenix::snapshot::capture(source.world());
    let bytes = serde_json::to_vec(&taken).unwrap();
    let stored: phoenix::snapshot::PhoenixSnapshot = serde_json::from_slice(&bytes).unwrap();

    let mut resumed = seeded();
    phoenix::snapshot::restore(resumed.world_mut(), &stored);
    resumed.update();

    let captures = resumed
        .world()
        .resource::<WorldContentRuntime>()
        .gm_despawn_captures
        .clone();
    assert!(
        captures.is_restorable(
            &removal.operator_id,
            &removal.correlation,
            removal.order.sequence
        ),
        "a restored checkpoint recovers the exact inverse data it was saved with"
    );

    let request = undo_of(&mut resumed, 2, 3, &removal);
    submit(&mut resumed, request.clone());
    settle(&mut resumed);
    assert_eq!(
        fact(&facts(&resumed), request.correlation.as_str()).outcome,
        GmActionOutcome::Applied
    );
    assert!(entity_for(&mut resumed, &npc).is_some());
    assert_eq!(
        contact_override(&resumed, &observer, &npc),
        Some(ContactMode::Reveal)
    );
}

#[test]
fn two_peers_replaying_the_same_grants_agree_on_the_facts_and_the_digest() {
    // The first peer discovers the placed identity as a live GM would, then its
    // OWN three grants become the portable script the second peer replays. The
    // grants are the whole input: same seed, same apply ticks, same order.
    let mut left = seeded();
    let (placement, npc) = place_removable(&mut left, 1, "removable-raider");
    let removal = despawn(&mut left, 1, 2, &npc);
    let inverse = undo_of(&mut left, 2, 3, &removal);
    submit(&mut left, inverse.clone());
    let script = serde_json::to_string(&vec![placement, removal.clone(), inverse]).unwrap();
    let last = removal.apply_tick + 40;

    let mut right = seeded();
    for request in serde_json::from_str::<Vec<GmActionGrant>>(&script).unwrap() {
        submit(&mut right, request);
    }
    // Both peers run to the SAME absolute tick, which is what makes the digest
    // comparison below a statement about the GM work rather than about clocks.
    for app in [&mut left, &mut right] {
        while tick(app) < last {
            app.update();
        }
    }
    let left_facts = facts(&left);
    let right_facts = facts(&right);
    assert_eq!(left_facts, right_facts);
    assert_eq!(
        left_facts
            .iter()
            .map(|fact| fact.outcome)
            .collect::<Vec<_>>(),
        vec![
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied
        ],
        "place, remove, restore: {left_facts:#?}"
    );
    assert_eq!(
        phoenix::sim_digest::world_digest(left.world_mut()),
        phoenix::sim_digest::world_digest(right.world_mut()),
        "the retained inverse data is folded, so two peers holding different \
         captures would be caught here"
    );
    assert!(entity_for(&mut left, &npc).is_some());
}

/// Two peers whose OWN SCREENS differ still agree on the digest across a GM
/// removal (issue #1444).
///
/// `gm_despawn::remove_entity` releases task activations on its way through, and
/// `core::task_lifecycle::TaskLifecycles` is presentation-class state for the
/// #894 digest boundary: nothing in the fixed tick branches on it and neither
/// `sim_digest` nor `snapshot` walks it. A peer that RESUMED a save therefore
/// holds no activations at all while a peer that ran the same ticks through
/// holds them — which is exactly the pair this plants, on the left peer only,
/// before running the SAME removal grant on both. Inverse data that carried what
/// the cleanup released would fold and save that difference, and two peers who
/// agree about the world would part company here — mid-live-event, with a
/// `DigestMismatch` nobody in the room could explain. The same bar covers the
/// Comms presence the cleanup also releases, which is rebuilt every tick from a
/// `LocalShip` query a shipless GM peer never runs; two in-process peers cannot
/// be made to disagree about a value both of them re-derive, so it is the
/// activation that carries the assertion.
#[test]
fn a_removal_agrees_across_peers_whose_own_screen_state_differs() {
    let mut left = seeded();
    let mut right = seeded();
    let (placement, npc) = place_removable(&mut left, 1, "removable-raider");
    submit(&mut right, placement);
    settle(&mut right);

    // Presentation-class state, on the left peer ONLY.
    let observer = fleet_observer(&mut left);
    let at = tick(&left);
    let mut tasks = left
        .world_mut()
        .get_resource_mut::<phoenix::core::task_lifecycle::TaskLifecycles>()
        .expect("the run keeps task activations");
    tasks.begin(
        phoenix::core::task_lifecycle::TaskSlot::new(observer, "sensors", "scan"),
        Some(npc.clone()),
        None,
        at,
    );
    assert_eq!(
        phoenix::sim_digest::world_digest(left.world_mut()),
        phoenix::sim_digest::world_digest(right.world_mut()),
        "the divergence planted above is outside the digest boundary to begin with"
    );

    // The plant is still standing when the removal runs, so the cleanup really
    // does release an activation on one peer and not on the other. Without this
    // the test would pass on a build that folded released links and prove
    // nothing at all.
    for (app, expected) in [(&left, 1), (&right, 0)] {
        assert_eq!(
            app.world()
                .resource::<phoenix::core::task_lifecycle::TaskLifecycles>()
                .active()
                .filter(|active| active.key.target.as_deref() == Some(npc.as_str()))
                .count(),
            expected
        );
    }

    let removal = despawn(&mut left, 1, 2, &npc);
    submit(&mut right, removal.clone());
    let last = removal.apply_tick + 20;
    for app in [&mut left, &mut right] {
        while tick(app) < last {
            app.update();
        }
    }
    for app in [&left, &right] {
        assert_eq!(
            fact(&facts(app), removal.correlation.as_str()).outcome,
            GmActionOutcome::Applied
        );
    }
    assert!(entity_for(&mut left, &npc).is_none());
    assert_eq!(
        phoenix::sim_digest::world_digest(left.world_mut()),
        phoenix::sim_digest::world_digest(right.world_mut()),
        "a retained capture must hold nothing derived from a peer's own screen"
    );
}

#[test]
fn a_runtime_spawned_structure_is_reversed_by_the_same_machinery_as_a_hull() {
    // The capture branches on whether the entity has a spawn RECIPE, never on
    // what kind of thing it turned out to be. A berth has no ship physics, no
    // helm, no AI and no weapon machines, so a capture that quietly assumed a
    // hull would fail here rather than in a live event.
    let mut app = seeded();
    let (_, berth) = place_removable(&mut app, 1, "removable-berth");
    assert!(
        app.world_mut()
            .query::<(&EntityUuid, &phoenix::ship::state::ShipPhysics)>()
            .iter(app.world())
            .all(|(uuid, _)| uuid.0 != berth),
        "the structure class is genuinely not a hull"
    );
    let observer = fleet_observer(&mut app);
    set_override(&mut app, &observer, &berth, ContactMode::Reveal);

    let removal = despawn(&mut app, 1, 2, &berth);
    assert_eq!(
        fact(&facts(&app), removal.correlation.as_str()).outcome,
        GmActionOutcome::Applied
    );
    assert!(entity_for(&mut app, &berth).is_none());

    let request = undo_of(&mut app, 2, 3, &removal);
    submit(&mut app, request.clone());
    settle(&mut app);
    assert_eq!(
        fact(&facts(&app), request.correlation.as_str()).outcome,
        GmActionOutcome::Applied
    );
    let restored = entity_for(&mut app, &berth).expect("the berth is back under its own identity");
    assert!(
        app.world()
            .get::<phoenix::entities::spawner::EntityTagsSection>(restored)
            .is_some_and(|tags| tags.0.iter().any(|tag| tag == "structure")),
        "rebuilt from its recorded recipe, tags and all"
    );
    assert_eq!(
        contact_override(&app, &observer, &berth),
        Some(ContactMode::Reveal)
    );
}
