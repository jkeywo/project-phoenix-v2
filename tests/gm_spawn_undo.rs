//! Taking a GM placement back before crews have had two seconds to see it
//! (issue #1443, PRD #1420 story 4).
//!
//! Everything here drives the production path: a real headless world load, the
//! real `apply_due_actions` reducer over real canonical grants, the real
//! `observe_gm_spawn_exposure` stopwatch running in `FixedLast`, the real
//! trigger pipeline that materialises and removes the placement, real
//! `snapshot::capture`/`restore`, the real `sim_digest::world_digest`, and the
//! real `publish_session_projection` message the GM journal panel reads.
//! Nothing here constructs a projection, an exposure record or a result by
//! hand.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::HostSlot,
    console_bridge::GmSessionChanged,
    core::messages::{ModifierSlot, ModifierSource},
    entities::spawner::EntityUuid,
    gm_action::*,
    gm_exposure::{GmSpawnExposure, GM_SPAWN_EXPOSURE_LIMIT_SECS},
    gm_journal::{GmJournalEntry, GmJournalProjection},
    sim_tick::SimTick,
};
use project_phoenix as phoenix;

const WORLD: &str = "tests/fixtures/worlds/gm_spawn_exposure.toml";
/// The cutoff in fixed steps at the fixture's shipped 60 Hz. Derived rather
/// than written down twice, so a retuned rate cannot leave this stale.
const LIMIT_TICKS: u64 = (GM_SPAWN_EXPOSURE_LIMIT_SECS as u64) * 60;
/// Well inside every player hull's authored 350-unit sensor horizon.
const SEEN: [i64; 3] = [60_000, 0, 0];
/// Well outside it, with room for any drift.
const UNSEEN: [i64; 3] = [30_000_000, 0, 0];

fn boot() -> App {
    let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1443),
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

/// Place one palette courier and let the ordinary trigger pipeline materialise
/// it, exactly as a live GM's placement lands.
fn place(app: &mut App, sequence: u64, operator: &str, variant: Option<&str>, at: [i64; 3]) {
    apply(
        app,
        sequence,
        operator,
        GmAction::SpawnPaletteEntity {
            palette: "courier".into(),
            variant: variant.map(str::to_string),
            position_mm: at,
            heading_mdeg: 0,
        },
    );
    app.update();
}

/// The deterministic scenario name one placement takes.
fn placed_name(sequence: u64) -> String {
    format!("gm_courier_{sequence}")
}

fn placement(name: &str) -> GmAffectedField {
    GmAffectedField::SpawnedEntity {
        name: name.to_string(),
        before: false,
        after: true,
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

/// Whether the world currently holds an entity under this placement's name.
fn stands(app: &mut App, name: &str) -> bool {
    let Some(uuid) = app
        .world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid
        .get(name)
        .cloned()
    else {
        return false;
    };
    app.world_mut()
        .query::<&EntityUuid>()
        .iter(app.world())
        .any(|id| id.0 == uuid)
}

fn ticks(app: &App, name: &str) -> u64 {
    app.world()
        .resource::<GmSpawnExposure>()
        .get(name)
        .map(|record| record.ticks)
        .unwrap_or_default()
}

fn latched(app: &App, name: &str) -> bool {
    app.world()
        .resource::<GmSpawnExposure>()
        .get(name)
        .is_some_and(|record| record.latched)
}

fn step(app: &mut App, steps: u64) {
    let before = app.world().resource::<SimTick>().0;
    while app.world().resource::<SimTick>().0 < before + steps {
        app.update();
    }
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

// ── The cutoff ──────────────────────────────────────────────────────────────

/// Under the cutoff, an inverse is the ordinary safe removal: the placement
/// goes, through the same cascade a GM despawn runs.
#[test]
fn a_placement_nobody_could_see_is_taken_back() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), UNSEEN);
    let name = placed_name(1);
    assert!(stands(&mut app, &name), "the placement landed");
    step(&mut app, LIMIT_TICKS * 3);
    assert_eq!(
        ticks(&app, &name),
        0,
        "nothing outside every player hull's horizon accumulates exposure"
    );
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(outcome(&mut app, "act-2"), (GmActionOutcome::Applied, None));
    app.update();
    assert!(!stands(&mut app, &name), "the placement is gone");
}

/// One step short of the cutoff the window is still open; the step that reaches
/// it closes the window for good. Both halves are asserted against the SAME
/// placement path so the boundary is exact rather than approximate.
#[test]
fn the_step_that_reaches_two_seconds_closes_the_window() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    while ticks(&app, &name) < LIMIT_TICKS - 1 {
        app.update();
    }
    assert_eq!(ticks(&app, &name), LIMIT_TICKS - 1);
    assert!(!latched(&app, &name), "one step short is still reversible");
    app.update();
    assert_eq!(ticks(&app, &name), LIMIT_TICKS);
    assert!(
        latched(&app, &name),
        "exactly at the cutoff the latch closes"
    );
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(
        outcome(&mut app, "act-2"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::SensorExposureElapsed)
        )
    );
    app.update();
    assert!(stands(&mut app, &name), "a refused inverse changes nothing");
}

/// The step BEFORE the cutoff really does apply — the same placement, the same
/// harness, one step earlier.
#[test]
fn one_step_short_of_the_cutoff_still_applies() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    while ticks(&app, &name) < LIMIT_TICKS - 1 {
        app.update();
    }
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(outcome(&mut app, "act-2"), (GmActionOutcome::Applied, None));
    app.update();
    assert!(!stands(&mut app, &name));
}

/// Leaving the range does not reset the count, and the latch never re-opens.
///
/// The placement is moved out of every horizon by hand — the same thing a helm
/// would do — after a spell inside one, and the counter holds where it stopped.
#[test]
fn leaving_the_range_holds_the_count_and_the_latch_stands() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    while ticks(&app, &name) < 20 {
        app.update();
    }
    warp(&mut app, &name, Vec3::new(30_000.0, 0.0, 0.0));
    step(&mut app, LIMIT_TICKS * 2);
    let held = ticks(&app, &name);
    assert!(
        (20..30).contains(&held),
        "the count holds where it stopped, not at zero: {held}"
    );
    assert!(!latched(&app, &name));
    // Back into range: the two spells add up rather than starting over.
    warp(&mut app, &name, Vec3::ZERO);
    while !latched(&app, &name) {
        app.update();
    }
    assert_eq!(ticks(&app, &name), LIMIT_TICKS);
    warp(&mut app, &name, Vec3::new(30_000.0, 0.0, 0.0));
    step(&mut app, LIMIT_TICKS);
    assert!(latched(&app, &name), "the latch is permanent");
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(
        outcome(&mut app, "act-2").1,
        Some(GmActionRefusalReason::SensorExposureElapsed)
    );
}

/// Two overlapping player hulls are still one second per second.
///
/// The fixture parks a second player-tagged cruiser fifty units from the first,
/// so a placement between them is inside BOTH horizons; the counter must match
/// the number of steps observed, not twice it.
#[test]
fn overlapping_player_hulls_count_one_second_per_second() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    let start = app.world().resource::<SimTick>().0;
    let seen = ticks(&app, &name);
    step(&mut app, 40);
    let elapsed = app.world().resource::<SimTick>().0 - start;
    assert_eq!(
        ticks(&app, &name) - seen,
        elapsed,
        "union coverage: overlapping watchers cannot count a step twice"
    );
}

/// A paused world contributes nothing. The GM pause is the ordinary typed
/// action, not a test-only flag.
#[test]
fn a_paused_world_does_not_spend_the_window() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    step(&mut app, 30);
    let held = ticks(&app, &name);
    assert!(held > 0);
    apply(
        &mut app,
        2,
        "gm-1",
        GmAction::SetSessionPaused { active: true },
    );
    for _ in 0..600 {
        app.update();
    }
    assert_eq!(
        ticks(&app, &name),
        held,
        "a step that never starts cannot count"
    );
    assert!(!latched(&app, &name));
}

/// A shrinking or growing sensor horizon moves the answer with it. The
/// modifier is the same `SensorRadarRange` slot a damaged sensor radar and an
/// authored `apply_modifier` trigger write.
#[test]
fn a_changed_sensor_range_changes_what_counts() {
    let mut app = boot();
    // 600 units out: beyond the authored 350-unit horizon of both hulls.
    place(&mut app, 1, "gm-1", Some("removable"), [600_000, 0, 0]);
    let name = placed_name(1);
    step(&mut app, 40);
    assert_eq!(ticks(&app, &name), 0, "out of range counts nothing");
    widen_player_sensors(&mut app, 1.0);
    let start = app.world().resource::<SimTick>().0;
    step(&mut app, 40);
    let elapsed = app.world().resource::<SimTick>().0 - start;
    assert_eq!(
        ticks(&app, &name),
        elapsed,
        "doubling the horizon brings the placement inside it"
    );
    widen_player_sensors(&mut app, -0.5);
    let held = ticks(&app, &name);
    step(&mut app, 40);
    assert_eq!(
        ticks(&app, &name),
        held,
        "shrinking it back stops the clock without resetting it"
    );
}

/// Concealment changes what a console DRAWS, never whether the ship was there
/// to be found. The counter runs regardless.
#[test]
fn concealing_the_placement_does_not_stop_the_clock() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    let uuid = app
        .world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid[&name]
        .clone();
    let ship = phoenix::command_admission::log::ShipKey(player_uuid(&mut app));
    apply(
        &mut app,
        2,
        "gm-1",
        GmAction::SetContactOverride {
            ship,
            target: uuid,
            mode: phoenix::gm_contact::ContactMode::Conceal,
        },
    );
    assert_eq!(outcome(&mut app, "act-2").0, GmActionOutcome::Applied);
    while !latched(&app, &name) {
        app.update();
    }
    apply(&mut app, 3, "gm-1", undo(1, "gm-1", placement(&name)));
    assert_eq!(
        outcome(&mut app, "act-3").1,
        Some(GmActionRefusalReason::SensorExposureElapsed),
        "a concealed contact was still there to be found"
    );
}

/// Damage is not a second cutoff. A placement nobody can see stays reversible
/// however badly it has been shot at.
#[test]
fn damage_is_not_a_second_cutoff() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), UNSEEN);
    let name = placed_name(1);
    let uuid = app
        .world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid[&name]
        .clone();
    apply(
        &mut app,
        2,
        "gm-1",
        GmAction::ApplyDirectEffect {
            target: uuid,
            scope: phoenix::gm_effect::GmDirectEffectScope::Entity,
            effect: phoenix::gm_effect::GmDirectEffectKind::Damage,
            amount_milli_hp: 5_000,
        },
    );
    step(&mut app, LIMIT_TICKS * 2);
    apply(&mut app, 3, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(outcome(&mut app, "act-3"), (GmActionOutcome::Applied, None));
}

// ── The ordinary safe-removal half ──────────────────────────────────────────

/// Being placed by a GM two ticks ago does not exempt anything from the
/// authored removal policy. The bare palette entry keeps the courier's own
/// tags, which do not include `gm_removable`.
#[test]
fn the_ordinary_safe_removal_policy_still_decides() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", None, UNSEEN);
    let name = placed_name(1);
    assert!(stands(&mut app, &name));
    apply(&mut app, 2, "gm-1", undo(1, "gm-1", placement(&name)));
    assert_eq!(
        outcome(&mut app, "act-2"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::ProtectedEntity)
        )
    );
    app.update();
    assert!(stands(&mut app, &name));
}

/// A placement that has already left the world by other means is not silently
/// reversed: the affected field moved.
#[test]
fn a_placement_that_has_already_gone_reports_the_field_moved() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), UNSEEN);
    let name = placed_name(1);
    let uuid = app
        .world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid[&name]
        .clone();
    apply(
        &mut app,
        2,
        "gm-1",
        GmAction::DespawnEntity {
            target: uuid.clone(),
        },
    );
    app.update();
    assert!(!stands(&mut app, &name));
    apply(&mut app, 3, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(
        outcome(&mut app, "act-3"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::AffectedStateChanged)
        )
    );
}

// ── Two GMs ─────────────────────────────────────────────────────────────────

/// Any equal GM may reverse another's placement, both operators land on the one
/// saved history, and the second of two racing requests is told the truth.
#[test]
fn two_game_masters_racing_to_undo_get_one_undo_and_one_refusal() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), UNSEEN);
    let name = placed_name(1);
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    apply(&mut app, 3, "gm-3", undo(1, "gm-1", placement(&name)));
    assert_eq!(outcome(&mut app, "act-2"), (GmActionOutcome::Applied, None));
    assert_eq!(
        outcome(&mut app, "act-3"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::AlreadyInverted)
        )
    );
    let inverse = row(&mut app, "act-2");
    assert_eq!(inverse.operator_id, "gm-2", "the undoing GM");
    let reference = inverse.undo_of.expect("the inverse names its original");
    assert_eq!(reference.operator_id, "gm-1", "the original GM");
    assert_eq!(reference.sequence, 1);
    assert!(
        row(&mut app, "act-1").inverted,
        "the original row says it has been reversed"
    );
}

/// An exact resubmission of an accepted inverse is idempotent rather than a
/// second removal.
#[test]
fn a_double_submit_of_the_same_inverse_is_a_no_op() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), UNSEEN);
    let name = placed_name(1);
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    // The same operator's same correlation: the journal's own idempotency.
    let tick = app.world().resource::<SimTick>().0;
    let repeat = grant(2, tick, "gm-2", undo(1, "gm-1", placement(&name)));
    assert!(
        app.world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(repeat)
            .is_ok(),
        "an exact retry is the cached grant, not a conflict"
    );
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    assert_eq!(outcome(&mut app, "act-2"), (GmActionOutcome::Applied, None));
}

// ── Preview and staleness ───────────────────────────────────────────────────

/// The published row reports CURRENT eligibility, in the units a GM reads, and
/// keeps reporting it once the window has shut.
#[test]
fn the_published_row_reports_current_eligibility() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    step(&mut app, 60);
    let early = row(&mut app, "act-1")
        .spawn_exposure
        .expect("a placement row carries its exposure");
    assert_eq!(early.limit_ms, 2_000);
    assert!(!early.latched);
    assert!(
        (900..=1_100).contains(&early.exposed_ms),
        "about one second in: {}",
        early.exposed_ms
    );
    while !latched(&app, &name) {
        app.update();
    }
    let late = row(&mut app, "act-1").spawn_exposure.unwrap();
    assert!(late.latched);
    assert_eq!(late.exposed_ms, 2_000);
    // Every other family's row is untouched by this — a Pause has no clock.
    apply(
        &mut app,
        2,
        "gm-1",
        GmAction::SetSessionPaused { active: true },
    );
    assert!(row(&mut app, "act-2").spawn_exposure.is_none());
}

/// A preview taken while the window was open does not decide anything: the
/// apply tick does. The request is byte-for-byte the one the early row would
/// have built.
#[test]
fn a_stale_preview_is_refused_at_the_apply_tick() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    step(&mut app, 30);
    let early = row(&mut app, "act-1");
    assert!(!early.spawn_exposure.unwrap().latched, "eligible when read");
    let request = undo(
        1,
        &early.operator_id,
        early.affected.clone().expect("the recorded pair"),
    );
    while !latched(&app, &name) {
        app.update();
    }
    apply(&mut app, 2, "gm-2", request);
    assert_eq!(
        outcome(&mut app, "act-2"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::SensorExposureElapsed)
        )
    );
}

/// The recorded pair is compared before anything else, so a request built
/// against a different placement is refused on its facts rather than acted on.
#[test]
fn a_request_naming_the_wrong_placement_is_refused_on_its_facts() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), UNSEEN);
    apply(
        &mut app,
        2,
        "gm-2",
        undo(1, "gm-1", placement("gm_courier_999")),
    );
    assert_eq!(
        outcome(&mut app, "act-2"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::InverseFactsMismatch)
        )
    );
}

// ── Save, restore, digest ───────────────────────────────────────────────────

/// A restored checkpoint recovers its EXACT eligibility, in both directions: a
/// save taken while the window was open reverses, and one taken after it shut
/// still refuses.
#[test]
fn a_restored_checkpoint_recovers_its_exact_eligibility() {
    let mut app = boot();
    place(&mut app, 1, "gm-1", Some("removable"), SEEN);
    let name = placed_name(1);
    step(&mut app, 30);
    let open = phoenix::snapshot::capture(app.world());
    let open_ticks = ticks(&app, &name);
    while !latched(&app, &name) {
        app.update();
    }
    let shut = phoenix::snapshot::capture(app.world());

    // The later save: still refused, and the counter came back closed.
    phoenix::snapshot::restore(app.world_mut(), &shut);
    assert!(latched(&app, &name));
    apply(&mut app, 2, "gm-2", undo(1, "gm-1", placement(&name)));
    assert_eq!(
        outcome(&mut app, "act-2").1,
        Some(GmActionRefusalReason::SensorExposureElapsed)
    );

    // The earlier save: the window is open again because the world really is
    // back at that moment, and the count is the one that was captured.
    phoenix::snapshot::restore(app.world_mut(), &open);
    assert_eq!(ticks(&app, &name), open_ticks);
    assert!(!latched(&app, &name));
    // The restore rewound the journal with the world, so the next canonical
    // sequence is 2 again — there is no abandoned timeline to skip past.
    apply(&mut app, 2, "gm-3", undo(1, "gm-1", placement(&name)));
    assert_eq!(outcome(&mut app, "act-2"), (GmActionOutcome::Applied, None));
}

/// The counter and its latch are part of what a divergence is defined over, and
/// a run that never places anything keeps the digest it had before this landed.
#[test]
fn the_digest_folds_the_counter_only_once_a_placement_exists() {
    let mut quiet = boot();
    let mut placed = boot();
    step(&mut quiet, 20);
    step(&mut placed, 20);
    assert_eq!(
        phoenix::sim_digest::world_digest(quiet.world()),
        phoenix::sim_digest::world_digest(placed.world()),
        "two identical runs agree before either places anything"
    );
    place(&mut placed, 1, "gm-1", Some("removable"), SEEN);
    step(&mut placed, 30);
    let before = phoenix::sim_digest::world_digest(placed.world());
    step(&mut placed, 30);
    assert_ne!(
        before,
        phoenix::sim_digest::world_digest(placed.world()),
        "the exposure counter moves the digest with it"
    );
    let snapshot = phoenix::snapshot::capture(placed.world());
    let after = phoenix::sim_digest::world_digest(placed.world());
    phoenix::snapshot::restore(placed.world_mut(), &snapshot);
    assert_eq!(
        after,
        phoenix::sim_digest::world_digest(placed.world()),
        "a captured counter restores to the same digest"
    );
}

// ── Helpers that move the world ─────────────────────────────────────────────

/// The uuid of the hull the local seat flies.
fn player_uuid(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&EntityUuid, With<phoenix::server_app::LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the fixture places a player hull")
        .0
        .clone()
}

/// Move one placement, the way a helm would.
fn warp(app: &mut App, name: &str, to: Vec3) {
    let uuid = app
        .world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid[name]
        .clone();
    let entity = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == uuid)
        .map(|(entity, _)| entity)
        .expect("the placement stands");
    if let Some(mut physics) = app
        .world_mut()
        .get_mut::<phoenix::ship::state::ShipPhysics>(entity)
    {
        physics.x = to.x;
        physics.y = to.y;
        physics.z = to.z;
    }
    if let Some(mut transform) = app.world_mut().get_mut::<Transform>(entity) {
        transform.translation = to;
    }
}

/// Scale every player hull's sensor horizon through the same `SensorRadarRange`
/// slot a damaged sensor radar writes.
fn widen_player_sensors(app: &mut App, bonus: f32) {
    let mut query = app
        .world_mut()
        .query_filtered::<&mut phoenix::modifiers::ShipModifiers, With<phoenix::server_app::Ship>>(
        );
    let mut modifiers: Vec<_> = query.iter_mut(app.world_mut()).collect();
    for entry in modifiers.iter_mut() {
        entry.add_or_update(phoenix::modifiers::Modifier {
            source: ModifierSource::World {
                id: "test".into(),
                tag: "sensor-range".into(),
            },
            slot: ModifierSlot::SensorRadarRange,
            bonus,
        });
    }
}
