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

use crate::core::messages::{CoordinationPayload, StationId, SystemId};
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
    /// Read the authoritative coalescer memory for continuation projection.
    pub(crate) fn continuation(&self) -> (&HashMap<StationId, IntentSnapshot>, u64) {
        (&self.last, self.generation)
    }

    /// Replace bootstrap memory with a restored continuation.
    pub(crate) fn replace_continuation(
        &mut self,
        last: HashMap<StationId, IntentSnapshot>,
        generation: u64,
    ) {
        self.last = last;
        self.generation = generation;
    }

    /// The next advisory's generation. A counter step, never a clock read.
    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
}

/// One installer attaches narration on every generic or GameStart ship.
/// See also [`crate::ship::components::PER_SHIP_BUS_SPAWN_SITES`].
pub const INTENT_NARRATION_SPAWN_SITES: &[(&str, &str)] = &[(
    "crates/phoenix-simulation/src/entities/ship_spawn.rs",
    "install",
)];

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
            Option<&crate::ship::state::ShipRedAlert>,
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
                crate::entities::config::GlobalConfig::default().intent_break_off_hull_fraction
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
                    // otherwise decide which one that is. Each group id is
                    // mapped to its `strings.csv` label id (issue #977) so the
                    // advisory `subject` — rendered raw in the popup body — is an
                    // id `localiseTree` resolves, not a bare machine token.
                    brownout_groups: {
                        let mut groups: Vec<String> = brownout
                            .map(|b| {
                                b.notified_groups
                                    .iter()
                                    .map(|g| crate::ship::power::power_group_label(g).to_string())
                                    .collect()
                            })
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
            let body = change.subject.clone().unwrap_or_default();
            let presentation = match change.kind {
                crate::core::messages::IntentKind::TargetAcquired => {
                    crate::core::messages::CoordinationPresentation::new(
                        "coordination.intent.target_acquired",
                        body,
                    )
                }
                crate::core::messages::IntentKind::TargetSwitched => {
                    crate::core::messages::CoordinationPresentation::new(
                        "coordination.intent.target_switched",
                        body,
                    )
                }
                crate::core::messages::IntentKind::CombatPostureEntered => {
                    crate::core::messages::CoordinationPresentation::titled(
                        "coordination.intent.combat_posture_entered",
                    )
                }
                crate::core::messages::IntentKind::CombatPostureLeft => {
                    crate::core::messages::CoordinationPresentation::titled(
                        "coordination.intent.combat_posture_left",
                    )
                }
                crate::core::messages::IntentKind::BreakingOff => {
                    crate::core::messages::CoordinationPresentation::titled(
                        "coordination.intent.breaking_off",
                    )
                }
                crate::core::messages::IntentKind::ShieldArcFocused => {
                    crate::core::messages::CoordinationPresentation::new(
                        "coordination.intent.shield_arc_focused",
                        body,
                    )
                }
                crate::core::messages::IntentKind::PowerBrownout => {
                    crate::core::messages::CoordinationPresentation::new(
                        "coordination.intent.power_brownout",
                        body,
                    )
                }
                crate::core::messages::IntentKind::ManoeuvreBegun => {
                    crate::core::messages::CoordinationPresentation::new(
                        "coordination.intent.manoeuvre_begun",
                        body,
                    )
                }
            };

            writer.write(CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                address: crate::core::messages::CoordinationAddress::Ship,
                payload: CoordinationPayload::IntentAdvisory {
                    kind: change.kind,
                    subject: change.subject,
                    generation,
                },
                presentation,
                // Emit the station's derived display-name id (issue #975), not
                // the English `name` — `localiseTree` resolves it on the client.
                sender_label: format!("station.{}.name", station_id.0),
                // Already a resolved station id, so opt out of the enqueue-time
                // system→station resolution with an empty `sender_system`.
                sender_system: SystemId(String::new()),
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
            crate::ship::system_registry::TACTICAL_STATION_ID.to_string(),
        )),
        NarratingSeat::Helm => Some(StationId(
            crate::ship::system_registry::HELM_STATION_ID.to_string(),
        )),
        NarratingSeat::Shields => station_owning_kind(
            config,
            &[
                crate::ship::system_registry::SHIELDS_KIND,
                crate::ship::system_registry::SHIELD_ARC_KIND,
            ],
        ),
        NarratingSeat::Power => {
            station_owning_kind(config, &[crate::ship::system_registry::POWER_REACTOR_KIND])
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
#[path = "intent_narration_systems_tests.rs"]
mod tests;
