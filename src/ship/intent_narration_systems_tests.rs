use super::*;
use crate::core::messages::IntentKind;
use crate::server_app::Ship;
use crate::ship::control_source::ControlSource;
use crate::ship::test_support::*;

#[derive(Resource, Default)]
struct AdvisoryBox(Vec<CoordinationEnqueue>);

fn collect_advisories(
    mut reader: MessageReader<CoordinationEnqueue>,
    mut box_: ResMut<AdvisoryBox>,
) {
    for m in reader.read() {
        if matches!(m.payload, CoordinationPayload::IntentAdvisory { .. }) {
            box_.0.push(m.clone());
        }
    }
}

fn narration_app() -> App {
    let mut app = test_app();
    app.init_resource::<AdvisoryBox>()
        .add_systems(PostUpdate, collect_advisories);
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(ShipIntentNarration::default());
    app
}

/// One AI DECISION tick.
///
/// The narrator is gated on the shared cadence, so a bare `app.update()` is
/// a rendered frame that may or may not be a decision. Fixtures asserting on
/// decision CONTENT arm the latch by hand — the sanctioned helper for
/// exactly this (`ai::cadence::arm_ai_tick`) — rather than relying on an
/// evaluate-every-frame fallback production does not have. The one fixture
/// that asserts on the CADENCE itself drives `Time` instead and never calls
/// this.
fn decide(app: &mut App) {
    crate::ai::cadence::arm_ai_tick(app);
    tick(app);
}

/// Run out the boot transients — a ship's helm policy enters its authored
/// initial leg a tick or two after spawn, which is a real decision and does
/// narrate — so the assertions below can be exact about what follows.
fn settle(app: &mut App) {
    for _ in 0..6 {
        decide(app);
    }
    drain(app);
}

fn drain(app: &mut App) -> Vec<CoordinationEnqueue> {
    let msgs = app.world().resource::<AdvisoryBox>().0.clone();
    app.world_mut().resource_mut::<AdvisoryBox>().0.clear();
    msgs
}

fn set_target(app: &mut App, uuid: Option<&str>) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::console::weapons::TacticalRadarSelection(
            uuid.map(|u| u.to_string()),
        ));
}

fn advisory_kinds(msgs: &[CoordinationEnqueue]) -> Vec<IntentKind> {
    msgs.iter()
        .filter_map(|m| match &m.payload {
            CoordinationPayload::IntentAdvisory { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect()
}

fn generations(msgs: &[CoordinationEnqueue]) -> Vec<u64> {
    msgs.iter()
        .filter_map(|m| match &m.payload {
            CoordinationPayload::IntentAdvisory { generation, .. } => Some(*generation),
            _ => None,
        })
        .collect()
}

/// AC: nothing in steady state. The ship boots, nothing decides anything
/// new, and 20 ticks go by in silence — the case that matters, because the
/// alternative is an advisory per shot and per thrust tick.
#[test]
fn a_ship_whose_decisions_do_not_change_narrates_nothing() {
    let mut app = narration_app();
    let ship = find_ship_entity(&mut app);

    // A ship mid-engagement HOLDING several live decisions, not an idle one
    // with nothing to say: a target locked, red alert set, a shield arc
    // focused, a power group browning out. This is the shape that produces
    // shots and thrust ticks by the dozen, and it must still be silent.
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::state::ShipRedAlert(true));
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::power::PowerBrownoutState {
            notified_groups: ["weapons".to_string()].into_iter().collect(),
            ..Default::default()
        });
    set_target(&mut app, Some("harrow-raider-1"));
    {
        let mut shields = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(ship)
            .expect("the fixture ship carries shields");
        shields.0.set_focused_facing(Some(0));
    }
    settle(&mut app);

    for _ in 0..20 {
        decide(&mut app);
    }
    assert!(
        drain(&mut app).is_empty(),
        "a bridge holding its decisions must produce no advisories at all, \
             however many ticks it holds them for"
    );
}

/// AC: target acquire, then switch — one advisory each, and silence while
/// the target is held.
#[test]
fn acquiring_then_switching_target_narrates_once_each() {
    let mut app = narration_app();
    settle(&mut app);

    set_target(&mut app, Some("harrow-raider-1"));
    decide(&mut app);
    let acquired = drain(&mut app);
    assert_eq!(advisory_kinds(&acquired), vec![IntentKind::TargetAcquired]);
    assert_eq!(
        acquired[0].presentation.title,
        "coordination.intent.target_acquired"
    );
    let CoordinationPayload::IntentAdvisory {
        subject: Some(subject),
        ..
    } = &acquired[0].payload
    else {
        panic!("target acquisition must retain its typed subject")
    };
    assert_eq!(
        &acquired[0].presentation.body, subject,
        "the producer carries the authored-or-literal subject without a client payload switch"
    );

    // Held: several decision ticks with the same lock say nothing.
    for _ in 0..5 {
        decide(&mut app);
    }
    assert!(
        drain(&mut app).is_empty(),
        "holding a lock is not a decision"
    );

    set_target(&mut app, Some("harrow-lance-2"));
    decide(&mut app);
    let switched = drain(&mut app);
    assert_eq!(advisory_kinds(&switched), vec![IntentKind::TargetSwitched]);
    assert_eq!(
        switched[0].presentation.title,
        "coordination.intent.target_switched"
    );
}

/// AC: combat posture, both directions, from the ship's own red alert.
#[test]
fn entering_and_leaving_red_alert_narrates_combat_posture() {
    let mut app = narration_app();
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::state::ShipRedAlert(false));
    settle(&mut app);

    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::state::ShipRedAlert(true));
    decide(&mut app);
    assert_eq!(
        advisory_kinds(&drain(&mut app)),
        vec![IntentKind::CombatPostureEntered]
    );

    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::state::ShipRedAlert(false));
    decide(&mut app);
    assert_eq!(
        advisory_kinds(&drain(&mut app)),
        vec![IntentKind::CombatPostureLeft]
    );
}

/// AC: break-off on damage, at the AUTHORED threshold. The hull is driven
/// across the world's own `intent_break_off_hull_fraction` rather than
/// across a number written here, so retuning the world retunes the test.
#[test]
fn crossing_the_authored_hull_threshold_narrates_breaking_off() {
    let mut app = narration_app();
    let threshold = crate::entities::config::GlobalConfig::default().intent_break_off_hull_fraction;
    settle(&mut app);

    // Take the whole hull to just under the authored fraction.
    {
        let ship = find_ship_entity(&mut app);
        let mut hull = app
            .world_mut()
            .get_mut::<crate::entities::spawner::EntitySystemHull>(ship)
            .expect("the fixture ship carries a hull");
        let entries: Vec<(crate::core::messages::SystemId, f32)> = hull
            .0
            .entries()
            .map(|(id, _, max)| (id.clone(), max))
            .collect();
        assert!(!entries.is_empty(), "precondition: the hull has entries");
        for (id, max) in entries {
            hull.0.set_hp(&id, max * (threshold - 0.1).max(0.0));
        }
    }
    decide(&mut app);
    assert_eq!(
        advisory_kinds(&drain(&mut app)),
        vec![IntentKind::BreakingOff]
    );

    // Still below: steady state, not a new decision.
    for _ in 0..5 {
        decide(&mut app);
    }
    assert!(drain(&mut app).is_empty());
}

/// AC: shield-arc focus.
#[test]
fn focusing_a_shield_arc_narrates_once() {
    let mut app = narration_app();
    settle(&mut app);

    {
        let ship = find_ship_entity(&mut app);
        let mut shields = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(ship)
            .expect("the fixture ship carries shields");
        shields.0.set_focused_facing(Some(0));
    }
    decide(&mut app);
    let msgs = drain(&mut app);
    assert_eq!(advisory_kinds(&msgs), vec![IntentKind::ShieldArcFocused]);
    assert!(
        matches!(
            &msgs[0].payload,
            CoordinationPayload::IntentAdvisory { subject: Some(s), .. } if !s.is_empty()
        ),
        "the advisory names the facing it focused"
    );
}

/// AC: brownout.
#[test]
fn a_power_group_entering_brownout_narrates_once() {
    let mut app = narration_app();
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::power::PowerBrownoutState::default());
    settle(&mut app);

    {
        let mut brownout = app
            .world_mut()
            .get_mut::<crate::ship::power::PowerBrownoutState>(ship)
            .expect("brownout state inserted above");
        brownout.notified_groups.insert("weapons".to_string());
    }
    decide(&mut app);
    let msgs = drain(&mut app);
    assert_eq!(advisory_kinds(&msgs), vec![IntentKind::PowerBrownout]);
    // The subject is the group's `strings.csv` label id (issue #977). The
    // producer also carries it as the generic presentation body, where
    // `localiseTree` resolves it without a client IntentKind switch.
    assert!(matches!(
        &msgs[0].payload,
        CoordinationPayload::IntentAdvisory { subject: Some(s), .. }
            if s == "power.group.weapons"
    ));

    for _ in 0..5 {
        decide(&mut app);
    }
    assert!(
        drain(&mut app).is_empty(),
        "a group that is still browning out is steady state"
    );
}

/// AC: lockstep determinism — the generation is a COUNTER.
///
/// Two advisories separated by a long stretch of simulated time are one
/// apart, and the elapsed time between them appears nowhere in the value. A
/// `Time::elapsed_secs` stamp would have moved by seconds across the idle
/// stretch below and would differ between two peers of the same lockstep
/// session.
#[test]
fn the_advisory_generation_is_a_counter_not_a_timestamp() {
    let mut app = narration_app();
    settle(&mut app);

    set_target(&mut app, Some("harrow-raider-1"));
    decide(&mut app);
    let first = generations(&drain(&mut app));
    assert_eq!(first.len(), 1, "precondition: exactly one advisory");

    // Burn a lot of simulated time with no decision change.
    for _ in 0..30 {
        decide(&mut app);
    }
    assert!(drain(&mut app).is_empty());

    set_target(&mut app, Some("harrow-lance-2"));
    decide(&mut app);
    let second = generations(&drain(&mut app));

    assert_eq!(
        second,
        vec![first[0] + 1],
        "the next advisory is the next COUNT — six seconds of simulated \
             time later, which a timestamp would have shown"
    );
}

/// AGENTS.md #6: the advisory is emitted from authoritative state whatever
/// the seat's control source, and `sender_origin` is the routing tag
/// stamped afterwards.
///
/// This is the #873 shape. An emit-side `operate_ai` conjunct would make
/// the human-held case silent instead of `Human`-stamped, and the whole
/// "two officers coordinate IRL" arm of the delivery matrix would become
/// unreachable from narration.
#[test]
fn sender_origin_follows_the_seat_and_never_gates_the_emission() {
    for (source, expected) in [
        (ControlSource::Ai, ControlSource::Ai),
        (ControlSource::Human, ControlSource::Human),
    ] {
        let mut app = narration_app();
        set_tactical_station_source(&mut app, source);
        settle(&mut app);

        set_target(&mut app, Some("harrow-raider-1"));
        decide(&mut app);
        let msgs = drain(&mut app);
        assert_eq!(
            advisory_kinds(&msgs),
            vec![IntentKind::TargetAcquired],
            "the fact is derived from the ship's own selection, so it is \
                 emitted whoever is holding Tactical"
        );
        assert_eq!(msgs[0].sender_origin, expected);
    }
}

/// Put every system the Tactical station owns on `source`, which is what
/// claiming or vacating the seat does.
fn set_tactical_station_source(app: &mut App, source: ControlSource) {
    let ids: Vec<crate::core::messages::SystemId> = {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship::components::ShipConfigComponent, With<Ship>>();
        let cfg = q.single(app.world()).expect("ship config").0.clone();
        cfg.systems
            .iter()
            .filter(|s| {
                s.station.as_ref().map(|st| st.0.as_str())
                    == Some(crate::ship::system_registry::TACTICAL_STATION_ID)
            })
            .map(|s| s.id.clone())
            .collect()
    };
    assert!(
        !ids.is_empty(),
        "the shipped hull must give the Tactical station systems for this \
             fixture to mean anything"
    );
    for id in ids {
        set_fine_control_source(app, id, source);
    }
}

/// AGENTS.md #7: the narrator samples on the shared AI cadence, not per
/// rendered frame.
///
/// The app is the production one (`test_app` builds `ShipPlugin`), so the
/// registration under test is the shipped one. The ship's target is changed
/// on **every rendered frame** for a stretch of simulated time; a narrator
/// with no `run_if` would take a decision snapshot on each of those frames
/// and narrate a switch every time, at whatever rate the host happens to
/// render. The bound is derived from the authored `[global] ai_tick_hz`
/// rather than written as a literal, so retuning the cadence retunes it.
#[test]
fn narration_samples_on_the_ai_cadence_not_per_frame() {
    const FRAMES: usize = 60;
    const FRAME_MS: u64 = 5;

    let mut app = narration_app();
    // Deliberately NO `arm_ai_tick` in this fixture: it is the one that
    // asserts on the cadence, so the latch is driven by `Time` exactly as
    // production drives it.
    settle(&mut app);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(FRAME_MS),
    ));

    for i in 0..FRAMES {
        set_target(&mut app, Some(&format!("contact-{i}")));
        tick(&mut app);
    }
    let narrated = advisory_kinds(&drain(&mut app)).len();

    let hz = crate::entities::config::GlobalConfig::default().ai_tick_hz;
    let span_secs = (FRAMES as f32) * (FRAME_MS as f32) / 1000.0;
    // +1 for the part-period the settle left on the shared timer.
    let max_decisions = (span_secs * hz).ceil() as usize + 1;

    assert!(
        narrated >= 1,
        "precondition: {FRAMES} target switches over {span_secs}s must \
             narrate something at all"
    );
    assert!(
        narrated <= max_decisions,
        "{narrated} advisories for {FRAMES} rendered frames spanning \
             {span_secs}s at the authored {hz} Hz decision rate — at most \
             {max_decisions} decisions happened, so the narrator is sampling \
             per FRAME, not per AI tick"
    );
    assert!(
        narrated < FRAMES,
        "an ungated narrator produces one advisory per rendered frame"
    );
}

/// AC: the narration state reaches the PLAYER ship too.
///
/// `spawn_game_start_entities` is the hand-rolled second spawn path that
/// `entities::spawner::spawn_entity` does not feed. A ship without
/// `ShipIntentNarration` narrates nothing, silently — the same failure four
/// earlier issues shipped, which is why the attachment is re-derived from
/// the crate's own source rather than trusted.
#[test]
fn the_narration_state_is_attached_at_every_spawn_site() {
    use crate::entities::ai_declaration_manifest::source_scan::spawn_site_source;
    assert!(
        !INTENT_NARRATION_SPAWN_SITES.is_empty(),
        "the scan must have something to check"
    );
    for (file, func) in INTENT_NARRATION_SPAWN_SITES {
        let body = spawn_site_source(file, func);
        assert!(
            body.contains("ShipIntentNarration"),
            "{file}::{func} never mentions `ShipIntentNarration`. Either the \
                 attachment moved (point INTENT_NARRATION_SPAWN_SITES at where it \
                 went) or this path never got it — and for \
                 `spawn_game_start_entities` that means the PLAYER ship's \
                 backfilled seats narrate nothing to its human crew, silently."
        );
    }
}
