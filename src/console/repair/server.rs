use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::damage::DamageTier;
use crate::messages::ModifierSlot;
use crate::messages::{
    QueueEntryPreview, RepairBlackboard, ServerMessage, SystemBlackboard, SystemHullStatus,
    SystemId, TeamSlot,
};
use crate::modifiers::ShipModifiers;
use crate::repair_teams::RepairTeams;
use crate::ship::system_registry::{repair_system_id, REPAIR_SYSTEM_ID};
use crate::ship_plugin::ShipSystemControlSources;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Per-entity component wrapping the pure `RepairTeams` state machine.
///
/// Issue #830 dropped the legacy global `Resource` derive: every ship reads and
/// writes its own `ShipRepairTeams` component (player + NPC alike), so there is
/// no ship-wide singleton to fall back to.
#[derive(Component, Clone)]
pub struct ShipRepairTeams(pub RepairTeams);

/// Priority queue of pending repair requests for a ship (issue #682).
/// Sorted by severity (worst tier first, then largest deficit).
/// Deduped by station_id: a new request for an already-queued station keeps
/// the worst tier and largest deficit.
#[derive(Component, Clone, Debug, Default)]
pub struct RepairRequestQueue {
    pub entries: Vec<RepairQueueEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepairQueueEntry {
    pub station_id: String,
    pub station_label: String,
    pub tier: DamageTier,
    pub deficit: f32,
}

impl RepairRequestQueue {
    pub fn push_or_merge(&mut self, entry: RepairQueueEntry) {
        if entry.tier == DamageTier::Destroyed {
            return;
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.station_id == entry.station_id)
        {
            if entry.tier > existing.tier {
                existing.tier = entry.tier;
            }
            if entry.deficit > existing.deficit {
                existing.deficit = entry.deficit;
            }
            if !entry.station_label.is_empty() {
                existing.station_label = entry.station_label.clone();
            }
        } else {
            self.entries.push(entry);
        }
    }

    /// Pop the highest-severity entry (worst tier, then largest deficit).
    pub fn pop_worst(&mut self) -> Option<RepairQueueEntry> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self
            .entries
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.tier
                    .partial_cmp(&b.tier)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        a.deficit
                            .partial_cmp(&b.deficit)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .map(|(i, _)| i)
            .unwrap();
        Some(self.entries.swap_remove(idx))
    }

    pub fn remove_station(&mut self, station_id: &str) {
        self.entries.retain(|e| e.station_id != station_id);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn peek(&self) -> Option<&RepairQueueEntry> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.iter().max_by(|a, b| {
            a.tier
                .partial_cmp(&b.tier)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.deficit
                        .partial_cmp(&b.deficit)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                tick_repair_teams.in_set(crate::sim_sets::SimSet::Physics),
                operate_repair_ai.in_set(crate::sim_sets::SimSet::Physics),
                publish_repair_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        )
        .add_plugins(repair_state_broadcaster());
        // The dispatch router registers itself in Physics, pinning its own
        // `.after(operate_repair_ai)` ordering (issue #830). See `super::dispatch`.
        super::dispatch::register_repair_dispatch(app);
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `RepairState` producer.
///
/// Broadcasts `RepairState` at 10 Hz to the `Repair` console holder only.
/// Registered by [`RepairPlugin`].
///
/// Reads the LocalShip's own per-entity `ShipRepairTeams` component (issue #830
/// dropped the global-Resource fallback). Stays `LocalShip`-filtered: this is
/// the player's own repair wire, and NPC team state never reaches a client.
pub fn repair_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::HoldingSystem(SystemId("repair".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let mut q =
                world.query_filtered::<&ShipRepairTeams, With<crate::server_app::LocalShip>>();
            let Some(slots) = q.iter(world).next().map(|t| t.0.slots().to_vec()) else {
                return vec![];
            };
            vec![ServerMessage::RepairState { teams: slots }]
        },
    )
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Tick repair teams each frame: advance timers and apply HP restoration.
///
/// Iterates every ship (`With<Ship>`) — player and NPC — so ships with a
/// per-entity `ShipRepairTeams` component (spawned when their TOML declares
/// a `[repair]` block) tick their own teams against their own
/// `EntitySystemHull`. Each ship applies its own `ShipModifiers.RepairRate`
/// multiplier.
pub fn tick_repair_teams(
    time: Res<Time>,
    mut ship_q: Query<
        (
            Option<&mut ShipRepairTeams>,
            &ShipModifiers,
            &mut crate::entity_spawner::EntitySystemHull,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let dt = time.delta_secs();

    for (teams_comp, modifiers, mut hull) in ship_q.iter_mut() {
        let Some(mut teams) = teams_comp else {
            continue;
        };
        let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
        teams.0.tick(dt * repair_mult, &mut hull.0);
    }
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

/// Per-`Ship` publisher (issue #830). Each ship builds its own repair blackboard
/// from its own `ShipRepairTeams` / `EntitySystemHull` / `RepairRequestQueue`
/// components and writes it into its own `ShipSystemBlackboards`. Ships without a
/// `[repair]` block carry no `ShipRepairTeams`; the missing-default idiom gives
/// them an empty team set. Only ships with `[behaviour]` carry
/// `ShipSystemBlackboards`, so the query naturally scopes to AI-bearing ships;
/// the wire broadcaster stays `LocalShip`-filtered.
fn publish_repair_blackboard(
    mut ship_q: Query<
        (
            Option<&ShipRepairTeams>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&RepairRequestQueue>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (teams_opt, hull_opt, repair_queue_ref, mut blackboards) in ship_q.iter_mut() {
        let default_teams;
        let teams: &ShipRepairTeams = match teams_opt {
            Some(t) => t,
            None => {
                default_teams = ShipRepairTeams(crate::repair_teams::RepairTeams::default());
                &default_teams
            }
        };
        let team_slots: Vec<TeamSlot> = teams.0.slots().to_vec();

        // Build the SystemHullStatus list from the authoritative `SystemHull`
        // iteration.
        let system_hull: Vec<SystemHullStatus> = hull_opt
            .map(|h| {
                h.0.iter()
                    .map(|(sid, entry)| SystemHullStatus {
                        system_id: sid.clone(),
                        display_name: entry.display_name.clone(),
                        current: entry.current,
                        max_hp: entry.max,
                        tier: h.0.tier_for(sid),
                        debuff_magnitude: h.0.debuff_magnitude_for(sid),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let damageable_systems: Vec<SystemId> =
            system_hull.iter().map(|s| s.system_id.clone()).collect();

        let queue_depth: Vec<QueueEntryPreview> = repair_queue_ref
            .map(|rq| {
                let mut entries = rq.entries.clone();
                entries.sort_by(|a, b| {
                    b.tier.cmp(&a.tier).then_with(|| {
                        b.deficit
                            .partial_cmp(&a.deficit)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                });
                entries
                    .iter()
                    .map(|e| QueueEntryPreview {
                        station_id: e.station_id.clone(),
                        station_label: e.station_label.clone(),
                        tier: e.tier,
                        deficit: e.deficit,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Emit the new SystemId-keyed hull + damageable list only (legacy
        // `console_hull` / `damageable_consoles` wire fields were dropped in #619).
        let bb = RepairBlackboard {
            teams: team_slots,
            travel_duration_secs: teams.0.timings().travel_duration,
            system_hull,
            damageable_systems,
            // Host-internal copy: unprojected. `system_hull` and `queue_depth` both
            // carry exact per-system detail and are filtered on the wire by
            // `visibility::project_repair_blackboard`, which also fills in the
            // aggregate (issue #737). The repair AI controller reads this copy and
            // needs every system.
            queue_depth,
            aggregate_hull_fraction: None,
        };

        blackboards.0.insert(
            SystemId(REPAIR_SYSTEM_ID.to_string()),
            SystemBlackboard::Repair(bb),
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// ── AI controller stub ─────────────────────────────────────────────────────────

pub fn all_systems_in_station_are_operational(
    station_id: &str,
    hull: &crate::damage::SystemHull,
    config: &crate::ship::config::ShipConfig,
) -> bool {
    let systems_in_station: Vec<_> = config
        .systems
        .iter()
        .filter(|s| s.station.as_ref().map(|st| st.0.as_str()) == Some(station_id))
        .collect();
    !systems_in_station.is_empty()
        && systems_in_station
            .iter()
            .all(|s| hull.tier_for(&s.id) == DamageTier::Operational)
}

// `best_damaged_system_in_station` was removed in #830: `operate_repair_ai` now
// emits a station-granular `DispatchRepairTeam` and the host repair router's
// `resolve_repair_target` picks the fine system in the station, so the AI no
// longer resolves the fine system inline. See `operate_repair_ai` for why this
// is a deliberate change of the healed-first heuristic, not an equivalence.

/// Per-kind AI loop for repair. Iterates every ship (`With<Ship>`) whose
/// Repair system is `ControlSource::Ai` and auto-dispatches idle teams to the
/// station of each unassigned repair-queue entry (worst tier, then largest
/// deficit). Ships with no per-entity `ShipRepairTeams` component silently
/// skip — an NPC without a `[repair]` block simply has no teams to dispatch.
///
/// After PRD #597 gap-5 closure: same code path for player Backfill AI and
/// NPC AI. The only differentiator is `ShipSystemControlSources`
/// (data-driven) and `LocalShip` marker.
///
/// Decide-and-emit (issue #830): the queue-based *station* decision is
/// unchanged, but instead of calling `teams.dispatch(..)` directly (the §2
/// violation) each assignment is emitted as an admitted `DispatchRepairTeam {
/// team_idx, target: Station(..) }` through the shared
/// [`crate::command_admission::validate_and_admit`] seam with this ship's own
/// `ai:<uuid>` token — the identical admitted path a human Engineering dispatch
/// takes. `handle_dispatch_repair_team` applies it later this tick (Physics,
/// `.after(operate_repair_ai)`).
///
/// # Which fine system heals first is now the shared applier's call
///
/// A station-granular admitted payload cannot carry the AI's old *private*
/// per-system choice, so the fine target is resolved by the router's
/// `resolve_repair_target` — the same code a human dispatch runs. This is the
/// point of admission symmetry (§2): both sources resolve the fine system
/// identically. It is a deliberate change, not an equivalence. The retired
/// inline `best_damaged_system_in_station` ranked candidates by **absolute HP
/// deficit** (`max - current`); `resolve_repair_target` ranks by **damage
/// fraction** (`1 - current/max`). For a station owning a single repairable
/// system the two agree, but shipped hulls have multi-system stations of
/// differing max HP (e.g. `alliance_destroyer`'s helm owns
/// `helm-engine-{port,starboard}` / `helm-radar` at max 15 and
/// `helm-lateral-thrust` at max 10), so when several of a station's systems are
/// damaged at once the healed-first system can differ from the pre-#830 choice.
/// The AI adopting the human path's fraction-ranking is the intended refinement
/// (same class as #826 dissolving the AI's bespoke resolution into the shared
/// seam), not a regression.
pub fn operate_repair_ai(
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Option<&crate::entity_spawner::EntityUuid>,
            &ShipSystemControlSources,
            Option<&ShipRepairTeams>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&mut RepairRequestQueue>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (
        entity_uuid,
        sources,
        teams_comp,
        hull_comp,
        repair_queue_comp,
        config_comp,
        mut admitted,
    ) in ships.iter_mut()
    {
        let policy = sources.0.policy_for(&repair_system_id());
        if !policy.operate_ai {
            continue;
        }
        // The queue + config are always present together on production ships
        // (the entity spawner inserts both unconditionally). Ships lacking
        // either simply have nothing to auto-dispatch — the old queue-less
        // hull-poll fallback (a direct-write §2 violation) is removed (#830).
        let (Some(teams), Some(hull), Some(mut rq), Some(config)) =
            (teams_comp, hull_comp, repair_queue_comp, config_comp)
        else {
            continue;
        };

        rq.entries.retain(|entry| {
            config
                .0
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| {
                    let t = hull.0.tier_for(&s.id);
                    t != DamageTier::Operational && t != DamageTier::Destroyed
                })
        });

        // Determine which stations already have at least one active team
        // (Travelling or Repairing), so idle teams are directed to
        // unassigned queue entries only (Option C).
        let assigned_stations: std::collections::HashSet<String> = teams
            .0
            .slots()
            .iter()
            .filter_map(|slot| match slot {
                TeamSlot::Travelling { system_id, .. } | TeamSlot::Repairing { system_id, .. } => {
                    system_id.as_ref().and_then(|sid| {
                        config
                            .0
                            .system(sid)
                            .and_then(|sc| sc.station.as_ref())
                            .map(|s| s.0.clone())
                    })
                }
                _ => None,
            })
            .collect();

        // Sort entries by priority (worst tier, then largest deficit).
        let mut sorted_entries: Vec<&RepairQueueEntry> = rq.entries.iter().collect();
        sorted_entries.sort_by(|a, b| {
            b.tier.cmp(&a.tier).then_with(|| {
                b.deficit
                    .partial_cmp(&a.deficit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        // Free team indices, consumed as we assign. Because emission does not
        // mutate `teams` this tick (the applier does, in Physics after us),
        // `lowest_free_team()` would return the same idx for every entry — so we
        // draw from a locally-consumed list instead.
        let mut free_teams = teams
            .0
            .slots()
            .iter()
            .enumerate()
            .filter_map(|(i, s)| matches!(s, TeamSlot::Idle).then_some(i));

        // Emit an admitted DispatchRepairTeam for each idle team → unassigned
        // queue entry. Station-granular: the applier resolves the fine system.
        let mut newly_assigned: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for entry in &sorted_entries {
            if assigned_stations.contains(&entry.station_id)
                || newly_assigned.contains(&entry.station_id)
            {
                continue;
            }
            let Some(idx) = free_teams.next() else {
                break;
            };
            emit_repair_ai_command(
                entity_uuid,
                crate::messages::SystemControlPayload::DispatchRepairTeam {
                    team_idx: idx as u8,
                    target: crate::messages::RepairTarget::Station(crate::messages::StationId(
                        entry.station_id.clone(),
                    )),
                },
                sources,
                &sessions,
                config,
                &mut admitted,
            );
            newly_assigned.insert(entry.station_id.clone());
        }
    }
}

/// Emit an admitted Repair AI command targeting the repair system through the
/// shared [`crate::command_admission::validate_and_admit`] seam, using this
/// ship's own `ai:<uuid>` token (mirrors `emit_sensors_ai_command`).
fn emit_repair_ai_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    payload: crate::messages::SystemControlPayload,
    sources: &ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship_plugin::ShipConfigComponent,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    let token = entity_uuid
        .map(|u| format!("ai:{}", u.0))
        .unwrap_or_else(|| "ai:backfill".to_string());
    crate::command_admission::validate_and_admit(
        &token,
        crate::system_registry::repair_system_id(),
        payload,
        sources,
        sessions,
        &config.0,
        admitted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::SystemHull;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::shield::ShieldSystem;
    use crate::ship_plugin::ShipSystemControlSources;
    use crate::simulation::SimOutbox;
    use crate::simulation::{ShipImpulse, ShipShields};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .add_plugins(RepairPlugin)
        .add_plugins(repair_state_broadcaster())
        .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_dispatch_repair_team can query it.
        let hull_config = &[
            (SystemId("helm".into()), 25.0_f32),
            (SystemId("helm-engine-port".into()), 25.0),
            (SystemId("tactical".into()), 25.0),
            (SystemId("power".into()), 25.0),
            (SystemId("shields".into()), 25.0),
            (SystemId("core".into()), 50.0),
        ];
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(hull_config)),
            crate::server_app::ShipSystemBlackboards::default(),
            ShipShields(ShieldSystem::default(), 0.5),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            crate::modifiers::ShipModifiers::new(),
            RepairRequestQueue::default(),
            // Nested tuple to keep the outer bundle within Bevy's 15-arity limit.
            // Issue #830: the global `ShipRepairTeams` Resource is gone; every
            // ship (including this test's LocalShip) carries its own component.
            (
                crate::ship_plugin::RepairHumanAlerted::default(),
                crate::ship_plugin::LastSystemTiers::default(),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(2)),
            ),
        ));
        app
    }

    /// Read the LocalShip's own `ShipRepairTeams` component (issue #830 — no
    /// global Resource). Returns an owned clone for assertion convenience.
    fn local_teams(app: &mut App) -> ShipRepairTeams {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipRepairTeams, With<crate::simulation::LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipRepairTeams")
            .clone()
    }

    /// Dispatch a team on the LocalShip's own `ShipRepairTeams` component.
    fn dispatch_local(app: &mut App, idx: usize, sid: SystemId, name: &str) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipRepairTeams, With<crate::simulation::LocalShip>>();
        q.single_mut(app.world_mut())
            .expect("LocalShip must carry ShipRepairTeams")
            .0
            .dispatch(idx, sid, name.to_string());
    }

    fn repair_bb(app: &mut App) -> RepairBlackboard {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::server_app::ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q
            .single(app.world())
            .expect("LocalShip must have ShipSystemBlackboards");
        let key = SystemId(REPAIR_SYSTEM_ID.to_string());
        let SystemBlackboard::Repair(bb) = bbs.0.get(&key).unwrap() else {
            panic!("expected Repair blackboard");
        };
        bb.clone()
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage {
                target,
                msg,
                delivery: crate::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::Identify {
                token: "eng".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::SelectStation {
                station: "Repair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "eng", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(
            teams.0.slots()[idx],
            crate::messages::TeamSlot::Travelling { .. }
        )
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
    }

    // ── Dispatch tests ──────────────────────────────────────────────────────

    /// Non-Repair console holder sending `DispatchRepairTeam` is ignored.
    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle after non-Repair dispatch"
        );
    }

    /// Repair holder dispatches team to a console → team enters Travelling.
    #[test]
    fn dispatch_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling after dispatch"
        );
    }

    /// A station dispatch must resolve to an owned fine hull system so a team
    /// can finish travelling and restore HP instead of immediately returning.
    #[test]
    fn station_dispatch_repairs_damaged_owned_fine_system() {
        let mut app = test_app();
        start_game(&mut app);

        let local_ship = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, With<crate::simulation::LocalShip>>();
            query
                .single(app.world())
                .expect("test fixture must contain one LocalShip")
        };
        app.world_mut()
            .entity_mut(local_ship)
            .insert(ShipRepairTeams(crate::repair_teams::RepairTeams::default()));

        let damaged_system = SystemId("helm-engine-port".into());
        let hp_before = 10.0;
        {
            let mut query = app.world_mut().query_filtered::<
                &mut crate::entity_spawner::EntitySystemHull,
                With<crate::simulation::LocalShip>,
            >();
            let mut hull = query
                .single_mut(app.world_mut())
                .expect("test fixture must contain one LocalShip hull");
            hull.0.set_hp(&damaged_system, hp_before);
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId(REPAIR_SYSTEM_ID.into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        {
            let teams = local_teams(&mut app);
            let TeamSlot::Travelling { system_id, .. } = &teams.0.slots()[0] else {
                panic!("team 0 should be travelling to the damaged fine system");
            };
            assert_eq!(system_id.as_ref(), Some(&damaged_system));
        }

        // Default travel time is five seconds and the test clock advances 0.2s
        // per update. Run long enough to arrive and perform at least one repair.
        for _ in 0..30 {
            tick(&mut app);
        }

        let mut query = app.world_mut().query_filtered::<
            &crate::entity_spawner::EntitySystemHull,
            With<crate::simulation::LocalShip>,
        >();
        let hull = query
            .single(app.world())
            .expect("test fixture must contain one LocalShip hull");
        assert!(
            hull.0.current_for(&damaged_system).unwrap() > hp_before,
            "the arrived team should restore the station-owned fine system"
        );
    }

    /// When team is busy, dispatching to a different console redirects it.
    #[test]
    fn all_busy_teams_ignore_further_dispatches() {
        let mut app = test_app();
        start_game(&mut app);

        // Dispatch both teams (default is 2).
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 1,
                    target: RepairTarget::Station(StationId("tactical".into())),
                },
            },
        );
        tick(&mut app);

        // Redirect team 0 to Power (different console) — now team 0 is Returning with queue
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("power".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        // team 0 should be Returning (redirected), team 1 still Travelling
        assert!(matches!(
            &teams.0.slots()[0],
            crate::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(&teams, 1));
    }

    /// RepairState broadcast includes the team slot states.
    #[test]
    fn repair_state_broadcast_includes_team_slots() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        let out1 = tick(&mut app);
        let out2 = tick(&mut app);

        let has_repair_state = out1.iter().chain(out2.iter()).any(|m| {
            matches!(&m.msg, ServerMessage::RepairState { teams } if
                teams.iter().any(|t| matches!(t, crate::messages::TeamSlot::Travelling { .. })))
        });
        assert!(
            has_repair_state,
            "RepairState should include a Travelling team after dispatch"
        );
    }

    // ── ControlSystem dispatch tests ─────────────────────────────────────────

    /// Repair holder dispatches via `ControlSystem` → team enters Travelling.
    #[test]
    fn control_system_dispatch_authorized_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling after ControlSystem dispatch"
        );
    }

    /// Non-Repair console holder sending `ControlSystem` dispatch is rejected.
    #[test]
    fn control_system_dispatch_unauthorized_sender_is_rejected() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle when non-Repair sender uses ControlSystem"
        );
    }

    /// `ControlSystem` dispatch is blocked when the repair system is AI-controlled.
    #[test]
    fn control_system_dispatch_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game(&mut app);

        // Set repair system to AI control.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::repair_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle when repair system is AI-controlled"
        );
    }

    #[test]
    fn control_system_dispatch_repair_target_core_dispatches_team() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Core,
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling to Core after RepairTarget::Core dispatch"
        );
    }

    /// End-to-end TOML-driven wiring check: build the runtime `RepairTeams`
    /// the same way `spawn_game_start_entities` does (parse alliance_battleship.toml
    /// → RepairConfig::to_runtime → RepairTeams::new_with_timings) and
    /// assert the timings match the TOML. Changing
    /// `travel_duration_secs = 5.0` to e.g. `99.0` in alliance_battleship.toml
    /// would fail this test.
    #[test]
    fn repair_teams_resource_reflects_battleship_toml_repair_block() {
        let toml_str = include_str!("../../../assets/entities/alliance_battleship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("alliance_battleship.toml must parse");
        let rc = config
            .repair
            .expect("alliance_battleship must declare [repair]");
        let timings = rc.to_runtime();
        let teams = crate::repair_teams::RepairTeams::new_with_timings(2, timings);
        assert_eq!(teams.timings().travel_duration, rc.travel_duration_secs);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            rc.repair_rate_hp_per_sec
        );
        // And the runtime defaults still match (until someone intentionally
        // diverges them).
        let baseline = crate::repair_teams::RepairTimings::default();
        assert_eq!(teams.timings().travel_duration, baseline.travel_duration);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            baseline.repair_rate_hp_per_sec
        );
    }

    // ── Blackboard publish tests ─────────────────────────────────────────────

    #[test]
    fn publish_repair_blackboard_contains_teams_and_hull() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = repair_bb(&mut app);
        assert!(!bb.teams.is_empty(), "expected at least one team slot");
        assert!(!bb.system_hull.is_empty(), "expected system_hull entries");
        assert!(
            bb.travel_duration_secs > 0.0,
            "expected positive travel duration"
        );
    }

    #[test]
    fn publish_repair_blackboard_reflects_dispatch() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        dispatch_local(&mut app, 0, SystemId("helm".into()), "Helm");
        tick(&mut app);

        let bb = repair_bb(&mut app);
        assert!(
            bb.teams
                .iter()
                .any(|t| matches!(t, TeamSlot::Travelling { .. })),
            "expected a Travelling team slot after dispatch"
        );
    }

    #[test]
    fn publish_repair_blackboard_contains_damageable_systems() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = repair_bb(&mut app);
        assert!(
            !bb.damageable_systems.is_empty(),
            "expected damageable_systems"
        );
        assert!(
            bb.damageable_systems.contains(&SystemId("helm".into())),
            "Helm should appear in damageable_systems"
        );
        assert!(
            bb.damageable_systems.contains(&SystemId("core".into())),
            "Core should appear in damageable_systems"
        );
    }

    /// A queue entry whose station's only system transitions Disabled→Destroyed
    /// must be evicted by the retain predicate (zombie-entry regression).
    #[test]
    fn queue_entry_evicted_when_all_systems_destroyed() {
        use crate::damage::SystemHull;
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};

        let station_id = "helm";
        let system_id = SystemId("helm".into());

        let config = ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: system_id.clone(),
                kind: "helm".into(),
                station: Some(StationId(station_id.into())),
                ai_only: false,
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        };

        let mut hull = SystemHull::from_config(&[(system_id.clone(), 25.0_f32)]);

        let mut rq = RepairRequestQueue { entries: vec![] };
        rq.entries.push(RepairQueueEntry {
            station_id: station_id.into(),
            station_label: "Helm".into(),
            tier: crate::damage::DamageTier::Disabled,
            deficit: 25.0,
        });
        assert_eq!(rq.entries.len(), 1, "entry must be present before retain");

        hull.set_hp(&system_id, 0.0);
        assert_eq!(
            hull.tier_for(&system_id),
            crate::damage::DamageTier::Destroyed,
            "system must be Destroyed after set_hp(0)"
        );

        rq.entries.retain(|entry| {
            config
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| {
                    let t = hull.tier_for(&s.id);
                    t != crate::damage::DamageTier::Operational
                        && t != crate::damage::DamageTier::Destroyed
                })
        });

        assert!(
            rq.entries.is_empty(),
            "queue entry must be evicted when all station systems are Destroyed"
        );
    }

    /// Verifies that operate_repair_ai loops over all entities with
    /// ShipSystemControlSources, gating on operate_ai (issue #590 AC).
    #[test]
    fn operate_repair_ai_runs_per_entity_for_ai_controlled_ships() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::system_registry::repair_system_id());
        assert!(policy.operate_ai, "AI Repair must gate through operate_ai");

        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::repair_system_id());
        assert!(!human_policy.operate_ai, "human Repair must not operate AI");
    }

    // ── NPC AI repair through admission (issue #830) ─────────────────────────

    /// A minimal ship config whose `helm` station owns a single `helm` fine
    /// system, so `resolve_repair_target(Station("helm"))` resolves to it.
    fn npc_repair_config() -> crate::ship_plugin::ShipConfigComponent {
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};
        crate::ship_plugin::ShipConfigComponent(ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: SystemId("helm".into()),
                kind: "helm".into(),
                station: Some(StationId("helm".into())),
                ai_only: false,
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        })
    }

    /// Build an app that runs the full per-entity admitted repair pipeline —
    /// `operate_repair_ai` (emit) → `handle_dispatch_repair_team` (apply) →
    /// `tick_repair_teams` — chained so the same-tick emit→apply→repair shape of
    /// production holds. `Sessions` is present because `validate_and_admit`
    /// consults it (the `ai:` path only needs the resource to exist).
    fn npc_repair_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(1000),
        ));
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ));
        // Stand in for `admit_system_commands`, which clears `AdmittedCommands`
        // once per tick before the AI decide systems refill it. Without this the
        // AI's `DispatchRepairTeam` would re-apply every tick and recall the team
        // (Travelling → Returning) forever, so it would never reach Repairing.
        app.add_systems(
            Update,
            (
                clear_admitted_commands,
                operate_repair_ai,
                crate::console::repair::dispatch::handle_dispatch_repair_team,
                tick_repair_teams,
            )
                .chain(),
        );
        app
    }

    /// Test-only mirror of admission's per-tick `AdmittedCommands` clear.
    fn clear_admitted_commands(mut q: Query<&mut crate::messages::AdmittedCommands>) {
        for mut admitted in q.iter_mut() {
            admitted.0.clear();
        }
    }

    /// Spawn an NPC ship (Ship marker, no LocalShip) whose Repair system is
    /// under the given control source, with a `helm` hull damaged by `damage`,
    /// a queue entry naming the `helm` station, an `EntityUuid` for its `ai:`
    /// token, and an empty `AdmittedCommands`.
    fn spawn_npc_repair(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
        damage: f32,
    ) -> Entity {
        use crate::ship::control_source::ControlSourceResolver;
        let mut resolver = ControlSourceResolver::new();
        resolver.set(repair_system_id(), source);

        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("helm".into()), 100.0_f32)]);
        let mut rng = rand::rng();
        hull.apply_damage(damage, &mut rng);

        let mut queue = RepairRequestQueue::default();
        queue.push_or_merge(RepairQueueEntry {
            station_id: "helm".into(),
            station_label: "Helm".into(),
            tier: DamageTier::Disabled,
            deficit: damage,
        });

        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entity_spawner::EntityUuid("npc-repair-1".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(2)),
                crate::entity_spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                npc_repair_config(),
                crate::messages::AdmittedCommands::default(),
            ))
            .id()
    }

    /// The NPC applier consumes the AI operator's admitted `DispatchRepairTeam`
    /// in the same tick and sends a team travelling — proving the per-entity
    /// emit→admit→apply chain runs on an NPC ship with no LocalShip marker.
    #[test]
    fn npc_applier_consumes_ai_emitted_dispatch_same_tick() {
        let mut app = npc_repair_app();
        let npc = spawn_npc_repair(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            80.0,
        );

        // One warm-up tick (TimePlugin baseline). The AI emits into the NPC's
        // own AdmittedCommands and the applier dispatches on the same tick.
        app.update();

        let teams = app
            .world()
            .get::<ShipRepairTeams>(npc)
            .expect("NPC must have ShipRepairTeams");
        assert!(
            teams
                .0
                .slots()
                .iter()
                .any(|s| matches!(s, TeamSlot::Travelling { .. })),
            "the NPC applier must have dispatched a team from the AI's own \
             AdmittedCommands, got {:?}",
            teams.0.slots()
        );
    }

    /// Regression for PRD #597 gap-5 (retained through #830): an NPC ship's
    /// AI-driven repair restores its own hull over time — now flowing through
    /// admission (`operate_repair_ai` emits, `handle_dispatch_repair_team`
    /// applies, `tick_repair_teams` heals) rather than a direct team write.
    #[test]
    fn npc_ship_with_repair_teams_regenerates_hull_over_time() {
        let mut app = npc_repair_app();
        let npc = spawn_npc_repair(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            80.0,
        );
        let hp_before = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .total_current();

        // 200 iterations comfortably covers the 5 s travel + repair time.
        for _ in 0..200 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
            .expect("NPC must still have hull component")
            .0
            .total_current();
        assert!(
            hp_after > hp_before,
            "NPC hull HP must increase after AI-admitted dispatch + repair \
             (before={hp_before}, after={hp_after})"
        );
    }

    /// A human-held Repair system rejects an `ai:` emission at the admission
    /// gate: `validate_and_admit` returns false and nothing is admitted. This is
    /// the symmetry contract — the AI operator gates on `operate_ai` before
    /// emitting, and admission independently enforces it.
    #[test]
    fn human_held_repair_rejects_ai_emission() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        let mut resolver = ControlSourceResolver::new();
        resolver.set(repair_system_id(), ControlSource::Human);
        let sources = ShipSystemControlSources(resolver);
        let sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
        let config = npc_repair_config();
        let mut admitted = crate::messages::AdmittedCommands::default();

        let admitted_ok = crate::command_admission::validate_and_admit(
            "ai:npc-repair-1",
            repair_system_id(),
            SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
            &sources,
            &sessions,
            &config.0,
            &mut admitted,
        );
        assert!(
            !admitted_ok,
            "ai: emission must be rejected when repair is human-held"
        );
        assert!(
            admitted.0.is_empty(),
            "no command may be admitted for a human-held repair system"
        );
    }
}
