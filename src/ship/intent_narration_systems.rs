//! Bevy adapter for the pure [`crate::ship::intent_narration`] coalescer
//! (issue #879) — the sibling-adapter half of AGENTS.md #10.
//!
//! Reads each ship's authoritative decision state once per shared AI tick,
//! hands the previous and new snapshots to the coalescer, and puts whatever
//! comes back on the channel-3 bus as a
//! [`CoordinationPayload::IntentAdvisory`]. Delivery — the fan-out to every
//! human seat on the source ship — belongs to `process_coordination_lag` and
//! the pure [`crate::ship::coordination::broadcast_to_ship`] router.
//!
//! # Why it does not ask who is holding the seat
//!
//! Nothing here branches on human-vs-AI to decide whether to emit. The snapshot
//! is read from authoritative system state — the tactical selection, the ship's
//! red alert, the hull, the shield grid, the power allocation, the helm policy's
//! own state machine — and the seat's control source is stamped onto
//! `sender_origin` afterwards as a routing tag, exactly as
//! `tick_power_brownout_advisory` and `tick_sensors_frequency_hint` stamp
//! theirs. That is the shape issue #873 had to restore after an emit-side
//! `operate_ai` conjunct made a coordination fact's existence depend on who was
//! sitting at the console (AGENTS.md #6), and re-adding one here would be the
//! same bug: a human-held Helm would stop narrating even to seats that need to
//! know the ship just went to combat posture.
//!
//! What the routing tag then does is exactly right for narration without any
//! help from this module: a backfilled seat's advisory (`sender_origin == Ai`)
//! pops up at every human seat, and a human-held seat's advisory is suppressed
//! at every human seat — two officers on the same bridge already talk to each
//! other.
//!
//! # Why it is gated on the shared AI cadence
//!
//! `SimSet` is configured in Bevy's `FixedUpdate` (issue #895), so an ungated system here would
//! sample decision state once per *rendered frame* (AGENTS.md #7). The snapshot
//! pair would then be two readings 16 ms apart on a fast host and 33 ms apart on
//! a slow one, and a decision that flickered inside one AI tick could narrate
//! twice. The gate is `run_if(ai_tick_ready)` on the one shared cadence, so a
//! snapshot pair is always two consecutive AI decisions.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::messages::{CoordinationPayload, StationId, SystemId};
use crate::ship::components::CoordinationEnqueue;
use crate::ship::coordination::seat_control_source;
use crate::ship::intent_narration::{coalesce_intent, IntentNarrationConfig, IntentSnapshot};

/// Per-ship narration memory: the last decision snapshot of every narrating
/// seat, plus the ship's advisory counter.
///
/// # Why the counter is a counter
///
/// `generation` is stamped onto every advisory this ship emits and is the
/// ordering handle a client (or a replaying peer) uses to tell two advisories
/// apart. It is incremented, never read from a clock: two lockstep peers
/// advancing the same simulation must produce byte-identical advisories, and
/// `Time::elapsed_secs` differs on every host. Same rule
/// `NavigationWaypoint::generation` follows for the Channel-3 nav handoff.
///
/// # Why per ship
///
/// Narration is per seat but the counter is per ship, because the crew hears
/// one stream: two advisories from different seats on the same bridge are
/// ordered against each other, not against their own seat's history.
#[derive(Component, Clone, Debug, Default)]
pub struct ShipIntentNarration {
    /// Last snapshot per narrating station. Absent = never observed, which the
    /// coalescer treats as "record, say nothing".
    last: HashMap<StationId, IntentSnapshot>,
    /// Monotonic per-ship advisory counter. First advisory is generation 1.
    generation: u64,
}

impl ShipIntentNarration {
    /// The next advisory's generation. A counter step, never a clock read.
    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
}

/// The two functions that must attach [`ShipIntentNarration`].
///
/// Same hazard, and same guard technique, as
/// [`crate::ship::components::PER_SHIP_BUS_SPAWN_SITES`]: the PLAYER ship never
/// goes through `entities::spawner::spawn_entity`, so a per-ship component
/// wired into only one of these reaches only NPCs, silently. Issues #785, #786,
/// #882 and #885 each shipped exactly that. A ship without this component
/// simply never narrates, and nothing warns.
pub const INTENT_NARRATION_SPAWN_SITES: &[(&str, &str)] = &[
    ("src/entities/spawner.rs", "spawn_entity"),
    ("src/server_app.rs", "spawn_game_start_entities"),
];

/// The stations that narrate, and which decision axes each one reports.
///
/// Helm carries three because posture, hull break-off and the manoeuvre leg are
/// all movement decisions and all resolve on the helm seat; the coalescer's
/// ladder collapses a tick where several move at once into one advisory.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NarratingSeat {
    /// Target acquire / switch.
    Tactical,
    /// Combat posture, break-off on damage, manoeuvre legs.
    Helm,
    /// Shield arc focus.
    Shields,
    /// Power brownout.
    Power,
}

/// Resolve the station that owns the first system of any of `kinds` on this
/// hull, from the ship's own authored config.
///
/// By KIND, not by system id: the hull decides both the id and the seat. The
/// battleship's shields live on `id = "shields-system"`, the courier's need not,
/// and #801 deleted the coarse ids that would have made an id lookup work at
/// all. Resolving by kind also means a hull that puts Shields on the
/// Engineering seat narrates to the seat it authored rather than to one spelled
/// out here (AGENTS.md #11).
fn station_owning_kind(
    config: &crate::ship::config::ShipConfig,
    kinds: &[&str],
) -> Option<StationId> {
    config
        .systems
        .iter()
        .find(|s| kinds.contains(&s.kind.as_str()) && s.station.is_some())
        .and_then(|s| s.station.clone())
}

/// Emit one coarsened advisory per narrating seat whose decision changed.
///
/// Runs once per shared AI tick (see the module docs) in `SimSet::Publish`, so
/// every decision this tick — helm policy state committed in `Physics`, damage
/// in `Damage`, power and coordination in `Modifiers` — has already settled.
#[allow(clippy::too_many_arguments)]
pub fn tick_intent_narration(
    mut ships: Query<
        (
            Entity,
            &crate::ship::components::ShipConfigComponent,
            &crate::ship::components::ShipSystemControlSources,
            &mut ShipIntentNarration,
            Option<&crate::console::weapons::beam::TacticalRadarSelection>,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship::shields::ShipShields>,
            Option<&crate::ship::power::PowerBrownoutState>,
            Option<&crate::ship::helm_ai::HelmSteeringAiPolicyState>,
        ),
        With<crate::server_app::Ship>,
    >,
    entity_names: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    let cfg = IntentNarrationConfig {
        break_off_hull_fraction: world_config
            .as_deref()
            .map(|wc| wc.global.intent_break_off_hull_fraction)
            .unwrap_or_else(|| {
                crate::entity_config::GlobalConfig::default().intent_break_off_hull_fraction
            }),
    };

    for (
        entity,
        ship_config,
        control_sources,
        mut narration,
        tactical_selection,
        red_alert,
        hull,
        shields,
        brownout,
        steering_state,
    ) in ships.iter_mut()
    {
        let config = &ship_config.0;
        for seat in [
            NarratingSeat::Tactical,
            NarratingSeat::Helm,
            NarratingSeat::Shields,
            NarratingSeat::Power,
        ] {
            let Some(station_id) = seat_station(config, seat) else {
                continue;
            };
            if config.station(&station_id).is_none() {
                // The hull does not crew this seat at all (an NPC-shaped hull,
                // or a two-station courier). Nothing to narrate to and nobody
                // to narrate for.
                continue;
            }

            let next = match seat {
                NarratingSeat::Tactical => IntentSnapshot {
                    target_label: tactical_selection
                        .and_then(|sel| sel.0.as_ref())
                        .map(|uuid| entity_label(&entity_names, uuid)),
                    ..Default::default()
                },
                NarratingSeat::Helm => IntentSnapshot {
                    combat_posture: Some(red_alert.map(|ra| ra.0).unwrap_or(false)),
                    hull_fraction: hull.and_then(|h| {
                        let max = h.0.total_max();
                        (max > 0.0).then(|| h.0.total_current() / max)
                    }),
                    manoeuvre: steering_state
                        .map(|s| s.0.current.clone())
                        .filter(|name| !name.is_empty()),
                    ..Default::default()
                },
                NarratingSeat::Shields => IntentSnapshot {
                    shield_focus: shields.and_then(|s| {
                        s.0.focused_facing
                            .and_then(|idx| s.0.facings.get(idx))
                            .map(|f| f.label.clone())
                    }),
                    ..Default::default()
                },
                NarratingSeat::Power => IntentSnapshot {
                    // Sorted: the advisory names the group that newly appeared,
                    // and `notified_groups` is a `HashSet`, whose order would
                    // otherwise decide which one that is.
                    brownout_groups: {
                        let mut groups: Vec<String> = brownout
                            .map(|b| b.notified_groups.iter().cloned().collect())
                            .unwrap_or_default();
                        groups.sort();
                        groups
                    },
                    ..Default::default()
                },
            };

            let change = coalesce_intent(narration.last.get(&station_id), &next, &cfg);
            narration.last.insert(station_id.clone(), next);
            let Some(change) = change else {
                continue;
            };

            // The routing tag, stamped AFTER the decision to emit — see the
            // module docs. Reduced from the fine systems this station actually
            // owns, so a station whose systems are all backfilled reads `Ai`
            // and one with a live human console reads `Human`.
            let policies: Vec<crate::ship::control_source::ControlTickPolicy> = config
                .systems
                .iter()
                .filter(|s| s.station.as_ref() == Some(&station_id))
                .map(|s| control_sources.0.policy_for(&s.id))
                .collect();
            let sender_origin = seat_control_source(&policies);
            let generation = narration.next_generation();

            writer.write(CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                target: SystemId(station_id.0.clone()),
                payload: CoordinationPayload::IntentAdvisory {
                    kind: change.kind,
                    subject: change.subject,
                    generation,
                },
                // Emit the station's derived display-name id (issue #975), not
                // the English `name` — `localiseTree` resolves it on the client.
                sender_label: format!("station.{}.name", station_id.0),
            });
        }
    }
}

/// Which station a narrating seat is on THIS hull.
///
/// Helm and Tactical are station ids in their own right (the two console-level
/// keys #801 introduced); Shields and Power are resolved through the system
/// each one owns, so the answer comes from the hull's authored config rather
/// than from a station id spelled out here.
fn seat_station(
    config: &crate::ship::config::ShipConfig,
    seat: NarratingSeat,
) -> Option<StationId> {
    match seat {
        NarratingSeat::Tactical => Some(StationId(
            crate::system_registry::TACTICAL_STATION_ID.to_string(),
        )),
        NarratingSeat::Helm => Some(StationId(
            crate::system_registry::HELM_STATION_ID.to_string(),
        )),
        NarratingSeat::Shields => station_owning_kind(
            config,
            &[
                crate::system_registry::SHIELDS_KIND,
                crate::system_registry::SHIELD_ARC_KIND,
            ],
        ),
        NarratingSeat::Power => {
            station_owning_kind(config, &[crate::system_registry::POWER_REACTOR_KIND])
        }
    }
}

/// A contact's human-readable name, falling back to its uuid — the same
/// resolution `ship::sensors` uses for target designations.
fn entity_label(
    entity_names: &Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    uuid: &str,
) -> String {
    entity_names
        .iter()
        .find_map(|(u, n)| (u.0 == uuid).then(|| n.0.clone()))
        .unwrap_or_else(|| uuid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_source::ControlSource;
    use crate::messages::IntentKind;
    use crate::ship::test_support::*;
    use crate::simulation::Ship;

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
            .insert(crate::weapons_plugin::TacticalRadarSelection(
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
            .insert(crate::ship_state::ShipRedAlert(true));
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
        assert_eq!(
            advisory_kinds(&drain(&mut app)),
            vec![IntentKind::TargetAcquired]
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
        assert_eq!(
            advisory_kinds(&drain(&mut app)),
            vec![IntentKind::TargetSwitched]
        );
    }

    /// AC: combat posture, both directions, from the ship's own red alert.
    #[test]
    fn entering_and_leaving_red_alert_narrates_combat_posture() {
        let mut app = narration_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_state::ShipRedAlert(false));
        settle(&mut app);

        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_state::ShipRedAlert(true));
        decide(&mut app);
        assert_eq!(
            advisory_kinds(&drain(&mut app)),
            vec![IntentKind::CombatPostureEntered]
        );

        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_state::ShipRedAlert(false));
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
        let threshold =
            crate::entity_config::GlobalConfig::default().intent_break_off_hull_fraction;
        settle(&mut app);

        // Take the whole hull to just under the authored fraction.
        {
            let ship = find_ship_entity(&mut app);
            let mut hull = app
                .world_mut()
                .get_mut::<crate::entity_spawner::EntitySystemHull>(ship)
                .expect("the fixture ship carries a hull");
            let entries: Vec<(crate::messages::SystemId, f32)> = hull
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
        assert!(matches!(
            &msgs[0].payload,
            CoordinationPayload::IntentAdvisory { subject: Some(s), .. } if s == "weapons"
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
        let ids: Vec<crate::messages::SystemId> = {
            let mut q = app
                .world_mut()
                .query_filtered::<&crate::ship::components::ShipConfigComponent, With<Ship>>();
            let cfg = q.single(app.world()).expect("ship config").0.clone();
            cfg.systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str())
                        == Some(crate::system_registry::TACTICAL_STATION_ID)
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

        let hz = crate::entity_config::GlobalConfig::default().ai_tick_hz;
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
        use crate::entities::ai_declaration_manifest::source_scan::{
            function_body, read_non_test_source,
        };
        assert!(
            !INTENT_NARRATION_SPAWN_SITES.is_empty(),
            "the scan must have something to check"
        );
        for (file, func) in INTENT_NARRATION_SPAWN_SITES {
            let src = read_non_test_source(file);
            let body = function_body(&src, func);
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
}
