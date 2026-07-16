use bevy::prelude::*;

use crate::messages::{
    AdmittedCommands, NavigationBlackboard, SystemBlackboard, SystemControlPayload, SystemId,
    WaypointSnapshot,
};
use crate::ship::system_registry::NAVIGATION_SYSTEM_ID;

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            handle_navigation_waypoint.in_set(crate::sim_sets::SimSet::Input),
        )
        // Refresh anchored waypoints from the parent entity's live
        // Transform every tick, before the broadcaster reads the
        // waypoint into the SimSnapshot. Auto-clear when the parent
        // entity is no longer present.
        .add_systems(
            Update,
            refresh_anchored_waypoint.in_set(crate::sim_sets::SimSet::Modifiers),
        )
        .add_systems(
            Update,
            operate_navigation_ai.in_set(crate::sim_sets::SimSet::Physics),
        )
        .add_systems(
            Update,
            publish_navigation_blackboard.in_set(crate::sim_sets::SimSet::Publish),
        );
    }
}

/// Authoritative navigation waypoint state.
///
/// Stores either a free position chosen by tap-to-place, or an entity-anchored
/// waypoint that follows the named entity's transform until the entity
/// despawns.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own waypoint.
///
/// # The Helm's goal, not a copy of it (issue #702)
///
/// This is the one navigation goal on the ship. The AI Helm reads it directly
/// rather than keeping a private `AiMemory.nav_goal` copy laundered through a
/// coordination message — one position, no split brain. Humans
/// (`SetNavigationWaypoint`) and `operate_navigation_ai` write it symmetrically.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct NavigationWaypoint {
    mode: Option<WaypointMode>,
    generation: u64,
}

/// Storage variant of the navigation waypoint.
#[derive(Clone, Debug, PartialEq)]
pub enum WaypointMode {
    /// Tap-to-place: the waypoint is a fixed world position and never moves.
    Free { x: f32, z: f32 },
    /// Anchored to an entity by UUID. `last_x` / `last_z` mirror the entity's
    /// last-known transform; they are refreshed each tick by
    /// [`refresh_anchored_waypoint`]. When the parent entity is no longer
    /// present, the waypoint is auto-cleared.
    Anchored {
        source_uuid: String,
        last_x: f32,
        last_z: f32,
    },
}

/// Whether two modes name the *same* waypoint — the identity the `generation`
/// counts, as distinct from structural equality.
///
/// A `Free` waypoint is its position. An `Anchored` waypoint is its parent
/// entity: `last_x`/`last_z` are a cache of that entity's transform, refreshed
/// every tick by [`refresh_anchored_waypoint`], so comparing them would make a
/// waypoint anchored to a *moving* ship count as brand new on every tick it
/// moved — re-incurring the Channel-3 lag forever and never letting the Helm
/// follow it.
fn same_waypoint(a: &WaypointMode, b: &WaypointMode) -> bool {
    match (a, b) {
        (WaypointMode::Free { x: ax, z: az }, WaypointMode::Free { x: bx, z: bz }) => {
            ax == bx && az == bz
        }
        (
            WaypointMode::Anchored {
                source_uuid: a_uuid,
                ..
            },
            WaypointMode::Anchored {
                source_uuid: b_uuid,
                ..
            },
        ) => a_uuid == b_uuid,
        _ => false,
    }
}

impl NavigationWaypoint {
    /// A waypoint already set to `mode` (generation 1). Test/spawn convenience.
    pub fn new(mode: WaypointMode) -> Self {
        let mut waypoint = Self::default();
        waypoint.set(mode);
        waypoint
    }

    /// The current waypoint, or `None` when none is set.
    pub fn mode(&self) -> Option<&WaypointMode> {
        self.mode.as_ref()
    }

    /// Monotonic id of the *current* waypoint (issue #702).
    ///
    /// Bumped by [`set`](Self::set) and [`clear`](Self::clear) whenever they
    /// actually change which waypoint is named. This is what carries the
    /// Channel-3 Navigation→Helm lag now that `AiMemory.nav_goal` is gone:
    /// `CoordinationPayload::NavigateTo` carries a generation rather than a
    /// position, `process_coordination_lag` latches it into
    /// `HelmWaypointClearance` when the message comes due, and the AI Helm
    /// follows the waypoint only while `clearance == generation`. Every *new*
    /// waypoint therefore re-incurs the lag — which a bare "has been cleared"
    /// bool would not: that would delay only the first.
    ///
    /// A `u64` counter rather than a timestamp because PRD #620 (P2P lockstep)
    /// needs this to be deterministic across peers.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Set the waypoint, bumping [`generation`](Self::generation) if this names
    /// a different waypoint than the current one.
    ///
    /// Re-setting the same waypoint is a no-op: it neither bumps the generation
    /// (which would re-incur the Channel-3 lag on a Helm already following it)
    /// nor marks the component changed. `operate_navigation_ai` re-sets its
    /// waypoint every tick, so this idempotence is load-bearing rather than an
    /// optimisation.
    pub fn set(&mut self, mode: WaypointMode) {
        match &mut self.mode {
            // Same waypoint: refresh the anchor cache in place, no bump.
            Some(current) if same_waypoint(current, &mode) => *current = mode,
            _ => {
                self.mode = Some(mode);
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }

    /// Clear the waypoint, bumping [`generation`](Self::generation) if one was
    /// set. Clearing an already-clear waypoint is a no-op — see [`set`](Self::set).
    pub fn clear(&mut self) {
        if self.mode.is_some() {
            self.mode = None;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Returns the broadcast-shaped snapshot for the current waypoint, or
    /// `None` if no waypoint is set.
    pub fn snapshot(&self) -> Option<WaypointSnapshot> {
        match &self.mode {
            None => None,
            Some(WaypointMode::Free { x, z }) => Some(WaypointSnapshot {
                x: *x,
                z: *z,
                source_uuid: None,
            }),
            Some(WaypointMode::Anchored {
                source_uuid,
                last_x,
                last_z,
            }) => Some(WaypointSnapshot {
                x: *last_x,
                z: *last_z,
                source_uuid: Some(source_uuid.clone()),
            }),
        }
    }
}

fn handle_navigation_waypoint(
    mut ship_query: Query<
        (&AdmittedCommands, &mut NavigationWaypoint),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, mut waypoint) in ship_query.iter_mut() {
        for cmd in admitted.for_target(NAVIGATION_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::SetNavigationWaypoint { x, z, source_uuid }
                    if x.is_finite() && z.is_finite() =>
                {
                    waypoint.set(make_waypoint_mode(*x, *z, source_uuid.as_deref()));
                }
                SystemControlPayload::ClearNavigationWaypoint => {
                    waypoint.clear();
                }
                _ => {}
            }
        }
    }
}

/// Build the appropriate `WaypointMode` from raw coordinates and an optional
/// anchor UUID. An empty UUID string is treated as "no anchor" (free waypoint).
fn make_waypoint_mode(x: f32, z: f32, source_uuid: Option<&str>) -> WaypointMode {
    match source_uuid {
        Some(uuid) if !uuid.is_empty() => WaypointMode::Anchored {
            source_uuid: uuid.to_string(),
            last_x: x,
            last_z: z,
        },
        _ => WaypointMode::Free { x, z },
    }
}

/// Each tick, if any ship's navigation waypoint is anchored to an entity,
/// look up the entity's current `Transform` by `EntityUuid` and refresh
/// the waypoint's stored coordinates. If no entity carries the anchored
/// UUID, auto-clear the waypoint (per the despawn policy). Iterates every
/// ship so both player and NPC waypoints track their anchors.
fn refresh_anchored_waypoint(
    mut waypoint_q: Query<&mut NavigationWaypoint, With<crate::server_app::Ship>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform)>,
) {
    for mut waypoint in waypoint_q.iter_mut() {
        let Some(WaypointMode::Anchored {
            source_uuid,
            last_x,
            last_z,
        }) = waypoint.mode.as_mut()
        else {
            continue;
        };

        let mut found = false;
        for (uuid, transform) in entity_q.iter() {
            if uuid.0 == *source_uuid {
                // Tracking the anchor is not a *new* waypoint: it is the same
                // waypoint, whose parent moved. Mutated in place without
                // bumping the generation, or a waypoint anchored to a moving
                // ship would re-incur the Channel-3 lag every tick and the AI
                // Helm would never follow it (issue #702).
                *last_x = transform.translation.x;
                *last_z = transform.translation.z;
                found = true;
                break;
            }
        }

        if !found {
            // Parent entity has despawned (or never existed). Auto-clear —
            // a real change of waypoint, so this one does bump the generation.
            waypoint.clear();
        }
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

fn publish_navigation_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    waypoint_q: Query<&NavigationWaypoint, With<crate::server_app::LocalShip>>,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    let cfg = &ship_config.0;
    let navigation_waypoint = waypoint_q.single().ok().and_then(|w| w.snapshot());
    let bb = NavigationBlackboard {
        nav_chart_range: cfg.nav_chart_range,
        nav_chart_shows: cfg.nav_chart_shows.clone(),
        nav_chart_selects: cfg.nav_chart_selects.clone(),
        navigation_waypoint,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(NAVIGATION_SYSTEM_ID.to_string()),
            SystemBlackboard::Navigation(bb),
        );
    }
}

// ── AI controller ──────────────────────────────────────────────────────────────

/// Per-entity AI loop for navigation. Loops over ALL ship entities (player and NPC)
/// where the Navigation system is `ControlSource::Ai`.
///
/// Reads the viewscreen blackboard's `scored_objectives`, picks the top
/// Helm-relevant objective with `score > 0`, resolves its `AiDirective` to a
/// world location using a nav-range-filtered entity view, sets the ship's
/// `NavigationWaypoint` (AI write path), and emits a `NavigateTo` coordination
/// message to Helm.
///
/// `NavigateTo` carries the waypoint's `generation`, not its position (issue
/// #702): the waypoint itself is the goal, and the message only tells the Helm
/// *which* waypoint it is now cleared to follow. See
/// [`NavigationWaypoint::generation`].
pub fn operate_navigation_ai(
    mut ships: Query<(
        Entity,
        &crate::ship_plugin::ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &mut NavigationWaypoint,
        &crate::ship_state::ShipPhysics,
        Option<&crate::entity_spawner::EntityUuid>,
        Option<&crate::ai_plugin::ObjectiveCursors>,
    )>,
    entities: Query<(&crate::entity_spawner::EntityUuid, &Transform)>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut coordination_writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    let all_entities: Vec<(String, [f32; 3])> = entities
        .iter()
        .map(|(uuid, transform)| {
            (
                uuid.0.clone(),
                [
                    transform.translation.x,
                    transform.translation.y,
                    transform.translation.z,
                ],
            )
        })
        .collect();

    let nav_range = ship_client_config
        .as_ref()
        .map(|c| c.0.nav_chart_range)
        .unwrap_or(0.0);

    for (entity, sources, blackboards, mut waypoint, ship_physics, _self_uuid_opt, cursors) in
        ships.iter_mut()
    {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        if !policy.operate_ai {
            continue;
        }

        let scored: Vec<crate::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
            _ => vec![],
        };

        let top = scored
            .iter()
            .filter(|o| {
                o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
            })
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        let Some(top_obj) = top else {
            waypoint.clear();
            continue;
        };

        let ship_pos = [ship_physics.x, 0.0, ship_physics.z];
        let nav_filtered: Vec<(String, [f32; 3])> = if nav_range > 0.0 && nav_range.is_finite() {
            all_entities
                .iter()
                .filter(|(_, pos)| {
                    let dx = pos[0] - ship_pos[0];
                    let dz = pos[2] - ship_pos[2];
                    (dx * dx + dz * dz).sqrt() <= nav_range
                })
                .cloned()
                .collect()
        } else {
            all_entities.clone()
        };

        // Resolve the directive to the waypoint Navigation wants the Helm to
        // travel to, or `None` when it names nowhere reachable.
        let resolved: Option<WaypointMode> = match &top_obj.directive {
            crate::messages::AiDirective::Destroy { target } => (!target.is_empty())
                .then(|| nav_filtered.iter().find(|(uuid, _)| uuid == target))
                .flatten()
                .map(|(_, pos)| WaypointMode::Anchored {
                    source_uuid: target.clone(),
                    last_x: pos[0],
                    last_z: pos[2],
                }),
            crate::messages::AiDirective::Reach { anchor } => (!anchor.is_empty())
                .then(|| anchor_pos(&world_config, anchor))
                .flatten()
                .map(|pos| WaypointMode::Free {
                    x: pos[0],
                    z: pos[2],
                }),
            crate::messages::AiDirective::Retreat { anchor } => (!anchor.is_empty())
                .then(|| anchor_pos(&world_config, anchor))
                .flatten()
                .map(|pos| WaypointMode::Free {
                    x: pos[0],
                    z: pos[2],
                }),
            crate::messages::AiDirective::Patrol { anchors, loop_path } => {
                // Resolve from the objective's *active cursor target*, not
                // `anchors[0]` (issue #702). This system was cursor-blind: it
                // parked the waypoint on the first anchor of the route and left
                // it there for the whole patrol, so Navigation kept telling the
                // Helm to fly to a waypoint the ship had already rounded laps
                // ago. The cursor is the objective's own record of where it is
                // on its route — the same one `helm_patrol` steers from and
                // `advance_objective_cursors` advances — so reading it is what
                // makes Navigation and Helm agree about which waypoint is
                // current.
                let index = cursors
                    .and_then(|c| {
                        c.0.iter()
                            .find(|cursor| cursor.objective_id == top_obj.id)
                            .map(|cursor| cursor.index())
                    })
                    .unwrap_or(0);
                let world_anchors = world_config
                    .as_ref()
                    .map(|wc| wc.anchors.clone())
                    .unwrap_or_default();
                crate::ai::patrol_cursor::cursor_target(index, anchors, *loop_path, &world_anchors)
                    .map(|pos| WaypointMode::Free {
                        x: pos[0],
                        z: pos[2],
                    })
            }
            _ => None,
        };

        let Some(mode) = resolved else {
            waypoint.clear();
            continue;
        };

        // `set` is idempotent for an unchanged waypoint, so re-running this
        // every tick does not re-bump the generation and re-incur the lag on a
        // Helm already following it.
        waypoint.set(mode);
        coordination_writer.write(crate::ship_plugin::CoordinationEnqueue {
            source_entity: entity,
            sender_origin: crate::ship::control_source::ControlSource::Ai,
            target: crate::system_registry::helm_system_id(),
            payload: crate::messages::CoordinationPayload::NavigateTo {
                generation: waypoint.generation(),
                label: top_obj.snapshot.text.clone(),
            },
            sender_label: "Navigation".into(),
        });
    }
}

/// World position of a named anchor, if the world config declares one.
fn anchor_pos(
    world_config: &Option<Res<crate::world::config::WorldConfig>>,
    anchor: &str,
) -> Option<[f32; 3]> {
    world_config
        .as_ref()
        .and_then(|wc| wc.anchors.get(anchor).copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::{ClientMessage, ServerMessage};
    use crate::server_app::{
        sim_state_broadcaster, LastBroadcastEntityPositions, LastBroadcastHull,
        LastBroadcastShields, ShipImpulse,
    };

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut sink: ResMut<Outbox>) {
        for msg in reader.read() {
            sink.0.push(msg.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            // Chain SimSet phases so handle (Input) → refresh (Modifiers) →
            // broadcast (Broadcast) run in the right order. Without this,
            // adding a second resource-touching system to a different set
            // makes the schedule non-deterministic and breaks the existing
            // broadcast assertions.
            .configure_sets(
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
            .init_resource::<crate::server_app::LastBroadcastBlackboards>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(NavigationPlugin)
            .add_plugins(sim_state_broadcaster())
            .add_plugins(crate::server_app::sim_outbox_broadcaster())
            .init_resource::<crate::simulation::SimOutbox>()
            .add_systems(
                Update,
                crate::server_app::broadcast_blackboard_updates
                    .in_set(crate::sim_sets::SimSet::PublishAggregate),
            )
            .init_resource::<Outbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .add_message::<crate::ship_plugin::CoordinationEnqueue>()
            .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_navigation_waypoint can query it.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::simulation::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            // PR 7 (issue #597) — NavigationWaypoint is now a per-entity Component.
            NavigationWaypoint::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            crate::modifiers::ShipModifiers::new(),
            crate::ship_state::ShipPhysics::default(),
        ));
        app
    }

    /// PR 7 test helper — read the LocalShip's `NavigationWaypoint` component.
    fn get_nav_waypoint(app: &mut App) -> Option<WaypointMode> {
        let mut q = app
            .world_mut()
            .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
        q.single(app.world()).ok().and_then(|w| w.mode().cloned())
    }

    /// The LocalShip waypoint's current generation (issue #702).
    fn nav_waypoint_generation(app: &mut App) -> u64 {
        let mut q = app
            .world_mut()
            .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry a NavigationWaypoint")
            .generation()
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
        let out = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game_with_navigation(app: &mut App) {
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
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "navigation", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    // Test helper mirroring `latest_navigation_blackboard` below; no test in
    // this module currently asserts on raw SimSnapshot, retained for parity.
    #[allow(dead_code)]
    fn latest_sim_snapshot(out: &[OutboundMessage]) -> Option<crate::messages::SimSnapshot> {
        out.iter().rev().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        })
    }

    fn latest_navigation_blackboard(
        out: &[OutboundMessage],
    ) -> Option<crate::messages::NavigationBlackboard> {
        out.iter().rev().find_map(|m| match &m.msg {
            ServerMessage::BlackboardUpdate { updates } => {
                updates.iter().find_map(|(_, bb)| match bb {
                    crate::messages::SystemBlackboard::Navigation(nav) => Some(nav.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
    }

    #[test]
    fn navigation_holder_can_set_and_clear_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 120.0,
                    z: -45.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 120.0, z: -45.0 })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    #[test]
    fn non_navigation_sender_cannot_change_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 6.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    #[test]
    fn invalid_waypoint_coordinates_are_ignored() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: f32::NAN,
                    z: 1.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    #[test]
    fn sim_state_broadcast_includes_and_omits_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 10.0,
                    z: 20.0,
                    source_uuid: None,
                },
            },
        );
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 10.0,
                z: 20.0,
                source_uuid: None,
            })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert!(bb.navigation_waypoint.is_none());
    }

    #[test]
    fn anchored_waypoint_tracks_moving_entity() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        // Spawn an entity carrying EntityUuid + Transform that the waypoint
        // will anchor to.
        let target_uuid = "target-1";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(50.0, 0.0, -100.0),
            ))
            .id();

        // Anchor the waypoint to that entity. The seed coords are the
        // entity's current position.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 50.0,
                    z: -100.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 50.0,
                z: -100.0,
                source_uuid: Some(target_uuid.into()),
            })
        );

        // Move the entity. The next broadcast should reflect the new
        // position with source_uuid preserved.
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(75.0, 0.0, -150.0));
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 75.0,
                z: -150.0,
                source_uuid: Some(target_uuid.into()),
            })
        );
    }

    #[test]
    fn anchored_waypoint_auto_clears_when_parent_despawns() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        let target_uuid = "target-despawn";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(10.0, 0.0, 20.0),
            ))
            .id();

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 10.0,
                    z: 20.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_some());

        // Despawn the parent entity. The next tick must auto-clear.
        app.world_mut().entity_mut(target).despawn();
        let out = tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert!(bb.navigation_waypoint.is_none());
    }

    #[test]
    fn empty_source_uuid_is_treated_as_free_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 1.0,
                    z: 2.0,
                    source_uuid: Some(String::new()),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 1.0, z: 2.0 })
        );
    }

    // ── ControlSystem dispatch tests ─────────────────────────────────────────

    /// Navigation holder sends `ControlSystem` waypoint — accepted.
    #[test]
    fn control_system_navigation_holder_can_set_and_clear_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 200.0,
                    z: -80.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 200.0, z: -80.0 })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    /// Non-navigation sender sends `ControlSystem` waypoint — rejected.
    #[test]
    fn control_system_unauthorized_sender_rejected() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 6.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "non-navigation sender should be rejected"
        );
    }

    /// When navigation system is AI-controlled, `ControlSystem` waypoint is rejected.
    #[test]
    fn control_system_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::navigation_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
        }

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 99.0,
                    z: 99.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "should reject waypoint when navigation is AI-controlled"
        );
    }

    /// Anchored waypoint set via `ControlSystem` still tracks the entity.
    #[test]
    fn control_system_anchored_waypoint_tracks_entity() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        let target_uuid = "anchor-cs-test";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(30.0, 0.0, -60.0),
            ))
            .id();

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 30.0,
                    z: -60.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored {
                source_uuid: target_uuid.into(),
                last_x: 30.0,
                last_z: -60.0,
            })
        );

        // Move entity — next tick should update last_x/last_z.
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(40.0, 0.0, -70.0));
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored {
                source_uuid: target_uuid.into(),
                last_x: 40.0,
                last_z: -70.0,
            })
        );
    }

    #[test]
    fn control_system_set_navigation_waypoint_works() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 15.0,
                    z: 25.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 15.0, z: 25.0 })
        );
    }

    #[test]
    fn control_system_clear_navigation_waypoint_works() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 5.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    // ── Helpers for operate_navigation_ai integration tests ────────────────

    fn set_navigation_control_source(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
    ) {
        let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(crate::system_registry::navigation_system_id(), source);
        }
    }

    fn inject_viewscreen_objective(
        app: &mut App,
        objectives: Vec<crate::messages::ScoredObjective>,
    ) {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        use crate::server_app::ShipSystemBlackboards;

        let bb = ViewscreenBlackboard {
            scored_objectives: objectives,
            ..Default::default()
        };
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
            bbs.0.insert(
                crate::system_registry::viewscreen_system_id(),
                SystemBlackboard::Viewscreen(bb),
            );
        }
    }

    fn spawn_test_entity(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    #[derive(Resource, Default)]
    struct NavCoordCapture(Vec<crate::ship_plugin::CoordinationEnqueue>);

    fn capture_nav_coord(
        mut reader: MessageReader<crate::ship_plugin::CoordinationEnqueue>,
        mut capture: ResMut<NavCoordCapture>,
    ) {
        for ev in reader.read() {
            capture.0.push(ev.clone());
        }
    }

    fn drain_nav_coord(app: &mut App) -> Vec<crate::ship_plugin::CoordinationEnqueue> {
        let msgs = app.world().resource::<NavCoordCapture>().0.clone();
        app.world_mut().resource_mut::<NavCoordCapture>().0.clear();
        msgs
    }

    #[test]
    fn operate_navigation_ai_destroy_sets_anchored_waypoint_and_emits_navigate_to() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        // Insert the entity within nav range (default 500).
        spawn_test_entity(&mut app, "target-entity", 400.0, 0.0);

        // Inject Destroy objective with score > 0.
        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "destroy-test".into(),
                score: 80.0,
                directive: crate::messages::AiDirective::Destroy {
                    target: "target-entity".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![
                    crate::messages::SystemAffinity::Helm,
                    crate::messages::SystemAffinity::Weapons,
                ],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "destroy-test".into(),
                    text: "Destroy target".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec!["target-entity".into()],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        // Check waypoint is set (Anchored).
        let wp = get_nav_waypoint(&mut app);
        assert!(
            matches!(wp, Some(WaypointMode::Anchored { .. })),
            "expected Anchored waypoint, got {:?}",
            wp
        );
        if let Some(WaypointMode::Anchored {
            source_uuid,
            last_x,
            last_z,
        }) = wp
        {
            assert_eq!(source_uuid, "target-entity");
            assert!((last_x - 400.0).abs() < 0.01);
            assert!((last_z - 0.0).abs() < 0.01);
        }

        // Check NavigateTo was emitted.
        let coords = drain_nav_coord(&mut app);
        let nav_to = coords.iter().find(|c| {
            matches!(
                &c.payload,
                crate::messages::CoordinationPayload::NavigateTo { .. }
            )
        });
        assert!(nav_to.is_some(), "expected NavigateTo coordination event");
    }

    #[test]
    fn operate_navigation_ai_reach_sets_free_waypoint_and_emits_navigate_to() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        // Insert a WorldConfig with an anchor.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("base".into(), [300.0, 0.0, -100.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "reach-test".into(),
                score: 70.0,
                directive: crate::messages::AiDirective::Reach {
                    anchor: "base".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "reach-test".into(),
                    text: "Reach base".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        // Check waypoint is Free.
        let wp = get_nav_waypoint(&mut app);
        assert_eq!(
            wp,
            Some(WaypointMode::Free {
                x: 300.0,
                z: -100.0
            })
        );

        // Check NavigateTo was emitted.
        let coords = drain_nav_coord(&mut app);
        let nav_to = coords.iter().find(|c| {
            matches!(
                &c.payload,
                crate::messages::CoordinationPayload::NavigateTo { .. }
            )
        });
        assert!(nav_to.is_some(), "expected NavigateTo coordination event");
        if let Some(crate::messages::CoordinationPayload::NavigateTo { generation, label }) =
            nav_to.map(|c| &c.payload)
        {
            // The message names *which* waypoint the Helm is cleared for; the
            // position lives on the waypoint itself (asserted above). Post-#702
            // no coordinates travel on the wire at all — that duplication was
            // the `nav_goal` split brain.
            assert_eq!(
                *generation,
                nav_waypoint_generation(&mut app),
                "NavigateTo must carry the current waypoint's generation, or the                  Helm's clearance can never match and it will never fly it"
            );
            assert_eq!(label, "Reach base");
        }
    }

    #[test]
    fn operate_navigation_ai_patrol_sets_free_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("patrol_pt".into(), [200.0, 0.0, 50.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "patrol-test".into(),
                score: 60.0,
                directive: crate::messages::AiDirective::Patrol {
                    anchors: vec!["patrol_pt".into()],
                    loop_path: true,
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "patrol-test".into(),
                    text: "Patrol area".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        let wp = get_nav_waypoint(&mut app);
        assert_eq!(wp, Some(WaypointMode::Free { x: 200.0, z: 50.0 }));
    }

    /// Navigation resolves a Patrol from the objective's **active cursor
    /// target**, not `anchors[0]` (issue #702).
    ///
    /// This system was cursor-blind: it parked the waypoint on the first anchor
    /// of the route and left it there for the whole patrol, so once the ship had
    /// rounded its first waypoint Navigation was still telling the Helm to fly
    /// to a leg it had finished laps ago. The cursor is the objective's own
    /// record of where it is on its route — the same one `helm_patrol` steers
    /// from and `advance_objective_cursors` advances — so reading it is what
    /// makes the two consoles agree.
    ///
    /// The route below needs two distinct anchors to tell the two behaviours
    /// apart: with a one-anchor route (as
    /// `operate_navigation_ai_patrol_sets_free_waypoint` uses) index 0 and the
    /// cursor always agree, and a cursor-blind implementation passes.
    #[test]
    fn operate_navigation_ai_patrol_follows_the_objective_cursor() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("leg_a".into(), [100.0, 0.0, 0.0]);
        wc.anchors.insert("leg_b".into(), [900.0, 0.0, -400.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "patrol-test".into(),
                score: 60.0,
                directive: crate::messages::AiDirective::Patrol {
                    anchors: vec!["leg_a".into(), "leg_b".into()],
                    loop_path: true,
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "patrol-test".into(),
                    text: "Patrol area".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        // The ship has already rounded leg_a; its cursor names leg_b.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .expect("LocalShip");
        let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol-test");
        crate::ai::patrol_cursor::advance_cursor(
            &mut cursor,
            &["leg_a".to_string(), "leg_b".to_string()],
            true,
            [100.0, 0.0, 0.0], // sitting on leg_a
            &app.world()
                .resource::<crate::world::config::WorldConfig>()
                .anchors
                .clone(),
            crate::ai::WAYPOINT_ARRIVAL_RADIUS,
        );
        assert_eq!(cursor.index(), 1, "precondition: cursor must name leg_b");
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ai_plugin::ObjectiveCursors(vec![cursor]));

        tick(&mut app);

        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free {
                x: 900.0,
                z: -400.0
            }),
            "Navigation must place the waypoint on the cursor's current leg (leg_b), \
             not on the route's first anchor (leg_a) — a cursor-blind Navigation \
             keeps ordering the Helm back to a leg it has already flown"
        );
    }

    #[test]
    fn operate_navigation_ai_no_objective_clears_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // First set a waypoint to verify it gets cleared.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut NavigationWaypoint, With<crate::server_app::LocalShip>>();
            if let Ok(mut wp) = q.single_mut(app.world_mut()) {
                wp.set(WaypointMode::Free { x: 500.0, z: 500.0 });
            }
        }
        assert!(
            get_nav_waypoint(&mut app).is_some(),
            "waypoint must be set before clearing test"
        );

        // Inject empty scored_objectives.
        inject_viewscreen_objective(&mut app, vec![]);

        tick(&mut app);

        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "waypoint must be cleared when no objective"
        );
    }

    #[test]
    fn operate_navigation_ai_target_out_of_nav_range_skips_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // Spawn target beyond default nav_chart_range (500).
        spawn_test_entity(&mut app, "far-entity", 1000.0, 0.0);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "destroy-far".into(),
                score: 80.0,
                directive: crate::messages::AiDirective::Destroy {
                    target: "far-entity".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "destroy-far".into(),
                    text: "Destroy far target".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec!["far-entity".into()],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        // The target is beyond nav range, so waypoint should be cleared.
        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "waypoint must be None when target is beyond nav range"
        );
    }

    #[test]
    fn operate_navigation_ai_human_controlled_does_not_set_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        // Keep Navigation on Human control (default).
        // set_navigation_control_source is NOT called.

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("some_anchor".into(), [100.0, 0.0, 0.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "reach-human".into(),
                score: 50.0,
                directive: crate::messages::AiDirective::Reach {
                    anchor: "some_anchor".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "reach-human".into(),
                    text: "Reach".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "human-controlled navigation must not set waypoints"
        );
    }

    /// Verifies operate_navigation_ai runs per-entity for AI-controlled ships (issue #592 AC).
    #[test]
    fn operate_navigation_ai_per_entity_ai_gate() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        use crate::ship_plugin::ShipSystemControlSources;

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::navigation_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        assert!(
            policy.operate_ai,
            "AI Navigation must gate through operate_ai"
        );

        // Human-controlled navigation must not operate AI.
        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::navigation_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        assert!(
            !human_policy.operate_ai,
            "Human Navigation must not operate AI"
        );
    }
}
