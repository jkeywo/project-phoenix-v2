use super::*;
use crate::core::messages::{CommsMessage, CommsPriority, CommsResponseView};
use crate::gm_comms::{GmCommsRoute, GmCommsVisibility};

fn message(id: &str, sender: &str) -> CommsMessage {
    CommsMessage::injected(
        id.into(),
        sender.into(),
        "sender".into(),
        "body".into(),
        Default::default(),
        vec![CommsResponseView {
            text: "reply".into(),
            important: false,
            available: true,
        }],
        format!("thread-{id}"),
        true,
        CommsPriority::Routine,
    )
}

fn route(id: &str, band: Option<&str>) -> GmCommsRoute {
    GmCommsRoute {
        id: id.into(),
        label: "label".into(),
        visibility: GmCommsVisibility::SelectedShips,
        senders: vec!["speaker".into()],
        hails: Vec::new(),
        attention_band: band.map(str::to_string),
    }
}

#[test]
fn authored_band_overrides_the_default_and_an_unknown_word_is_ignored_by_the_reader() {
    assert_eq!(band_for(None), GmAttentionBand::Attention);
    assert_eq!(
        band_for(Some(&route("a", None))),
        GmAttentionBand::Attention
    );
    assert_eq!(
        band_for(Some(&route("a", Some("urgent")))),
        GmAttentionBand::Urgent
    );
    assert_eq!(
        band_for(Some(&route("a", Some("background")))),
        GmAttentionBand::Background
    );
    // The loader refuses this spelling outright (see `gm_comms::validate_routes`);
    // the reader still refuses to invent a band from it.
    assert_eq!(
        band_for(Some(&route("a", Some("Urgent")))),
        GmAttentionBand::Attention
    );
    assert_eq!(GmAttentionBand::from_authored("critical"), None);
}

#[test]
fn only_a_live_unanswered_message_with_options_is_pending() {
    let live = message("m1", "speaker");
    assert!(pending(&live));
    let mut answered = live.clone();
    answered.selected_response = Some(0);
    assert!(!pending(&answered));
    let mut orphaned = live.clone();
    orphaned.is_orphaned = true;
    assert!(!pending(&orphaned));
    let mut placeholder = live;
    placeholder.responses.clear();
    assert!(!pending(&placeholder));
}

#[test]
fn occurrence_identity_follows_the_message_and_carries_the_authored_route() {
    let world = WorldConfig {
        gm_comms_routes: vec![route("private", Some("background"))],
        ..Default::default()
    };
    let mut inbox = crate::console::comms::inbox::CommsInbox::new();
    let mut addressed = message("m1", "speaker-uuid");
    addressed.recipient_ship = Some(crate::command_admission::log::ShipKey(
        "ship-uuid".to_string(),
    ));
    inbox.inject(addressed);
    let names = BTreeMap::from([
        ("speaker-uuid".to_string(), "speaker".to_string()),
        ("ship-uuid".to_string(), "Valiant".to_string()),
    ]);
    let rows = collect(Some(&world), &inbox, &names);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "comms:m1");
    assert_eq!(rows[0].band, GmAttentionBand::Background);
    assert_eq!(rows[0].reason.id, PENDING_COMMS_REASON);
    assert_eq!(rows[0].reason.params.get("sender").unwrap(), "speaker");
    assert_eq!(rows[0].reason.params.get("ship").unwrap(), "Valiant");
    assert_eq!(rows[0].target.route.as_deref(), Some("private"));
    assert_eq!(rows[0].target.conversation.as_deref(), Some("thread-m1"));
}

/// A hail nobody addressed at one hull says so, rather than rendering the
/// addressed sentence around an empty `{ship}`.
#[test]
fn a_conversation_with_no_recipient_ship_reads_as_fleet_wide() {
    let mut inbox = crate::console::comms::inbox::CommsInbox::new();
    let fleet_wide = message("m1", "speaker-uuid");
    assert!(fleet_wide.recipient_ship.is_none());
    inbox.inject(fleet_wide);
    let names = BTreeMap::from([("speaker-uuid".to_string(), "speaker".to_string())]);
    let rows = collect(None, &inbox, &names);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason.id, PENDING_COMMS_FLEET_REASON);
    assert_eq!(rows[0].reason.params.get("sender").unwrap(), "speaker");
    // No blank parameter at all: the sentence that reads this id does not
    // mention a ship, so carrying an empty one would be a lie in waiting.
    assert!(!rows[0].reason.params.contains_key("ship"));
    assert!(rows[0].target.ship.is_none());
}

// ── Idle NPC advisory (issue #1435) ──────────────────────────────────────
//
// The world-level behaviour lives in `tests/gm_idle_npc.rs`, over real
// authored hulls and real GM actions. What is here is the half that cannot
// be reached from an integration test: an Objective SCOPED to a hull, whose
// only writer (`ObjectiveManager::set_recipients`) is crate-private because
// the trusted activation seam is its only caller.

/// Build the smallest world the stopwatch needs: a tick, an objective
/// manager, the watch, and ships carrying exactly the components the
/// production query filters on.
fn idle_world() -> App {
    let mut app = App::new();
    app.init_resource::<crate::sim_tick::SimTick>()
        .init_resource::<crate::world::server::ObjectiveManagerRes>()
        .init_resource::<GmIdleNpcWatch>();
    app
}

/// One hull with the given scored pool on its Viewscreen blackboard.
fn hull(app: &mut App, uuid: &str, pool: Vec<crate::core::messages::ScoredObjective>) {
    let mut blackboards = crate::server_app::ShipSystemBlackboards(Default::default());
    blackboards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(
            crate::core::messages::ViewscreenBlackboard {
                red_alert: false,
                hull_integrity_pct: 100.0,
                last_damage_taken_secs: None,
                last_weapon_fired_secs: None,
                last_attacker_uuid: None,
                scored_objectives: pool,
                combat_lock: None,
                science_target: None,
            },
        ),
    );
    app.world_mut().spawn((
        EntityUuid(uuid.to_string()),
        blackboards,
        crate::server_app::Ship,
    ));
}

fn patrol_pool(score: f32) -> Vec<crate::core::messages::ScoredObjective> {
    vec![crate::core::messages::ScoredObjective {
        id: "patrol".into(),
        score,
        directive: crate::core::messages::AiDirective::Patrol {
            anchors: vec!["a".into(), "b".into()],
            loop_path: true,
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: Vec::new(),
        snapshot: crate::core::messages::ObjectiveSnapshot {
            progress: None,
            unassigned: false,
            id: "patrol".into(),
            text: "patrol".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: Vec::new(),
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    }]
}

fn observe(app: &mut App) {
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut().run_system_once(observe_idle_npcs).unwrap();
}

/// A standing order is an order; a doctrine entry that has gated itself down
/// to zero is not, which is exactly the difference between "the ship is
/// holding station" and "the ship has nothing left to do".
#[test]
fn a_positively_scored_standing_directive_is_an_order_and_a_gated_out_one_is_not() {
    let mut app = idle_world();
    hull(&mut app, "patrolling", patrol_pool(45.0));
    hull(&mut app, "gated-out", patrol_pool(0.0));
    hull(&mut app, "no-doctrine", Vec::new());
    observe(&mut app);
    let watch = app.world().resource::<GmIdleNpcWatch>();
    assert_eq!(watch.spell("patrolling"), None);
    assert_eq!(watch.spell("gated-out").map(|s| s.ticks), Some(1));
    assert_eq!(watch.spell("no-doctrine").map(|s| s.ticks), Some(1));
}

/// An Objective addressed to this hull is a job, and resolving it puts the
/// hull back on the clock from zero. An Objective addressed to NOBODY in
/// particular is the mission's, and must not silence the advisory for every
/// NPC in the world.
#[test]
fn an_objective_scoped_to_the_hull_ends_the_spell_and_an_unscoped_one_does_not() {
    let mut app = idle_world();
    hull(&mut app, "idle-one", Vec::new());
    hull(&mut app, "idle-two", Vec::new());
    observe(&mut app);
    assert_eq!(
        app.world()
            .resource::<GmIdleNpcWatch>()
            .spell("idle-one")
            .map(|s| s.ticks),
        Some(1)
    );

    // A mission line nobody addressed changes nothing for either hull.
    {
        let mut manager = app
            .world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>();
        manager.0.add("fleet-wide", "text", false, Vec::new());
    }
    observe(&mut app);
    let watch = app.world().resource::<GmIdleNpcWatch>();
    assert_eq!(watch.spell("idle-one").map(|s| s.ticks), Some(2));
    assert_eq!(watch.spell("idle-two").map(|s| s.ticks), Some(2));

    // One addressed at `idle-one` ends only that hull's spell.
    {
        let mut manager = app
            .world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>();
        manager.0.add("escort", "text", false, Vec::new());
        assert!(manager.0.set_recipients("escort", vec!["idle-one".into()]));
    }
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 40;
    observe(&mut app);
    let watch = app.world().resource::<GmIdleNpcWatch>();
    assert_eq!(watch.spell("idle-one"), None);
    assert_eq!(watch.spell("idle-two").map(|s| s.ticks), Some(3));

    // Completing it puts the hull back on the clock — from zero, at a new
    // start tick, so the row it eventually raises is a new occurrence.
    {
        let mut manager = app
            .world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>();
        assert!(manager.0.complete("escort"));
    }
    observe(&mut app);
    assert_eq!(
        app.world().resource::<GmIdleNpcWatch>().spell("idle-one"),
        Some(GmIdleSpell {
            started_tick: 40,
            ticks: 1
        })
    );
}

/// A hull that leaves the world takes its wait with it, so a hull recreated
/// under the same identity is watched from scratch.
#[test]
fn a_ship_that_leaves_the_world_is_forgotten_rather_than_frozen() {
    let mut app = idle_world();
    hull(&mut app, "gone-soon", Vec::new());
    observe(&mut app);
    observe(&mut app);
    assert_eq!(
        app.world()
            .resource::<GmIdleNpcWatch>()
            .spell("gone-soon")
            .map(|s| s.ticks),
        Some(2)
    );
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<EntityUuid>>()
        .iter(app.world())
        .next()
        .unwrap();
    app.world_mut().despawn(entity);
    observe(&mut app);
    assert!(app.world().resource::<GmIdleNpcWatch>().is_empty());

    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 99;
    hull(&mut app, "gone-soon", Vec::new());
    observe(&mut app);
    assert_eq!(
        app.world().resource::<GmIdleNpcWatch>().spell("gone-soon"),
        Some(GmIdleSpell {
            started_tick: 99,
            ticks: 1
        })
    );
}

/// The authored knobs, as pure arithmetic over the settings.
#[test]
fn the_authored_grace_converts_to_exact_ticks_and_refuses_the_unusable() {
    let mut settings = GmAttentionSettings::default();
    assert_eq!(settings.idle_npc_grace_secs, DEFAULT_IDLE_NPC_GRACE_SECS);
    assert_eq!(settings.idle_npc_band(), GmAttentionBand::Background);
    assert_eq!(settings.idle_grace_ticks(60.0), 1800);
    assert_eq!(settings.idle_grace_ticks(30.0), 900);
    assert!(settings.validate().is_ok());

    // A grace shorter than a tick still costs a whole tick — the fixed loop
    // has no smaller unit to spend.
    settings.idle_npc_grace_secs = 0.001;
    assert_eq!(settings.idle_grace_ticks(30.0), 1);

    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        settings.idle_npc_grace_secs = bad;
        let error = settings.validate().unwrap_err();
        assert!(
            error.contains("[gm_attention]") && error.contains("idle_npc_grace_secs"),
            "{error}"
        );
    }

    settings.idle_npc_grace_secs = 5.0;
    settings.idle_npc_band = Some("urgent".into());
    assert!(settings.validate().is_ok());
    assert_eq!(settings.idle_npc_band(), GmAttentionBand::Urgent);
    settings.idle_npc_band = Some("Urgent".into());
    let error = settings.validate().unwrap_err();
    assert!(
        error.contains("idle_npc_band") && error.contains("'urgent'"),
        "{error}"
    );
}

/// The rows themselves: below the grace nothing is said, at it exactly one
/// row is, and the off switch says nothing at any age.
#[test]
fn idle_rows_appear_at_the_grace_and_the_off_switch_suppresses_them_at_any_age() {
    let mut watch = GmIdleNpcWatch::default();
    watch.spells.insert(
        "ship-a".into(),
        GmIdleSpell {
            started_tick: 7,
            ticks: 59,
        },
    );
    let names = BTreeMap::from([("ship-a".to_string(), "Drifter".to_string())]);
    let settings = GmAttentionSettings {
        idle_npc_grace_secs: 2.0,
        idle_npc_band: Some("urgent".into()),
        ..Default::default()
    };
    assert!(idle_rows(&settings, 30.0, &watch, &names).is_empty());

    watch.spells.get_mut("ship-a").unwrap().ticks = 60;
    let rows = idle_rows(&settings, 30.0, &watch, &names);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "idle:ship-a:7");
    assert_eq!(rows[0].band, GmAttentionBand::Urgent);
    assert_eq!(rows[0].category, GmAttentionCategory::IdleNpc);
    assert_eq!(rows[0].reason.id, IDLE_NPC_REASON);
    assert_eq!(rows[0].reason.params.get("ship").unwrap(), "Drifter");
    assert_eq!(rows[0].reason.params.get("idle").unwrap(), "0:02");
    assert_eq!(
        rows[0]
            .target
            .ship
            .as_ref()
            .map(|ship| ship.entity_id.as_str()),
        Some("ship-a")
    );
    assert!(rows[0].target.route.is_none());
    assert!(rows[0].target.event.is_none());

    watch.spells.get_mut("ship-a").unwrap().ticks = 30 * 3600;
    let disabled = GmAttentionSettings {
        idle_npc_disabled: true,
        ..settings
    };
    assert!(idle_rows(&disabled, 30.0, &watch, &names).is_empty());
}

/// Simulation minutes and seconds, floored — a wait is reported as the time
/// actually served, never rounded up into one the hull has not spent.
#[test]
fn the_reported_idle_age_is_floored_simulation_time() {
    assert_eq!(format_sim_clock(0, 30.0), "0:00");
    assert_eq!(format_sim_clock(29, 30.0), "0:00");
    assert_eq!(format_sim_clock(30, 30.0), "0:01");
    assert_eq!(format_sim_clock(30 * 90, 30.0), "1:30");
    assert_eq!(format_sim_clock(60 * 125, 60.0), "2:05");
    // A world with no usable rate cannot claim an age it cannot measure.
    assert_eq!(format_sim_clock(600, 0.0), "0:00");
}
