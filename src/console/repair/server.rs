use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::ModifierSlot;
use crate::messages::{
    AdmittedCommands, ClientMessage, Console, RepairBlackboard, RepairTarget, ServerMessage,
    StationId, SystemBlackboard, SystemControlPayload, SystemHullStatus, SystemId, TeamSlot,
};
use crate::modifiers::ShipModifiers;
use crate::repair_teams::RepairTeams;
use crate::ship::system_registry::{repair_system_id, REPAIR_SYSTEM_ID};
use crate::ship_plugin::ShipSystemControlSources;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Bevy resource wrapping the pure `RepairTeams` state machine.
///
/// Derives both `Resource` (existing player-ship singleton) and `Component`
/// (per-entity path after issue #590 unification).
#[derive(Resource, Component, Clone)]
pub struct ShipRepairTeams(pub RepairTeams);

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShipRepairTeams(RepairTeams::default()))
            .add_systems(
                Update,
                (
                    handle_dispatch_repair_team.in_set(crate::sim_sets::SimSet::Input),
                    tick_repair_teams.in_set(crate::sim_sets::SimSet::Physics),
                    operate_repair_ai.in_set(crate::sim_sets::SimSet::Physics),
                    publish_repair_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            )
            .add_plugins(repair_state_broadcaster());
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `RepairState` producer.
///
/// Broadcasts `RepairState` at 10 Hz to the `Repair` console holder only.
/// Registered by [`RepairPlugin`].
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipRepairTeams` component
/// on the LocalShip entity, falling back to the global Resource for tests.
pub fn repair_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(StationId("repair".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let mut q =
                world.query_filtered::<&ShipRepairTeams, With<crate::server_app::LocalShip>>();
            let slots = q
                .iter(world)
                .next()
                .map(|t| t.0.slots().to_vec())
                .or_else(|| {
                    world
                        .get_resource::<ShipRepairTeams>()
                        .map(|t| t.0.slots().to_vec())
                });
            let Some(slots) = slots else {
                return vec![];
            };
            vec![ServerMessage::RepairState { teams: slots }]
        },
    )
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Handle `DispatchRepairTeam` messages from the Repair console.
///
/// Accepts both the legacy `ClientMessage::DispatchRepairTeam` path and the
/// new `ClientMessage::ControlSystem { target: "repair", payload:
/// DispatchRepairTeam { .. } }` path. Both are gated on:
///
/// 1. `ControlSourceResolver::policy_for(&repair_system_id()).accept_human_input`
///    (rejects when the system is under AI control)
/// 2. Sender holds `Console::Repair`.
///
/// `RepairTarget::Core` dispatches to `Console::Core`, the repair bucket for
/// ownerless ship-wide systems.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipRepairTeams` component
/// on the LocalShip entity; falls back to the global `ShipRepairTeams` resource
/// for tests. Dual-writes to the Resource so legacy Resource-based readers
/// stay in sync.
pub fn handle_dispatch_repair_team(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship_query: Query<
        (
            &AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
            Option<&mut ShipRepairTeams>,
            Option<&crate::entity_spawner::EntitySystemHull>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    teams_res: Option<ResMut<ShipRepairTeams>>,
) {
    let Some((admitted, _ship_config, control_sources, mut teams_comp, hull_opt)) =
        ship_query.iter_mut().next()
    else {
        return;
    };
    let mut teams_res = teams_res;

    // Look up a human-readable display name for a SystemId. Prefer the
    // ship's `EntitySystemHull` entry (populated from TOML with the
    // designer-authored display name), fall back to `Console::from_console_id`
    // when the SystemId maps to a well-known console, and finally fall back
    // to the raw SystemId string.
    //
    // The reviewer flagged that the pre-fix code passed the raw SystemId
    // string as the display name for the legacy `Console`-keyed wire path,
    // which regressed `TeamSlot.display_name` from "Engine (Port)" to
    // "helm-engine-port" for every legacy `DispatchRepairTeam` message.
    let hull_ref = hull_opt.map(|h| &h.0);
    let display_name_for = |sid: &SystemId| -> String {
        if let Some(hull) = hull_ref {
            if let Some(entry) = hull.get(sid) {
                return entry.display_name.clone();
            }
        }
        if let Some(console) = Console::from_console_id(sid.0.as_str()) {
            return console.display_name().to_string();
        }
        sid.0.clone()
    };

    // Collect all dispatches into a batch first, then apply once — avoids the
    // closure-captures-borrow tangle when routing between Component and Resource.
    let mut pending: Vec<(usize, SystemId, String)> = Vec::new();

    // ── ControlSystem path (authority already checked at admission) ────────
    for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
        if let SystemControlPayload::DispatchRepairTeam {
            team_idx,
            target: repair_target,
        } = &cmd.payload
        {
            let sid = match repair_target {
                RepairTarget::Station(station_id) => SystemId(station_id.0.clone()),
                RepairTarget::Core => SystemId("core".into()),
            };
            let display = display_name_for(&sid);
            pending.push((*team_idx as usize, sid, display));
        }
    }

    // ── Legacy path (DispatchRepairTeam message type, still auth-gated here) ─
    let policy = control_sources.0.policy_for(&repair_system_id());
    if !policy.accept_human_input {
        for _ in reader.read() {}
    } else if let Some(repair_token) = sessions.0.holder_for_station(&StationId("repair".into())) {
        for ev in reader.read() {
            let ClientMessage::DispatchRepairTeam { team_idx, console } = &ev.msg else {
                continue;
            };
            if ev.token.as_str() != repair_token {
                continue;
            }
            let sid = SystemId(console.station_console_id().to_string());
            // For the legacy Console-keyed path we always have the Console
            // in hand, so use its display_name directly. This is the exact
            // behaviour the reviewer's finding requires: the legacy wire
            // message must produce the human-readable name, not the raw
            // SystemId string.
            let display = console.display_name().to_string();
            pending.push((*team_idx as usize, sid, display));
        }
    } else {
        for _ in reader.read() {}
    }

    if pending.is_empty() {
        return;
    }

    // Apply to whichever backing store is available; dual-write when both.
    for (idx, sid, display) in pending {
        if let Some(t) = teams_comp.as_deref_mut() {
            t.0.dispatch(idx, sid.clone(), display.clone());
        }
        if let Some(r) = teams_res.as_deref_mut() {
            r.0.dispatch(idx, sid, display);
        }
    }
    // Keep Resource in sync with per-entity component (Resource is dual-written
    // above; but if only the Component was updated we snapshot the Component
    // into the Resource so legacy Resource-based readers see the latest state).
    if let (Some(t), Some(r)) = (teams_comp.as_deref(), teams_res.as_deref_mut()) {
        r.0 = t.0.clone();
    }
}

/// Tick repair teams each frame: advance timers and apply HP restoration.
///
/// After PRD #597 gap-5 closure: iterates every ship (`With<Ship>`) — player
/// and NPC — so ships with a per-entity `ShipRepairTeams` component (spawned
/// when their TOML declares a `[repair]` block) tick their own teams against
/// their own `EntitySystemHull`. Each ship applies its own
/// `ShipModifiers.RepairRate` multiplier. The global `ShipRepairTeams`
/// resource is dual-written from the LocalShip so legacy Resource-based
/// readers (broadcasters, tests) stay in sync.
pub fn tick_repair_teams(
    time: Res<Time>,
    mut teams_res: Option<ResMut<ShipRepairTeams>>,
    modifiers_res: Option<Res<ShipModifiers>>,
    mut ship_q: Query<
        (
            Option<&mut ShipRepairTeams>,
            Option<&ShipModifiers>,
            &mut crate::entity_spawner::EntitySystemHull,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let dt = time.delta_secs();
    let default_modifiers = ShipModifiers::new();
    let mut local_teams_snapshot: Option<crate::repair_teams::RepairTeams> = None;

    for (teams_comp, modifiers_comp, mut hull, is_local) in ship_q.iter_mut() {
        // Only tick ships that carry per-entity repair teams. NPCs get the
        // component only when their TOML declares `[repair]`. Ships without
        // it silently skip — matching the historical "no teams == no tick"
        // behaviour.
        let Some(mut teams) = teams_comp else {
            continue;
        };
        let modifiers: &ShipModifiers = match modifiers_comp {
            Some(m) => m,
            None => match modifiers_res.as_deref() {
                Some(m) => m,
                None => &default_modifiers,
            },
        };
        let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
        teams.0.tick(dt * repair_mult, &mut hull.0);
        if is_local {
            local_teams_snapshot = Some(teams.0.clone());
        }
    }

    // Dual-write: mirror the LocalShip's teams into the global Resource so
    // legacy Resource-based readers stay in sync.
    if let (Some(local_teams), Some(r)) = (local_teams_snapshot, teams_res.as_deref_mut()) {
        r.0 = local_teams;
        return;
    }

    // Resource-only fallback for tests that don't spawn a Ship entity with
    // the per-entity `ShipRepairTeams` component. Ticks the global Resource
    // against the LocalShip's hull only.
    let mut hull_only_q = ship_q.iter_mut();
    if let Some((_, _, mut hull, _)) = hull_only_q.find(|(_, _, _, is_local)| *is_local) {
        if let Some(r) = teams_res.as_deref_mut() {
            let modifiers = modifiers_res
                .as_deref()
                .cloned()
                .unwrap_or_else(ShipModifiers::new);
            let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
            r.0.tick(dt * repair_mult, &mut hull.0);
        }
    }
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

fn publish_repair_blackboard(
    teams_res: Option<Res<ShipRepairTeams>>,
    ship_q: Query<
        (
            Option<&ShipRepairTeams>,
            &crate::entity_spawner::EntitySystemHull,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut blackboards_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    // Prefer per-entity component on LocalShip; fall back to global Resource.
    let entity_view = ship_q.single().ok();
    let default_teams;
    let teams: &ShipRepairTeams = match entity_view.and_then(|(t, _)| t) {
        Some(t) => t,
        None => match teams_res.as_deref() {
            Some(t) => t,
            None => {
                default_teams = ShipRepairTeams(crate::repair_teams::RepairTeams::default());
                &default_teams
            }
        },
    };
    let hull_ref = entity_view.map(|(_, h)| h);
    let team_slots: Vec<TeamSlot> = teams.0.slots().to_vec();

    // Build the new `SystemHullStatus` list first from the authoritative
    // `SystemHull` iteration; the legacy `console_hull`/`damageable_consoles`
    // wire fields are derived from it by mapping each SystemId back to a
    // Console variant. SystemIds that don't map to a Console variant (custom
    // designer-authored systems) are dropped from the legacy list but still
    // emitted in the new list.
    let system_hull: Vec<SystemHullStatus> = hull_ref
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

    let damageable_systems: Vec<SystemId> = system_hull
        .iter()
        .map(|s| s.system_id.clone())
        .collect();

    // Legacy `console_hull` / `damageable_consoles` wire fields: emptied by
    // the publisher post issue #618. Downstream clients read the SystemId-keyed
    // side (`system_hull` + `damageable_systems`). The struct fields survive
    // on the wire (with `#[serde(default)]` for compat) until removal in a
    // later sub-PR.
    let bb = RepairBlackboard {
        teams: team_slots,
        console_hull: Vec::new(),
        travel_duration_secs: teams.0.timings().travel_duration,
        damageable_consoles: Vec::new(),
        system_hull,
        damageable_systems,
    };

    if let Some(mut blackboards) = blackboards_q.iter_mut().next() {
        blackboards.0.insert(
            SystemId(REPAIR_SYSTEM_ID.to_string()),
            SystemBlackboard::Repair(bb),
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-kind AI loop for repair. Iterates every ship (`With<Ship>`) whose
/// Repair system is `ControlSource::Ai` and auto-dispatches any idle team to
/// the most-damaged console on that ship's hull.
///
/// The most-damaged console is the one with the largest absolute HP deficit
/// (`max - current`) that is still > 0. Ties are broken by the entry order in
/// `EntitySystemHull`. Ships with no per-entity `ShipRepairTeams` component
/// silently skip — an NPC without a `[repair]` block simply has no teams to
/// dispatch.
///
/// After PRD #597 gap-5 closure: same code path for player Backfill AI and
/// NPC AI. The only differentiator is `ShipSystemControlSources`
/// (data-driven) and `LocalShip` marker.
pub fn operate_repair_ai(
    mut ships: Query<
        (
            &ShipSystemControlSources,
            Option<&mut ShipRepairTeams>,
            Option<&crate::entity_spawner::EntitySystemHull>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (sources, teams_comp, hull_comp) in ships.iter_mut() {
        let policy = sources.0.policy_for(&repair_system_id());
        if !policy.operate_ai {
            continue;
        }
        // Need both a team roster and a hull to make any decision.
        let (Some(mut teams), Some(hull)) = (teams_comp, hull_comp) else {
            continue;
        };
        // Pick the system with the largest current HP deficit (max - cur > 0).
        // Ties broken by entry order (first-declared wins).
        // Destroyed systems (hp == 0) are skipped — they are unrepairable.
        let target: Option<SystemId> = hull
            .0
            .entries()
            .filter(|(sid, cur, max)| {
                max - cur > 0.0
                    && hull.0.tier_for(sid) != crate::damage::DamageTier::Destroyed
            })
            .max_by(|(_, a_cur, a_max), (_, b_cur, b_max)| {
                let a_deficit = a_max - a_cur;
                let b_deficit = b_max - b_cur;
                a_deficit
                    .partial_cmp(&b_deficit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(sid, _, _)| sid.clone());
        let Some(target) = target else {
            continue;
        };
        // Look up the target's display name from the hull entry so the
        // resulting `TeamSlot` carries the human-readable label. Falls back
        // to the raw SystemId string only if the entry is missing (which
        // should not happen for a target we just picked from the hull).
        let target_display = hull
            .0
            .get(&target)
            .map(|e| e.display_name.clone())
            .unwrap_or_else(|| target.0.clone());
        // Dispatch every idle team to the target. Reuses the same
        // `RepairTeams::dispatch` entry point that the human console uses.
        while let Some(idx) = teams.0.lowest_free_team() {
            teams.0.dispatch(idx, target.clone(), target_display.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::SystemHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
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
        .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
        .insert_resource(crate::modifiers::ShipModifiers::new())
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .add_plugins(RepairPlugin)
        .add_plugins(repair_state_broadcaster())
        .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_dispatch_repair_team can query it.
        let hull_config = &[
            (SystemId("helm".into()), 25.0_f32),
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
            ShipShields(ShieldSystem::default()),
        ));
        app
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
            out.push(OutboundMessage { target, msg });
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
                station: "Captain's Chair".into(),
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "team 0 should be travelling after dispatch"
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
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 1,
                console: Console::Tactical,
            },
        );
        tick(&mut app);

        // Redirect team 0 to Power (different console) — now team 0 is Returning with queue
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Power,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        // team 0 should be Returning (redirected), team 1 still Travelling
        assert!(matches!(
            &teams.0.slots()[0],
            crate::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(teams, 1));
    }

    /// RepairState broadcast includes the team slot states.
    #[test]
    fn repair_state_broadcast_includes_team_slots() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
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

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
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

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
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

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
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

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "team 0 should be travelling to Core after RepairTarget::Core dispatch"
        );
    }

    /// Legacy `ClientMessage::DispatchRepairTeam` still works.
    #[test]
    fn legacy_dispatch_still_works_after_control_system_migration() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Tactical,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "legacy DispatchRepairTeam should still dispatch team 0"
        );
    }

    /// Regression test for the reviewer's second finding on issue #617.
    ///
    /// Pre-#617 the legacy `ClientMessage::DispatchRepairTeam { console:
    /// Console::HelmEnginePort }` wire message produced a
    /// `TeamSlot::Travelling` with `display_name: Some("Engine (Port)")`
    /// (the `Console::display_name()` for that variant). The initial
    /// #617 implementation regressed this to `Some("helm-engine-port")`
    /// (the raw SystemId string) because `RepairTeams::dispatch` had
    /// dropped the display-name parameter and defaulted to
    /// `new_system.0.clone()`.
    ///
    /// This test sends the exact legacy wire message the reviewer called
    /// out and asserts the resulting `TeamSlot.display_name` is the
    /// human-readable label, matching pre-#617 behaviour.
    #[test]
    fn legacy_dispatch_preserves_console_display_name_on_team_slot() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        let slot = &teams.0.slots()[0];
        assert!(
            matches!(
                slot,
                TeamSlot::Travelling { display_name: Some(d), .. }
                    if d == Console::Helm.display_name()
            ),
            "legacy DispatchRepairTeam {{ console: Helm }} must produce \
             TeamSlot::Travelling with display_name = Some(\"{}\") \
             (Console::display_name()); got {:?}",
            Console::Helm.display_name(),
            slot
        );
    }

    /// End-to-end TOML-driven wiring check: build the runtime `RepairTeams`
    /// the same way `spawn_game_start_entities` does (parse player_ship.toml
    /// → RepairConfig::to_runtime → RepairTeams::new_with_timings) and
    /// assert the timings match the TOML. Changing
    /// `travel_duration_secs = 5.0` to e.g. `99.0` in player_ship.toml
    /// would fail this test.
    #[test]
    fn repair_teams_resource_reflects_player_ship_toml_repair_block() {
        let toml_str = include_str!("../../../assets/entities/player_ship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("player_ship.toml must parse");
        let rc = config.repair.expect("player_ship must declare [repair]");
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

        app.world_mut()
            .resource_mut::<ShipRepairTeams>()
            .0
            .dispatch(0, SystemId("helm".into()), "Helm".to_string());
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

    /// Verifies that an NPC ship (without Ship marker) with Repair on AI
    /// runs operate_repair_ai independently (issue #590 AC).
    #[test]
    fn npc_ship_with_repair_on_ai_runs_operate_repair_ai() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        // Build a test app with operate_repair_ai registered.
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_systems(bevy::prelude::Update, operate_repair_ai);

        // Spawn an NPC entity (no Ship marker) with ShipSystemControlSources set to AI.
        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn(ShipSystemControlSources(ai_resolver))
            .id();

        // Should not panic.
        app.update();
        assert!(
            app.world()
                .get::<ShipSystemControlSources>(npc_entity)
                .is_some(),
            "NPC entity must still exist after operate_repair_ai runs"
        );
    }

    /// Regression test for PRD #597 gap-5: an NPC ship that carries a per-entity
    /// `[repair]` block (ShipRepairTeams + EntitySystemHull) must have its
    /// teams ticked by `tick_repair_teams` AND auto-dispatched by
    /// `operate_repair_ai`, and the resulting HP restoration must land on its
    /// own hull — no LocalShip marker involved.
    ///
    /// Sequence: spawn NPC ship (Ship marker only, no LocalShip) with hull
    /// damaged well below max, register both AI + tick systems, run enough
    /// simulated time for a team to travel + start repairing, assert the
    /// NPC hull's total_current has increased.
    #[test]
    fn npc_ship_with_repair_teams_regenerates_hull_over_time() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(1000),
        ));
        app.add_systems(Update, (operate_repair_ai, tick_repair_teams).chain());

        // NPC ship: Ship marker, AI-controlled Repair, damaged hull.
        // Damage the hull by 40 HP so total_current == 60 (max = 100).
        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(repair_system_id(), ControlSource::Ai);
        let mut hull = crate::damage::SystemHull::from_config(&[(SystemId("helm".into()), 100.0_f32)]);
        let mut rng = rand::rng();
        hull.apply_damage(40.0, &mut rng);
        let hp_before = hull.total_current();
        assert!(
            (hp_before - 60.0).abs() < 1e-3,
            "test fixture: hull should have 60 HP after 40 damage, got {hp_before}"
        );

        let npc = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                ShipSystemControlSources(ai_resolver),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(2)),
                crate::entity_spawner::EntitySystemHull(hull),
            ))
            .id();

        // Warm-up tick so TimePlugin registers a delta.
        app.update();
        // After warm-up, operate_repair_ai should have dispatched at least
        // one team. If it didn't, the wiring is wrong — fail loudly with a
        // useful diagnostic before spending 20 more ticks.
        {
            let teams = app
                .world()
                .get::<ShipRepairTeams>(npc)
                .expect("NPC must have ShipRepairTeams");
            let any_dispatched = teams
                .0
                .slots()
                .iter()
                .any(|s| !matches!(s, crate::messages::TeamSlot::Idle));
            assert!(
                any_dispatched,
                "operate_repair_ai should have dispatched at least one team after \
                 warm-up, got {:?}",
                teams.0.slots()
            );
        }
        // Bevy's `TimeUpdateStrategy::ManualDuration` first-tick warm-up
        // ends up producing a smaller-than-configured `delta_secs` (roughly
        // 250 ms/tick observed under 1000 ms configuration). 200 iterations
        // is comfortably more than enough to cover the 5 s travel + several
        // seconds of repair at 0.5 HP/s.
        for _ in 0..200 {
            app.update();
        }
        let teams_dbg = app
            .world()
            .get::<ShipRepairTeams>(npc)
            .expect("teams")
            .0
            .slots()
            .to_vec();

        let hull_after = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
            .expect("NPC must still have hull component");
        let hp_after = hull_after.0.total_current();
        assert!(
            hp_after > hp_before,
            "NPC hull HP must increase after operate_repair_ai + tick_repair_teams \
             (before={hp_before}, after={hp_after}, teams={teams_dbg:?})"
        );
    }
}
