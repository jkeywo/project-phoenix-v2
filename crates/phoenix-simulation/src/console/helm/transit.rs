//! One actual drive's admitted order, paid phases and discrete physical arrival.
//! Course/Stock/Power/ShipPhysics remain their existing owners.
use crate::{
    core::messages::{
        AdmittedCommands, AiDirective, SystemBlackboard, SystemControlPayload as Control, SystemId,
    },
    entities::spawner::EntityUuid,
    infrastructure::InfrastructureCondition,
    server_app::ShipSystemBlackboards,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_model::transit::*;
use phoenix_sim_gameplay::ship::{composition::transit_composition, transit::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[path = "transit_ai.rs"]
mod ai;
#[path = "transit_placement.rs"]
pub(crate) mod placement;

#[path = "transit_worlds.rs"]
mod worlds;
pub(crate) use worlds::StaticTransitScopes;

pub(crate) fn register(app: &mut App) {
    use crate::{
        authoritative::{DeclareState, StateClass},
        command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer},
        core::messages::SystemControlPayloadDiscriminants as Payload,
        sim_sets::{FixedStep, SimSet},
    };
    crate::ai::cadence::register_ai_cadence(app);
    for (payload, feedback) in [
        (
            Payload::BeginTransitToWaypoint,
            &[Payload::BeginTransitToWaypoint][..],
        ),
        (Payload::CancelTransit, &[Payload::CancelTransit][..]),
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(
                crate::ship::system_registry::TRANSIT_DRIVE_KIND,
                payload,
            )
            .with_feedback(FeedbackAddress::DeclaredKindOnly, feedback)
            .with_capability_ownership(),
        );
    }
    app.declare_state::<ShipTransitDrives>(StateClass::Folded, "transit-drive-orders")
        .declare_state::<TransitReadouts>(StateClass::Derived, "transit-last-step-readouts")
        .declare_state::<StaticArrivalPolicy>(StateClass::Derived, "static-arrival-body-policy")
        .declare_state::<StaticTransitScopes>(
            StateClass::Derived,
            "accepted-static-transit-world-recipes",
        )
        .add_systems(
            FixedUpdate,
            ai::operate
                .in_set(SimSet::Physics)
                .in_set(FixedStep::OperateTransitAi)
                .run_if(crate::ai::cadence::ai_tick_ready)
                .run_if(any_with_component::<ShipTransitDrives>),
        )
        .add_systems(
            FixedUpdate,
            handle_commands
                .in_set(SimSet::Physics)
                .in_set(FixedStep::HandleTransitCommands)
                .run_if(any_with_component::<ShipTransitDrives>),
        )
        .add_systems(
            FixedUpdate,
            tick.in_set(SimSet::Physics)
                .in_set(FixedStep::TickTransit)
                .run_if(any_with_component::<ShipTransitDrives>),
        );
    app.add_systems(
        FixedUpdate,
        publish
            .in_set(SimSet::Publish)
            .in_set(FixedStep::PublishTransit),
    );
}

pub(crate) fn publish(
    scopes: Option<Res<StaticTransitScopes>>,
    mut ships: Query<(
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipTransitDrives,
        &TransitReadouts,
        &mut ShipSystemBlackboards,
        Option<&ShipPhysics>,
        Option<&crate::console::navigation::NavigationWaypoint>,
        Option<&ShipPowerSystem>,
        (
            Option<&crate::resources::fitting_server::CompiledShipFitting>,
            Option<&crate::resources::fitting_server::ShipFitting>,
        ),
    )>,
) {
    for (config, sources, drives, readouts, mut boards, physics, waypoint, power, fitting) in
        &mut ships
    {
        let mut publications = Vec::new();
        for (owner, state) in &drives.0 {
            let Some(binding) = config
                .0
                .system(owner)
                .filter(|system| system.kind == crate::ship::system_registry::TRANSIT_DRIVE_KIND)
                .and_then(|system| transit_composition(&system.components))
            else {
                continue;
            };
            let current = physics.zip(waypoint).and_then(|(physics, waypoint)| {
                course(&config.0, &boards, owner, waypoint, physics.y)
            });
            let order = state.phase.order();
            let phase = match state.phase {
                TransitPhase::Idle => TransitPhaseView::Idle,
                TransitPhase::Preparing { .. } => TransitPhaseView::Preparing,
                TransitPhase::Committed { .. } => TransitPhaseView::Committed,
                TransitPhase::Arrived { .. } => TransitPhaseView::Arrived,
            };
            let allowed = online(sources, owner)
                && match fitting {
                    (Some(profile), Some(state)) => {
                        !crate::resources::fitting_server::unavailable(profile, state, owner)
                    }
                    (None, None) => true,
                    _ => false,
                };
            let can_begin = !drives.0.values().any(|state| state.phase.active())
                && scopes
                    .as_deref()
                    .map_or(drives.1 == TransitWorldBinding::Base, |scopes| {
                        scopes.validate_context(drives, u64::MAX).is_ok()
                    })
                && allowed
                && physics.is_some()
                && current.as_ref().is_some_and(|destination| {
                    if !known_route(destination, scopes.as_deref()) {
                        return false;
                    }
                    let Some(power) = power else {
                        return false;
                    };
                    let Some(consumer) = power
                        .0
                        .config()
                        .consumers
                        .iter()
                        .find(|consumer| &consumer.system == owner)
                    else {
                        return false;
                    };
                    let own = physics.expect("physical owner checked");
                    TransitOrder {
                        destination: destination.clone(),
                        original_world: drives.1.clone(),
                        origin: [own.x, own.y, own.z],
                        profile: AcceptedLocalJumpProfile {
                            config: binding.config.clone(),
                            preparation_energy_per_sec: consumer.continuous_cost_per_sec,
                            commit_energy: consumer.discrete_cost,
                        },
                        accepted_tick: 0,
                        request: None,
                    }
                    .validate(0)
                    .is_ok()
                });
            let preparation_fraction = match phase {
                TransitPhaseView::Idle => 0.0,
                TransitPhaseView::Preparing => {
                    (state.worked_secs()
                        / f64::from(
                            order
                                .expect("preparing order")
                                .profile
                                .config
                                .preparation_work_secs,
                        )) as f32
                }
                _ => 1.0,
            };
            let waiting = match readouts.0.get(owner) {
                Some(TransitAdvance::Waiting(reason)) => Some(match reason {
                    TransitWait::Unavailable => TransitWaitingView::Unavailable,
                    TransitWait::Unfunded => TransitWaitingView::Unfunded,
                    TransitWait::OutOfRange => TransitWaitingView::OutOfRange,
                    TransitWait::NoPlacement => TransitWaitingView::NoPlacement,
                }),
                _ => None,
            };
            let view = TransitDriveView {
                owner: owner.clone(),
                phase,
                preparation_fraction,
                fuel_required: order.map_or(binding.config.fuel.amount, |order| {
                    order.profile.config.fuel.amount
                }),
                can_begin,
                can_cancel: online(sources, owner)
                    && matches!(state.phase, TransitPhase::Preparing { .. }),
                waiting,
                request: order.and_then(|order| order.request.clone()),
                course: order.map(|order| order.destination.clone()).or(current),
                actual_arrival: match &state.phase {
                    TransitPhase::Arrived { arrival, .. } => Some(arrival.point),
                    _ => None,
                },
                last_step_tick: state.last_tick,
            };
            if view.validate().is_ok() {
                publications.push((owner.clone(), SystemBlackboard::Transit(view)));
            }
        }
        boards.0.retain(|key, value| {
            !matches!(value, SystemBlackboard::Transit(_))
                || publications.iter().any(|(owner, _)| owner == key)
        });
        boards.0.extend(publications);
    }
}

#[derive(Component, Clone, Debug, Default)]
pub struct TransitReadouts(pub BTreeMap<SystemId, TransitAdvance>);

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipTransitDrives(
    pub BTreeMap<SystemId, TransitState>,
    /// Actual accepted current arrival-policy context, independent of origin lifetime.
    pub TransitWorldBinding,
    pub Option<TransitWorldArrival>,
);

impl ShipTransitDrives {
    pub fn from_ship(ship: &crate::ship::config::ShipConfig) -> Option<Self> {
        let states: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::TRANSIT_DRIVE_KIND)
            .map(|system| (system.id.clone(), TransitState::default()))
            .collect();
        (!states.is_empty()).then_some(Self(states, TransitWorldBinding::Base, None))
    }
    pub fn validate(
        &self,
        ship: &crate::ship::config::ShipConfig,
        tick: u64,
    ) -> Result<(), String> {
        self.1.validate()?;
        if let Some(receipt) = &self.2 {
            if receipt.tick == 0
                || receipt.tick > tick
                || self
                    .0
                    .get(&receipt.owner)
                    .is_none_or(|state| state.last_tick.is_none_or(|last| last < receipt.tick))
                || ship
                    .system(&receipt.owner)
                    .and_then(|system| transit_composition(&system.components))
                    .is_none_or(|binding| !binding.config.world_travel)
                || receipt.route.trim().is_empty()
                || receipt.route.len() > 512
                || receipt.route.chars().any(char::is_control)
                || !self.0.contains_key(&receipt.owner)
            {
                return Err("invalid latest actual World-arrival frontier".into());
            }
        }
        let expected = Self::from_ship(ship).unwrap_or_default();
        if !self.0.keys().eq(expected.0.keys()) {
            return Err("saved Transit owner roster differs from accepted actual drives".into());
        }
        if self.0.values().filter(|state| state.phase.active()).count() > 1 {
            return Err(
                "multiple active Transit orders cannot share one physical arrival owner".into(),
            );
        }
        for (owner, state) in &self.0 {
            let system = ship.system(owner).ok_or("missing actual drive")?;
            crate::ship::composition::validate_composition(system)?;
            state.validate(tick)?;
            if let Some(order) = state.phase.order() {
                let binding = transit_composition(&system.components)
                    .ok_or("saved Transit owner has no accepted drive component")?;
                if order.profile.config != *binding.config {
                    return Err(
                        "saved Transit profile differs from accepted installed drive configuration"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }
}

/// Immutable body capability, derived from the accepted ordinary entity config.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct StaticArrivalPolicy(pub ArrivalGravityConfig);

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(state) = config
        .ship_config
        .as_ref()
        .and_then(ShipTransitDrives::from_ship)
    {
        commands.insert((state, TransitReadouts::default()));
    }
    if let Some(policy) = &config.arrival_gravity {
        commands.insert(StaticArrivalPolicy(policy.clone()));
    }
}

// Differential owner tests retain the original two eager clone lines. This
// oracle is absent from production parameters, resources and execution.
#[cfg(test)]
#[derive(Resource)]
pub(crate) struct EagerTransitStaging;

#[allow(clippy::too_many_arguments)]
pub(crate) fn tick(
    now: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    world: Res<crate::world::config::WorldConfig>,
    #[cfg(test)] eager: Option<Res<EagerTransitStaging>>,
    context: (
        Option<Res<StaticTransitScopes>>,
        Option<Res<crate::world::server::WorldLayerMap>>,
        Option<ResMut<crate::world::server::PendingWorldLayerChanges>>,
    ),
    mut runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    pending_capacity: Option<
        Res<crate::effect_queue::EffectQueue<crate::infrastructure::CapacityAdjustment>>,
    >,
    bodies: Query<
        (
            &EntityUuid,
            &Transform,
            &StaticArrivalPolicy,
            Option<&ShipPhysics>,
            Option<&crate::world::server::EntityOriginLayer>,
        ),
        Without<ShipTransitDrives>,
    >,
    mut ships: Query<(
        &EntityUuid,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &mut ShipTransitDrives,
        &mut TransitReadouts,
        Option<&mut ShipPowerSystem>,
        Option<&mut ShipPhysics>,
        Option<&mut InfrastructureCondition>,
        Option<&crate::resources::logistics_server::CompiledShipLogistics>,
        Option<&ShipDamage>,
        (
            Option<&crate::resources::fitting_server::CompiledShipFitting>,
            Option<&crate::resources::fitting_server::ShipFitting>,
        ),
    )>,
) {
    let dt = fixed.timestep().as_secs_f64();
    let (scopes, layers, mut pending_layers) = context;
    let accepted_base = world
        .transit_destinations
        .is_empty()
        .then(|| placement::accepted_bodies(&world).ok())
        .flatten();
    let mut body_points = BTreeMap::new();
    for (uuid, transform, policy, physics, origin) in &bodies {
        let point = physics.map_or(transform.translation.to_array(), |physics| {
            [physics.x, physics.y, physics.z]
        });
        // Duplicate actual identities cannot choose a physical source by ECS
        // traversal order. Withhold this placement until unambiguous again.
        body_points
            .entry(uuid.0.as_str())
            .and_modify(|entry| *entry = None)
            .or_insert(Some((
                point,
                &policy.0,
                origin.map(|origin| origin.0.as_str()),
            )));
    }
    let mut order: Vec<_> = ships.iter().map(|(uuid, ..)| uuid.0.clone()).collect();
    order.sort();
    for id in order {
        let Some((
            uuid,
            config,
            sources,
            mut drives,
            mut readouts,
            mut power,
            mut physics,
            mut inventory,
            logistics,
            damage,
            fitting,
        )) = ships.iter_mut().find(|(uuid, ..)| uuid.0 == id)
        else {
            continue;
        };
        readouts.0.retain(|owner, _| drives.0.contains_key(owner));
        let mut pending = BTreeMap::<String, i64>::new();
        for change in pending_capacity
            .as_deref()
            .into_iter()
            .flat_map(|queue| &queue.0)
            .filter(|change| change.uuid == uuid.0 && change.delta < 0)
        {
            let debit = pending.entry(change.capacity.clone()).or_default();
            *debit = debit.saturating_add(change.delta.saturating_neg());
        }
        let mut arrived_world = None;
        for (owner, state) in &mut drives.0 {
            if state.last_tick == Some(now.0) {
                continue;
            }
            let selected_scope = state
                .phase
                .order()
                .and_then(|order| scopes.as_deref()?.for_order(order));
            let layers_ready = selected_scope.is_none_or(|scope| {
                worlds::ensure_layers(
                    scope,
                    scopes.as_deref().expect("scope owner"),
                    layers.as_deref(),
                    pending_layers.as_deref_mut(),
                    state.phase.order().is_some_and(|order| {
                        matches!(order.destination, TransitDestination::World { .. })
                    }),
                )
            });
            // Idle/arrived/unavailable steps must still advance their clock and
            // readout. Stage actual resource owners only if advance requests work.
            let mut staged_power = None;
            let mut staged_inventory = None;
            #[cfg(test)]
            if eager.is_some() {
                staged_power = power.as_deref().map(|power| power.0.clone());
                staged_inventory = inventory.as_deref().map(|inventory| inventory.0.clone());
            }
            let mut flags = Vec::new();
            let available = power.as_deref().is_some_and(|power|power.0.config().consumers.iter().any(|consumer|&consumer.system==owner)) && online(sources, owner)
                && inventory.is_some()
                && logistics.is_some()
                && config.0.system(owner).is_some_and(|system| {
                    system.kind == crate::ship::system_registry::TRANSIT_DRIVE_KIND
                        && transit_composition(&system.components).is_some()
                })
                && match fitting {
                    (Some(profile), Some(state)) => {
                        !crate::resources::fitting_server::unavailable(profile, state, owner)
                    }
                    (None, None) => true,
                    _ => false,
                }
                && state.phase.order().is_none_or(|order| {
                    logistics.and_then(|profile|profile.0.store(&order.profile.config.fuel.store).ok())
                        .is_some_and(|store|store.role==phoenix_model::logistics::StoreRole::Local && store.system.as_ref()==Some(owner)
                            && matches!(&store.endpoint,phoenix_model::logistics::ResourceEndpoint::Capacity{id} if inventory.as_deref().is_some_and(|inventory|inventory.0.capacity_stock(id).is_some())))
                });
            let operation = damage.map_or(1.0, |damage| {
                damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
            });
            let position = physics
                .as_deref()
                .map_or([0.0; 3], |physics| [physics.x, physics.y, physics.z]);
            let result = state.advance(
                now.0,
                dt,
                position,
                physics.is_some(),
                available,
                operation,
                |action, order| {
                    if staged_power.is_none() {
                        staged_power = power.as_deref().map(|power| power.0.clone());
                    }
                    if staged_inventory.is_none() {
                        staged_inventory =
                            inventory.as_deref().map(|inventory| inventory.0.clone());
                    }
                    work(
                        action,
                        order,
                        owner,
                        staged_power
                            .as_mut()
                            .ok_or("actual Transit Power owner is absent")?,
                        staged_inventory
                            .as_mut()
                            .ok_or("actual Transit fuel owner is absent")?,
                        logistics.ok_or("actual Transit managed fuel binding is absent")?,
                        &pending,
                        &mut flags,
                    )
                },
                |order, _| {
                    if !layers_ready {
                        return None;
                    }
                    let recipes = if let Some(scope) = selected_scope {
                        scope.bodies.clone()
                    } else if order.original_world == TransitWorldBinding::Base
                        && matches!(order.destination, TransitDestination::Local { .. })
                    {
                        accepted_base
                            .as_ref()?
                            .iter()
                            .cloned()
                            .map(|body| (None, body))
                            .collect()
                    } else {
                        return None;
                    };
                    if recipes.iter().any(|(source, body)| {
                        runtime
                            .as_deref()
                            .and_then(|runtime| runtime.name_to_uuid.get(&body.body_id))
                            .or_else(|| world.name_to_uuid.get(&body.body_id))
                            .and_then(|uuid| body_points.get(uuid.as_str()).copied().flatten())
                            .is_none_or(|(point, policy, origin)| {
                                point != body.position
                                    || *policy != body.policy
                                    || source.as_deref() != origin
                            })
                    }) {
                        return None;
                    }
                    placement::solution(
                        order,
                        &uuid.0,
                        owner,
                        recipes.into_iter().map(|(_, body)| body).collect(),
                    )
                },
            );
            match result {
                Ok(result) => {
                    // Keep both Mut dereferences even without staged work: the
                    // eager successful transaction marked these owners Changed.
                    if let (Some(power), Some(next)) = (power.as_deref_mut(), staged_power) {
                        power.0 = next;
                    }
                    if let (Some(inventory), Some(next)) =
                        (inventory.as_deref_mut(), staged_inventory)
                    {
                        inventory.0 = next;
                    }
                    if let TransitAdvance::Arrived { point } = result {
                        if let Some(TransitDestination::World { id, .. }) =
                            state.phase.order().map(|order| &order.destination)
                        {
                            arrived_world = scopes
                                .as_deref()
                                .and_then(|scopes| scopes.routes.get(id))
                                .map(|route| {
                                    (
                                        route.binding.clone(),
                                        TransitWorldArrival {
                                            owner: owner.clone(),
                                            route: id.clone(),
                                            tick: now.0,
                                        },
                                    )
                                });
                        }
                        if let Some(physics) = physics.as_deref_mut() {
                            physics.x = point[0];
                            physics.y = point[1];
                            physics.z = point[2];
                            physics.forward_speed = 0.0;
                            physics.lateral_speed = 0.0;
                            physics.vertical_speed = 0.0;
                        }
                    }
                    if let Some(runtime) = runtime.as_deref_mut() {
                        crate::infrastructure::server::mirror_flags(
                            runtime, &flags, &uuid.0, &None,
                        );
                    }
                    readouts.0.insert(owner.clone(), result);
                }
                Err(error) => warn!("Transit {} refused invalid causal step: {error}", owner.0),
            }
        }
        if let Some((binding, receipt)) = arrived_world {
            drives.1 = binding;
            drives.2 = Some(receipt);
        }
    }
}

pub(super) fn online(sources: &ShipSystemControlSources, owner: &SystemId) -> bool {
    sources.0.entries().any(|(id, _)| id == owner)
        && !sources.0.is_offline(owner)
        && sources.0.source_for(owner) != crate::ship::control_source::ControlSource::Offline
}

/// Exact already-permitted course; no live target/anchor lookup is available.
pub(super) fn course(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    current: &crate::console::navigation::NavigationWaypoint,
    height: f32,
) -> Option<TransitDestination> {
    let binding = transit_composition(&config.system(owner)?.components)?;
    let view = config.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Navigation(board) = view.get(binding.navigation_source)? else {
        return None;
    };
    if current.generation() != board.course_generation || !height.is_finite() {
        return None;
    }
    let point = board.navigation_waypoint.as_ref()?;
    let point = [point.x, height, point.z];
    let mut matching = board.transit_routes.iter().filter(|route| {
        route.point[0] == point[0] && route.point[2] == point[2] && route.validate().is_ok()
    });
    let matched = matching.next();
    if matching.next().is_some() {
        return None;
    }
    let destination = if let Some(route) = matched {
        if !binding.config.world_travel {
            return None;
        }
        TransitDestination::World {
            id: route.id.clone(),
            point: route.point,
            course_generation: board.course_generation,
        }
    } else {
        TransitDestination::Local {
            point,
            course_generation: board.course_generation,
        }
    };
    destination.validate().ok()?;
    Some(destination)
}

fn visible_request(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    key: &TransitRequestKey,
) -> bool {
    if key.validate().is_err() {
        return false;
    }
    let view = config.observations_for_system(owner, &boards.0);
    let Some(SystemBlackboard::Viewscreen(board)) =
        view.get(&crate::ship::system_registry::viewscreen_system_id())
    else {
        return false;
    };
    board.scored_objectives.iter().any(|row| row.id == key.objective_id && row.score > 0.0
        && !row.snapshot.unassigned && row.relevance.contains(&crate::core::messages::SystemAffinity::Helm)
        && matches!(row.directive, AiDirective::Transit { revision } if revision == key.revision))
}

pub(crate) fn handle_commands(
    tick: Res<crate::sim_tick::SimTick>,
    scopes: Option<Res<StaticTransitScopes>>,
    mut outbox: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
    mut ships: Query<(
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemBlackboards,
        &ShipSystemControlSources,
        Option<&ShipPowerSystem>,
        Option<&ShipPhysics>,
        Option<&crate::console::navigation::NavigationWaypoint>,
        &mut ShipTransitDrives,
        (
            Option<&crate::resources::fitting_server::CompiledShipFitting>,
            Option<&crate::resources::fitting_server::ShipFitting>,
        ),
    )>,
) {
    for (commands, config, boards, sources, power, physics, waypoint, mut drives, fitting) in
        &mut ships
    {
        for command in &commands.0 {
            let Some(system) = config
                .0
                .system(&command.target)
                .filter(|system| system.kind == crate::ship::system_registry::TRANSIT_DRIVE_KIND)
            else {
                continue;
            };
            let Some(binding) = transit_composition(&system.components) else {
                continue;
            };
            if !matches!(
                command.payload,
                Control::BeginTransitToWaypoint { .. } | Control::CancelTransit
            ) {
                continue;
            }
            let unavailable = match fitting {
                (Some(profile), Some(state)) => {
                    crate::resources::fitting_server::unavailable(profile, state, &command.target)
                }
                (None, None) => false,
                _ => true,
            };
            let competing = drives
                .0
                .iter()
                .any(|(owner, state)| owner != &command.target && state.phase.active());
            let original_world = drives.1.clone();
            let current_valid = scopes
                .as_deref()
                .map_or(original_world == TransitWorldBinding::Base, |scopes| {
                    scopes.validate_context(&drives, tick.0).is_ok()
                });
            let applied = if !current_valid || !online(sources, &command.target) {
                false
            } else if let Some(state) = drives.0.get_mut(&command.target) {
                match &command.payload {
                    Control::CancelTransit => state.cancel(tick.0).is_ok(),
                    Control::BeginTransitToWaypoint { request } if !unavailable && !competing => {
                        let order = physics.zip(waypoint).and_then(|(physics, waypoint)| {
                            let destination =
                                course(&config.0, boards, &command.target, waypoint, physics.y)?;
                            if !known_route(&destination, scopes.as_deref()) {
                                return None;
                            }
                            if request.as_ref().is_some_and(|key| {
                                !visible_request(&config.0, boards, &command.target, key)
                            }) {
                                return None;
                            }
                            let consumer = power?
                                .0
                                .config()
                                .consumers
                                .iter()
                                .find(|consumer| consumer.system == command.target)?;
                            Some(TransitOrder {
                                destination,
                                original_world: original_world.clone(),
                                origin: [physics.x, physics.y, physics.z],
                                profile: AcceptedLocalJumpProfile {
                                    config: binding.config.clone(),
                                    preparation_energy_per_sec: consumer.continuous_cost_per_sec,
                                    commit_energy: consumer.discrete_cost,
                                },
                                accepted_tick: tick.0,
                                request: request.clone(),
                            })
                        });
                        order.is_some_and(|order| state.begin(order, tick.0).is_ok())
                    }
                    _ => false,
                }
            } else {
                false
            };
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbox,
                command,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

/// A same-step claim is staged on the existing capacity and energy owners. No
/// claim, fuel count or accepted credit is retained outside this transaction.
fn work(
    action: TransitWork,
    order: &TransitOrder,
    owner: &SystemId,
    power: &mut phoenix_sim_gameplay::modifiers::power_system::PowerSystem,
    inventory: &mut crate::infrastructure::InfrastructureState,
    logistics: &crate::resources::logistics_server::CompiledShipLogistics,
    pending: &BTreeMap<String, i64>,
    flags: &mut Vec<crate::infrastructure::FlagChange>,
) -> Result<TransitGrant, String> {
    match action {
        TransitWork::Prepare { seconds } => {
            let amount = seconds * f64::from(order.profile.preparation_energy_per_sec);
            if !amount.is_finite() || amount > f64::from(f32::MAX) {
                return Err("accepted preparation work is not representable".into());
            }
            let grant = power.spend_continuous(owner, amount as f32)?;
            Ok(TransitGrant::Preparation {
                seconds: (f64::from(grant.granted)
                    / f64::from(order.profile.preparation_energy_per_sec))
                .min(seconds),
            })
        }
        TransitWork::Commit => {
            let fuel = &order.profile.config.fuel;
            let store = logistics.0.store(&fuel.store)?;
            if store.role != phoenix_model::logistics::StoreRole::Local
                || store.system.as_ref() != Some(owner)
            {
                return Err("accepted fuel lacks actual drive-local ownership".into());
            }
            let phoenix_model::logistics::ResourceEndpoint::Capacity { id } = &store.endpoint
            else {
                return Err("accepted fuel lacks actual capacity endpoint".into());
            };
            let mut stock = inventory
                .capacity_stock(id)
                .cloned()
                .ok_or("actual fuel stock is unavailable")?;
            stock.validate()?;
            if stock
                .available()
                .saturating_sub(pending.get(id).copied().unwrap_or(0))
                < fuel.amount
            {
                return Ok(TransitGrant::Commitment { paid: false });
            }
            let claim = format!("transit:{}:{}", owner.0, order.accepted_tick);
            if stock.reserved.contains_key(&claim) {
                return Err("accepted transit claim collides with another owner".into());
            }
            stock.reserve(&claim, fuel.amount)?;
            let mut staged_power = power.clone();
            if !staged_power.try_work_discrete_amount(owner, order.profile.commit_energy)? {
                return Ok(TransitGrant::Commitment { paid: false });
            }
            stock.consume(&claim);
            let changes = inventory.replace_capacity_stock(id, stock)?;
            *power = staged_power;
            flags.extend(changes);
            Ok(TransitGrant::Commitment { paid: true })
        }
    }
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;

fn known_route(destination: &TransitDestination, scopes: Option<&StaticTransitScopes>) -> bool {
    match destination {
        TransitDestination::Local { .. } => true,
        TransitDestination::World { id, point, .. } => scopes
            .and_then(|scopes| scopes.routes.get(id))
            .is_some_and(|route| route.view.point == *point),
    }
}

/// Explicit real authored-layer spawn join; never invoked during a restore.
/// EntityOriginLayer retains lifetime/script ownership independently.
pub(crate) fn seed_layer_origin(commands: &mut EntityCommands, path: &str) {
    let path = path.to_owned();
    commands.queue(move |mut entity: EntityWorldMut| {
        if !entity.contains::<ShipTransitDrives>() {
            return;
        }
        let binding = entity.world_scope(|world| {
            world
                .get_resource::<StaticTransitScopes>()
                .ok_or_else(|| "actual Transit World recipes are absent".to_owned())
                .and_then(|scopes| scopes.binding_for_origin(&path))
        });
        match binding {
            Ok(binding) => {
                entity
                    .get_mut::<ShipTransitDrives>()
                    .expect("checked actual drive")
                    .1 = binding;
            }
            Err(error) => {
                // Unsupported binding is explicit and cannot grant base policy.
                bevy::log::error!(
                    "refused layer-authored Transit World binding for {path}: {error}"
                );
                entity
                    .get_mut::<ShipTransitDrives>()
                    .expect("checked actual drive")
                    .1 = TransitWorldBinding::Layer { path };
            }
        }
    });
}
