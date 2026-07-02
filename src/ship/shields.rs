use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{
    AdmittedCommands, Console, CoordinationPayload, ShieldFacingStatus, ShieldsBlackboard,
    SystemBlackboard, SystemControlPayload, SystemId, ViewDirection,
};
use crate::ship_plugin::CoordinationEnqueue;


// ── Components ─────────────────────────────────────────────────────────────────

/// The ship's shield system.
///
/// Per-ship shield system — a `ShieldSystem` wrapped in a Component.
///
/// Pure per-ship Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<ShipShields>`.
#[derive(Component)]
pub struct ShipShields(pub crate::shield::ShieldSystem);

// ── Resources ──────────────────────────────────────────────────────────────────

/// TOML-loaded configuration for the shields AI controller.
///
/// Loaded from `[shields.ai]` in the ship entity TOML. Defaults are used
/// when the section is absent.
///
/// Dual `Resource + Component` post ship-parity audit: production reads
/// use the Resource form (single ship-wide AI tuning), but the Component
/// derive is available if NPC ships ever need per-ship AI tuning.
#[derive(Resource, Component, Clone, Debug)]
pub struct ShieldsAiConfigResource {
    /// HP fraction (0.0–1.0) at or above which a restored facing fires the
    /// `ShieldFacingRestored` coordination message to Helm.
    pub restored_notify_pct: f32,
}

impl Default for ShieldsAiConfigResource {
    fn default() -> Self {
        Self {
            restored_notify_pct: 0.5,
        }
    }
}

/// Per-facing notification state for the shields coordination emitter.
///
/// Indexed by facing index (usize). Both flags reset when a facing comes back
/// online so the down/restore cycle repeats on the next offline event.
///
/// Per-ship Component so NPC ships' shields can emit their own advisories
/// through their own `CoordinationQueue` without stepping on the player's
/// shield-notification state.
#[derive(Component, Default, Clone)]
pub struct ShieldsCoordinationState {
    pub down_notified: Vec<bool>,
    pub restore_notified: Vec<bool>,
}

impl ShieldsCoordinationState {
    fn ensure_len(&mut self, n: usize) {
        if self.down_notified.len() < n {
            self.down_notified.resize(n, false);
            self.restore_notified.resize(n, false);
        }
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipShieldsPlugin;

impl Plugin for ShipShieldsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CoordinationEnqueue>()
            .init_resource::<ShieldsAiConfigResource>()
            .add_systems(
                Update,
                (
                    handle_shields_messages.in_set(crate::sim_sets::SimSet::Input),
                    emit_shields_coordination.in_set(crate::sim_sets::SimSet::Input),
                    operate_shields_ai.in_set(crate::sim_sets::SimSet::Physics),
                    tick_shields.in_set(crate::sim_sets::SimSet::Modifiers),
                    publish_shields_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            )
            .add_plugins(shields_state_broadcaster());
    }
}

/// Tick shield regen and offline timers each frame for every ship
/// (player + NPCs). PR-7 (issue #597) unifies this with the old
/// `tick_npc_shield_regen` — one system iterating all ships with `Ship` marker.
pub fn tick_shields(
    time: Res<Time>,
    mut shields_q: Query<&mut ShipShields, With<crate::server_app::Ship>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for mut shield in shields_q.iter_mut() {
        shield.0.tick(dt);
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

pub fn shields_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(Console::Shields),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let Ok(shields) = world
                .query_filtered::<&ShipShields, With<crate::server_app::LocalShip>>()
                .single(world)
            else {
                return vec![];
            };
            let facings: Vec<ShieldFacingStatus> = shields
                .0
                .snapshot()
                .into_iter()
                .map(|s| ShieldFacingStatus {
                    label: s.label,
                    hp: s.hp,
                    max_hp: s.max_hp,
                    online: s.online,
                    offline_remaining: s.offline_remaining,
                    is_focused: s.is_focused,
                })
                .collect();
            vec![crate::messages::ServerMessage::ShieldStatus { facings }]
        },
    )
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetShieldFocus` messages from every ship's Shields console.
///
/// Iterates every ship (player + NPC) so both the player's Shields console
/// commands and the future NPC `operate_shields_ai` writes into
/// `AdmittedCommands` flip each ship's own shield focus.
pub fn handle_shields_messages(
    mut ship_query: Query<
        (&AdmittedCommands, &mut ShipShields),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, mut shields) in ship_query.iter_mut() {
        for cmd in admitted.for_target(crate::system_registry::SHIELDS_SYSTEM_ID) {
            let SystemControlPayload::SetShieldFocus { facing } = &cmd.payload else {
                continue;
            };
            let idx = facing.as_ref().map(|d| match d {
                ViewDirection::Fore => 0,
                ViewDirection::Port => 1,
                ViewDirection::Aft => 2,
                ViewDirection::Starboard => 3,
            });
            shields.0.set_focused_facing(idx);
        }
    }
}

/// Emit `ShieldFacingDown` and `ShieldFacingRestored` coordination messages
/// per-ship via the centralized `CoordinationEnqueue` channel (channel 3).
///
/// Iterates every ship (player + NPC). Each `CoordinationEnqueue` stamps
/// its source ship so `handle_coordination_enqueue` routes it into the
/// correct ship's `CoordinationQueue` component.
pub fn emit_shields_coordination(
    mut ship_q: Query<
        (
            Entity,
            &ShipShields,
            &crate::ship_state::ShipRedAlert,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut ShieldsCoordinationState,
        ),
        With<crate::server_app::Ship>,
    >,
    ai_config: Res<ShieldsAiConfigResource>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, shields, red_alert, control_sources, mut coord_state) in ship_q.iter_mut() {
        let snapshots = shields.0.snapshot();
        coord_state.ensure_len(snapshots.len());

        let red_alert = red_alert.0;
        let sender_origin = control_sources
            .0
            .source_for(&crate::system_registry::shields_system_id());

        for (i, snap) in snapshots.iter().enumerate() {
            if !snap.online {
                if !coord_state.down_notified[i] {
                    coord_state.down_notified[i] = true;
                    coord_state.restore_notified[i] = false;

                    let payload = CoordinationPayload::ShieldFacingDown {
                        label: snap.label.clone(),
                        offline_remaining: snap.offline_remaining,
                    };
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        target: crate::system_registry::helm_system_id(),
                        payload,
                        sender_label: "Shields".to_string(),
                    });
                }
            } else {
                // Facing is online. Check for restore notification before clearing state.
                if coord_state.down_notified[i]
                    && !coord_state.restore_notified[i]
                    && red_alert
                    && snap.max_hp > 0
                    && (snap.hp as f32 / snap.max_hp as f32) >= ai_config.restored_notify_pct
                {
                    coord_state.restore_notified[i] = true;

                    let payload = CoordinationPayload::ShieldFacingRestored {
                        label: snap.label.clone(),
                    };
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        target: crate::system_registry::helm_system_id(),
                        payload,
                        sender_label: "Shields".to_string(),
                    });
                }

                // Reset cycle state when facing returns to full online status so
                // the next offline event starts fresh.
                if coord_state.restore_notified[i] || !coord_state.down_notified[i] {
                    // Already clean — nothing to reset.
                } else if snap.max_hp > 0
                    && (snap.hp as f32 / snap.max_hp as f32) >= ai_config.restored_notify_pct
                    && !red_alert
                {
                    // Facing recovered but not on red alert; clear so next cycle works.
                    coord_state.down_notified[i] = false;
                    coord_state.restore_notified[i] = false;
                }
            }
        }
    }
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

fn publish_shields_blackboard(
    shields_q: Query<&ShipShields, With<crate::server_app::LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntityConsoleHull, With<crate::server_app::LocalShip>>,
    physics_q: Query<&crate::ship_state::ShipPhysics, With<crate::simulation::LocalShip>>,
    weapons_target_q: Query<&crate::weapons_plugin::WeaponsTarget, With<crate::server_app::LocalShip>>,
    asteroid_q: Query<
        (&crate::simulation::AsteroidUuid, &Transform),
        Without<crate::entity_spawner::EntityUuid>,
    >,
    entity_q: Query<
        (&crate::entity_spawner::EntityUuid, &Transform),
        Without<crate::simulation::AsteroidUuid>,
    >,
    mut ship_bbs_q: Query<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::LocalShip>>,
) {
    let Some(shields) = shields_q.iter().next() else {
        return;
    };
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let facings: Vec<ShieldFacingStatus> = shields
        .0
        .snapshot()
        .into_iter()
        .map(|s| ShieldFacingStatus {
            label: s.label,
            hp: s.hp,
            max_hp: s.max_hp,
            online: s.online,
            offline_remaining: s.offline_remaining,
            is_focused: s.is_focused,
        })
        .collect();

    let (total_hp, total_current) = hull_q
        .single()
        .map(|h| (h.0.total_max(), h.0.total_current()))
        .unwrap_or((100.0, 100.0));
    let hull_integrity_pct = if total_hp > 0.0 {
        ((total_current / total_hp) * 100.0).clamp(0.0, 100.0)
    } else {
        100.0
    };

    let focused_facing = facings
        .iter()
        .find(|f| f.is_focused)
        .map(|f| f.label.clone());

    let any_offline = facings.iter().any(|f| !f.online);
    let grid_status = if any_offline {
        "EMITTER OFFLINE"
    } else {
        "GRID NOMINAL"
    }
    .to_string();

    let target_bearing = weapons_target_q.single().ok().and_then(|wt| {
        let uuid = wt.0.as_ref()?;
        let live = asteroid_q
            .iter()
            .find(|(u, _)| u.0 == *uuid)
            .map(|(_, t)| (t.translation.x, t.translation.z))
            .or_else(|| {
                entity_q
                    .iter()
                    .find(|(u, _)| u.0 == *uuid)
                    .map(|(_, t)| (t.translation.x, t.translation.z))
            })?;
        let dx = live.0 - physics.x;
        let dz = live.1 - physics.z;
        let bearing_rad =
            (dz.atan2(dx) - physics.yaw + std::f32::consts::PI) % (2.0 * std::f32::consts::PI);
        Some(bearing_rad.to_degrees())
    });

    let bb = ShieldsBlackboard {
        facings,
        hull_integrity_pct,
        focused_facing,
        target_bearing,
        grid_status,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(crate::system_registry::SHIELDS_SYSTEM_ID.to_string()),
            SystemBlackboard::Shields(bb),
        );
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-kind AI plugin for shields.
///
/// Gated on policy.operate_ai for the Shields system. No behaviour is
/// implemented yet — this is a compile-verified stub that will be filled in
/// when the Shields AI controller is designed.
fn operate_shields_ai(
    ships: Query<&crate::ship_plugin::ShipSystemControlSources>,
) {
    for sources in &ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::shields_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement shields AI logic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::{ClientMessage, *};
    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::ship::control_source::ControlSource;
    use crate::ship_plugin::CoordinationEnqueue;
    use crate::simulation::{
        LastBroadcastEntityPositions, LastBroadcastHull, LastBroadcastShields,
        ShipImpulse, ShipShields, SimOutbox,
    };
    use crate::system_registry::SHIELDS_SYSTEM_ID;

    #[derive(Resource)]
    struct ShipEntity(Entity);

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    #[derive(Resource, Default)]
    struct CoordEnqueueBox(Vec<CoordinationEnqueue>);

    fn collect_coord(
        mut reader: MessageReader<CoordinationEnqueue>,
        mut box_: ResMut<CoordEnqueueBox>,
    ) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let config = crate::shield::ShieldConfig {
            num_facings: 2,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::LocalShip,
                ShipShields(crate::shield::ShieldSystem::new(&config)),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                {
                    let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                    cs.0.set(crate::system_registry::shields_system_id(), ControlSource::Ai);
                    cs
                },
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                crate::ship_state::ShipRedAlert::default(),
                ShieldsCoordinationState::default(),
            ))
            .id();
        app.insert_resource(ShipEntity(ship));
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .init_resource::<CoordEnqueueBox>()
            .add_plugins(ShipShieldsPlugin)
            .add_systems(PostUpdate, collect)
            .add_systems(PostUpdate, collect_coord);
        app
    }

    fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
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

    fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
        let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
        app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
        msgs
    }

    fn start_game_with_shields(app: &mut App) {
        push_msg(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "shields",
            ClientMessage::Identify {
                token: "shields".into(),
                name: "Scotty".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "shields",
            ClientMessage::SelectStation {
                station: "Shields".into(),
            },
        );
        tick(app);
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "shields", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    // ── Blackboard publish tests ─────────────────────────────────────────────

    fn shields_bb(app: &mut App) -> ShieldsBlackboard {
        let mut q = app.world_mut().query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        // Safety: test always spawns exactly one LocalShip entity.
        let bbs = q.single(app.world()).expect("no LocalShip with ShipSystemBlackboards");
        let key = SystemId(SHIELDS_SYSTEM_ID.to_string());
        let SystemBlackboard::Shields(bb) = bbs.0.get(&key).unwrap() else {
            panic!("expected Shields blackboard");
        };
        bb.clone()
    }

    #[test]
    fn publish_shields_blackboard_contains_hull_integrity() {
        let mut app = test_app();
        app.update();
        assert!((shields_bb(&mut app).hull_integrity_pct - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn publish_shields_blackboard_four_facings() {
        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        let _ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::LocalShip,
                ShipShields(crate::shield::ShieldSystem::new(&config)),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                {
                    let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                    cs.0.set(crate::system_registry::shields_system_id(), ControlSource::Ai);
                    cs
                },
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
            ))
            .id();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .add_plugins(ShipShieldsPlugin);
        app.update();
        assert_eq!(shields_bb(&mut app).facings.len(), 4);
    }

    fn ship_e(app: &mut App) -> Entity {
        app.world().resource::<ShipEntity>().0
    }

    #[test]
    fn publish_shields_blackboard_shows_focused_facing() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .set_focused_facing(Some(0));
        app.update();
        assert!(shields_bb(&mut app).focused_facing.is_some());
    }

    #[test]
    fn publish_shields_blackboard_clears_focused_facing() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        let mut e = app.world_mut().entity_mut(se);
        let mut shields = e.get_mut::<ShipShields>().unwrap();
        shields.0.set_focused_facing(Some(0));
        shields.0.set_focused_facing(None);
        drop(shields);
        drop(e);
        app.update();
        assert_eq!(shields_bb(&mut app).focused_facing, None);
    }

    #[test]
    fn publish_shields_blackboard_grid_offline_when_facing_down() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);
        app.update();
        assert_eq!(shields_bb(&mut app).grid_status, "EMITTER OFFLINE");
    }

    #[test]
    fn publish_shields_blackboard_stable_on_double_update() {
        let mut app = test_app();
        app.update();
        app.update();
        assert!((shields_bb(&mut app).hull_integrity_pct - 100.0).abs() < f32::EPSILON);
    }

    // ── Coordination tests ──────────────────────────────────────────────────

    fn test_app_with_helm() -> App {
        let config = crate::shield::ShieldConfig {
            num_facings: 2,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::LocalShip,
                ShipShields(crate::shield::ShieldSystem::new(&config)),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                {
                    let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                    cs.0.set(crate::system_registry::shields_system_id(), ControlSource::Ai);
                    cs
                },
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                crate::ship_state::ShipRedAlert::default(),
                ShieldsCoordinationState::default(),
            ))
            .id();
        app.insert_resource(ShipEntity(ship));
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .init_resource::<CoordEnqueueBox>()
            .add_plugins(ShipShieldsPlugin)
            .add_systems(PostUpdate, collect)
            .add_systems(PostUpdate, collect_coord);
        app
    }

    fn start_game_with_shields_and_helm(app: &mut App) {
        push_msg(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Sulu".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "helm", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    #[test]
    fn shield_facing_down_coordination_sent_to_helm_when_facing_goes_offline() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        // Drain facing 0 offline.
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let down_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .collect();

        assert!(
            !down_msgs.is_empty(),
            "expected a ShieldFacingDown CoordinationEnqueue to be sent"
        );
        assert!(
            down_msgs
                .iter()
                .all(|m| m.target == crate::system_registry::helm_system_id()),
            "ShieldFacingDown should target the helm system"
        );
    }

    #[test]
    fn shield_facing_down_fires_only_once_per_offline_cycle() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);

        tick(&mut app); // first tick — fires
        drain_coord(&mut app); // discard first tick's messages

        tick(&mut app); // second tick — should not re-fire
        let coord_msgs = drain_coord(&mut app);

        let count = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .count();

        assert_eq!(
            count, 0,
            "ShieldFacingDown should not fire again on the same offline cycle"
        );
    }

    #[test]
    fn shield_facing_restored_fires_on_red_alert_when_hp_recovers() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        // Put facing offline.
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);
        tick(&mut app);
        drain_coord(&mut app); // discard down notification

        // Manually restore the facing and set HP to above threshold.
        {
            let mut e = app.world_mut().entity_mut(se);
            let mut shields = e.get_mut::<ShipShields>().unwrap();
            let facing = &mut shields.0.facings[0];
            facing.offline_remaining = 0.0;
            facing.hp = 60; // 60/100 = 0.6 >= 0.5 threshold
        }

        // Activate red alert via per-entity ShipRedAlert component.
        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_state::ShipRedAlert, bevy::prelude::With<crate::simulation::LocalShip>>();
            if let Ok(mut ra) = q.single_mut(app.world_mut()) { ra.toggle(); }
        }

        // Mark down_notified on the per-ship ShieldsCoordinationState so
        // the restore branch can fire.
        {
            let se = ship_e(&mut app);
            let mut e = app.world_mut().entity_mut(se);
            let mut coord = e.get_mut::<ShieldsCoordinationState>().unwrap();
            if coord.down_notified.is_empty() {
                coord.down_notified.push(true);
                coord.restore_notified.push(false);
            } else {
                coord.down_notified[0] = true;
            }
        }

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let restored_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
            .collect();

        assert!(
            !restored_msgs.is_empty(),
            "expected a ShieldFacingRestored CoordinationEnqueue on red alert after recovery"
        );
    }

    #[test]
    fn shield_facing_restored_does_not_fire_without_red_alert() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);
        tick(&mut app);
        drain_coord(&mut app); // discard down notification

        {
            let mut e = app.world_mut().entity_mut(se);
            let mut shields = e.get_mut::<ShipShields>().unwrap();
            let facing = &mut shields.0.facings[0];
            facing.offline_remaining = 0.0;
            facing.hp = 60;
        }

        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShieldsCoordinationState>()
            .map(|mut coord| {
                if coord.down_notified.is_empty() {
                    coord.down_notified.push(true);
                    coord.restore_notified.push(false);
                } else {
                    coord.down_notified[0] = true;
                }
            });

        // No red alert active.
        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let count = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
            .count();

        assert_eq!(
            count, 0,
            "ShieldFacingRestored should not fire when not on red alert"
        );
    }

    /// Verify that the `CoordinationEnqueue` event carries `sender_origin = Ai`
    /// by default (no explicit `ShipSystemControlSources` set), confirming the
    /// channel-3 routing matrix will treat it as AI-originated and route
    /// correctly (AI → Human = Popup; AI → AI = Consume) at delivery time.
    #[test]
    fn shield_facing_down_coordination_carries_ai_sender_origin_for_routing() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let down_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .collect();

        assert!(!down_msgs.is_empty(), "expected ShieldFacingDown enqueue");
        assert!(
            down_msgs
                .iter()
                .all(|m| m.sender_origin == ControlSource::Ai),
            "default sender_origin should be Ai (shields console has no holder)"
        );
        assert!(
            down_msgs
                .iter()
                .all(|m| m.target == crate::system_registry::helm_system_id()),
            "ShieldFacingDown should target the helm system"
        );
    }
}
