//! The saved GM journal a real host publishes, and what a real capture/restore
//! does to it (issue #1441).
//!
//! Everything here drives the production path: real world load, real
//! `apply_due_actions` reducer, real `snapshot::capture`/`restore`, and the real
//! `publish_session_projection` system whose Host-Channel message the GM journal
//! panel reads. Nothing constructs a projection by hand.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::HostSlot,
    console_bridge::GmSessionChanged,
    gm_action::*,
    gm_journal::{GmJournalProjection, GM_JOURNAL_WINDOW},
    sim_tick::SimTick,
};
use project_phoenix as phoenix;

const WORLD: &str = "tests/fixtures/worlds/gm_npc_doctrine.toml";

fn boot() -> App {
    let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1441),
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
    // Deliberately a slot that is neither the sequencer nor 0/1: a projection
    // that leaked the transport origin would be visible as a `7` in the JSON.
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
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}

/// The journal exactly as the GM page receives it: through the production
/// publisher and its Host-Channel message, not read off a resource.
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

fn rows(projection: &GmJournalProjection) -> Vec<(String, String, u64, Option<u64>, String)> {
    projection
        .entries
        .iter()
        .map(|entry| {
            (
                entry.operator_id.clone(),
                entry.correlation.clone(),
                entry.tick,
                entry.sequence,
                serde_json::to_value(entry.outcome)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect()
}

/// Three real terminal outcomes, publicly attributed, with no transport
/// identity and no second store behind them.
#[test]
fn published_journal_carries_real_applied_no_op_and_refused_facts_without_session_secrets() {
    let mut app = boot();
    apply(
        &mut app,
        1,
        "gm-alex",
        GmAction::SetSessionPaused { active: true },
    );
    apply(
        &mut app,
        2,
        "gm-alex",
        GmAction::SetSessionPaused { active: true },
    );
    apply(
        &mut app,
        3,
        "gm-sam",
        GmAction::DespawnEntity {
            target: "no-such-entity".into(),
        },
    );
    let projection = published(&mut app);
    assert_eq!(projection.total, 3);
    assert_eq!(projection.capacity, MAX_GM_ACTIONS_PER_RUN);
    let outcomes: Vec<_> = rows(&projection)
        .into_iter()
        .map(|row| (row.0, row.1, row.3, row.4))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            ("gm-alex".into(), "act-1".into(), Some(1), "applied".into()),
            ("gm-alex".into(), "act-2".into(), Some(2), "no-op".into()),
            ("gm-sam".into(), "act-3".into(), Some(3), "refused".into()),
        ],
        "the panel reads the operator, the order and the terminal outcome of every action"
    );
    assert_eq!(
        projection.entries[2].target.as_deref(),
        Some("no-such-entity"),
        "a refusal still names the target the operator asked for"
    );
    assert_eq!(
        serde_json::to_value(projection.entries[2].reason)
            .unwrap()
            .as_str()
            .unwrap(),
        "unknown-entity",
        "and why the world refused it"
    );
    // Structural: the published rows carry exactly these public fields. The
    // grant's `from`/`sequenced_by`/`recovery_generation` and the `HostSlot`
    // inside `GmActionOrder::origin` are transport identity and never appear.
    let json = serde_json::to_value(&projection).unwrap();
    for row in json["entries"].as_array().unwrap() {
        let mut keys: Vec<_> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert!(
            keys.iter().all(|key| [
                "action_kind",
                "correlation",
                "operator_id",
                "outcome",
                "reason",
                "sequence",
                "target",
                "tick"
            ]
            .contains(key)),
            "unexpected public field in {row}"
        );
    }
}

/// The one canonical journal. A capture rewinds the visible history with the
/// world; entries made after the save are gone, not retained as a branch.
#[test]
fn capture_and_restore_load_exactly_the_saved_history_and_discard_later_entries() {
    let mut live = boot();
    apply(
        &mut live,
        1,
        "gm-alex",
        GmAction::SetSessionPaused { active: true },
    );
    apply(
        &mut live,
        2,
        "gm-sam",
        GmAction::DespawnEntity {
            target: "no-such-entity".into(),
        },
    );
    let saved_rows = rows(&published(&mut live));
    assert_eq!(saved_rows.len(), 2);
    let saved = phoenix::snapshot::capture(live.world());

    // Keep facilitating past the save.
    apply(
        &mut live,
        3,
        "gm-alex",
        GmAction::SetSessionPaused { active: false },
    );
    apply(
        &mut live,
        4,
        "gm-sam",
        GmAction::DespawnEntity {
            target: "still-no-such-entity".into(),
        },
    );
    let later = published(&mut live);
    assert_eq!(later.total, 4);
    assert!(later
        .entries
        .iter()
        .any(|entry| entry.correlation == "act-4"));

    // A real restore on a real second host.
    let mut resumed = boot();
    let report = phoenix::snapshot::restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    let restored = published(&mut resumed);
    assert_eq!(
        rows(&restored),
        saved_rows,
        "the restored journal is exactly the save's history"
    );
    assert_eq!(restored.total, 2);
    assert!(
        !restored
            .entries
            .iter()
            .any(|entry| entry.correlation == "act-3" || entry.correlation == "act-4"),
        "entries made after the save are discarded, not kept as an abandoned branch"
    );

    // New work appends to the RESTORED log, reusing the sequence the discarded
    // entries occupied — there is no second timeline to collide with.
    apply(
        &mut resumed,
        3,
        "gm-jo",
        GmAction::SetSessionPaused { active: false },
    );
    let continued = published(&mut resumed);
    assert_eq!(continued.total, 3);
    assert_eq!(
        continued
            .entries
            .iter()
            .map(|entry| entry.correlation.as_str())
            .collect::<Vec<_>>(),
        vec!["act-1", "act-2", "act-3"]
    );
    assert_eq!(continued.entries[2].operator_id, "gm-jo");
    assert!(
        continued.total <= GM_JOURNAL_WINDOW,
        "this run stays inside the presentation window, so `entries` is the whole history"
    );
    assert_eq!(continued.entries.len(), continued.total);
}
