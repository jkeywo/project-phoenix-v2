//! Fitted logistics: ordinary System commands, permitted reports and real stocks.
use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload as Control};
use crate::infrastructure::{FlagChange, InfrastructureCondition};
use crate::resources::logistics::{
    apply_stock_action, CompiledLogistics, EndpointStock, LogisticsState, LogisticsStockAction,
    ResourceEndpoint,
};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::prelude::*;
use phoenix_model::logistics::StoreRole;
use phoenix_model::logistics_ui::*;
use std::collections::BTreeMap;

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    app.declare_state::<CompiledShipLogistics>(
        StateClass::Derived,
        "located-delivery-batches-and-receiving-claims",
    )
    .declare_state::<ShipLogistics>(
        StateClass::Folded,
        "located-delivery-batches-and-receiving-claims",
    )
    .register_admitted_consumer(
        ConsumerMatcher::kind(crate::ship::system_registry::LOGISTICS_KIND).with_feedback(
            FeedbackAddress::DeclaredKindOrCanonical("logistics"),
            &[
                Payload::SetLogisticsEnabled,
                Payload::SetStockTarget,
                Payload::SetStockReserve,
                Payload::SetStockPriority,
                Payload::DispatchDelivery,
                Payload::ReturnDelivery,
            ],
        ),
    );
    // Stock goals are the magazine's sole admitted capability; other equipment
    // retains its machinery consumer alongside the shared payload-only handler.
    app.register_admitted_consumer(
        ConsumerMatcher::kind(crate::ship::system_registry::TORPEDO_MAGAZINE_KIND).with_feedback(
            FeedbackAddress::DeclaredKindOrCanonical("torpedo-magazine"),
            &[Payload::SetStockTarget],
        ),
    );
    for kind in crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("built-in registry")
        .kinds()
        .filter(|kind| {
            ![
                crate::ship::system_registry::LOGISTICS_KIND,
                crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
            ]
            .contains(kind)
        })
    {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(kind, Payload::SetStockTarget).with_feedback(
                FeedbackAddress::DeclaredKindOnly,
                &[Payload::SetStockTarget],
            ),
        );
    }
    app.add_systems(
        FixedUpdate,
        refresh_replenishment
            .in_set(crate::sim_sets::SimSet::Input)
            .after(crate::sim_sets::FixedStep::ApplyConsoleViews)
            .before(operate_ai),
    );
    app.add_systems(
        FixedUpdate,
        operate_ai
            .in_set(crate::sim_sets::SimSet::Input)
            .after(crate::sim_sets::FixedStep::ApplyConsoleViews)
            .run_if(crate::ai::cadence::ai_tick_ready),
    );
    app.add_systems(
        FixedUpdate,
        (tick, refuse_unmanaged_stock_controls)
            .chain()
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::infrastructure::server::tick_infrastructure_condition),
    );
    app.add_systems(
        FixedUpdate,
        (publish_local_stocks, publish)
            .chain()
            .in_set(crate::sim_sets::SimSet::Publish)
            .after(crate::console::weapons::blackboard::publish_torpedo_magazine_blackboard),
    );
}

#[derive(Component, Clone)]
pub struct CompiledShipLogistics(pub CompiledLogistics);

#[derive(Component, Clone, Debug)]
pub struct ShipLogistics(pub LogisticsState);

// Other shipped magazines still expose the same admitted System boundary, but
// only fitted logistics owns a stock goal. Complete a correlated refusal rather
// than advertising support and silently dropping an unsupported local setting.
fn refuse_unmanaged_stock_controls(
    ships: Query<&AdmittedCommands, Without<ShipLogistics>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for admitted in &ships {
        for command in &admitted.0 {
            if matches!(command.payload, Control::SetStockTarget { .. }) {
                crate::command_admission::finish_admitted_action_feedback(
                    &mut outbound,
                    command,
                    crate::core::messages::ActionFeedbackOutcome::Refused,
                );
            }
        }
    }
}

/// Install from the same validated definition for players, NPCs and restores.
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = crate::resources::logistics_profile::compile_entity_logistics(config)
        .expect("validated logistics profile")
    {
        let state = LogisticsState::new(&profile);
        commands.insert((CompiledShipLogistics(profile), ShipLogistics(state)));
    }
}

/// Temporary transaction projection. Canonical quantities remain with their
/// existing Infrastructure and magazine owners, including tube commitments.
pub fn endpoint_view(
    profile: &CompiledLogistics,
    inventory: Option<&InfrastructureCondition>,
    torpedoes: Option<&TorpedoSystemResource>,
) -> Result<BTreeMap<ResourceEndpoint, EndpointStock>, String> {
    profile
        .config()
        .stores
        .iter()
        .map(|store| {
            let (stock, held) =
                crate::resources::server::endpoint_stock(&store.endpoint, inventory, torpedoes)
                    .ok_or("managed stock owner is unavailable")?;
            Ok((store.endpoint.clone(), EndpointStock { stock, held }))
        })
        .collect()
}

/// Validate every changed owner before publishing any of them. Returned flag
/// edges and capacity levels are mirrored by the normal infrastructure adapter.
fn transact(
    profile: &CompiledLogistics,
    action: &LogisticsStockAction,
    inventory: Option<&mut InfrastructureCondition>,
    torpedoes: Option<&mut TorpedoSystemResource>,
) -> Result<(i64, Vec<FlagChange>), String> {
    let mut stocks = endpoint_view(profile, inventory.as_deref(), torpedoes.as_deref())?;
    let moved = apply_stock_action(profile, action, &mut stocks)?;
    let flags = replace_endpoints(profile, stocks, inventory, torpedoes)?;
    Ok((moved, flags))
}

/// Commit a validated temporary projection for delivery or finite fabrication.
pub(crate) fn replace_endpoints(
    profile: &CompiledLogistics,
    stocks: BTreeMap<ResourceEndpoint, EndpointStock>,
    inventory: Option<&mut InfrastructureCondition>,
    torpedoes: Option<&mut TorpedoSystemResource>,
) -> Result<Vec<FlagChange>, String> {
    profile.validate_stocks(&stocks)?;
    let mut next_inventory = inventory.as_deref().map(|owner| owner.0.clone());
    let mut next_torpedoes = torpedoes.as_deref().map(|owner| owner.0.clone());
    let mut flags = Vec::new();
    for (endpoint, value) in stocks {
        match endpoint {
            ResourceEndpoint::Capacity { id } => {
                flags.extend(
                    next_inventory
                        .as_mut()
                        .ok_or("missing capacity owner")?
                        .replace_capacity_stock(&id, value.stock)?,
                );
            }
            ResourceEndpoint::TorpedoMagazine {} => {
                next_torpedoes
                    .as_mut()
                    .ok_or("missing magazine owner")?
                    .magazine = value.stock;
            }
        }
    }
    if let (Some(owner), Some(next)) = (inventory, next_inventory) {
        owner.0 = next;
    }
    if let (Some(owner), Some(next)) = (torpedoes, next_torpedoes) {
        owner.0 = next;
    }
    Ok(flags)
}

/// Execute a maintained/local order against the current real deficit. Both
/// direct requests and controller-maintained requests use this same boundary;
/// a stale report or another request this tick cannot reserve the deficit twice.
fn dispatch_order(
    profile: &CompiledLogistics,
    state: &mut LogisticsState,
    tick: u64,
    request: &DeliveryRequest,
    mut inventory: Option<&mut InfrastructureCondition>,
    mut torpedoes: Option<&mut TorpedoSystemResource>,
    flags: &mut Vec<FlagChange>,
) -> Result<phoenix_model::job::JobId, String> {
    let stocks = endpoint_view(profile, inventory.as_deref(), torpedoes.as_deref())?;
    let destination = profile.store(&request.destination)?;
    let current = &stocks[&destination.endpoint];
    let occupied = current
        .stock
        .level
        .checked_add(current.stock.incoming_total())
        .and_then(|amount| amount.checked_add(current.held))
        .ok_or("local occupancy overflows")?;
    let deficit = state.goals[&destination.id].saturating_sub(occupied).max(0);
    let amount = request.amount.min(deficit);
    if amount <= 0 {
        return Err("local stock target has no remaining deficit".into());
    }
    state.dispatch(
        profile,
        tick,
        &request.source,
        &request.destination,
        amount,
        |action| {
            let (quantity, edges) = transact(
                profile,
                action,
                inventory.as_deref_mut(),
                torpedoes.as_deref_mut(),
            )?;
            flags.extend(edges);
            Ok(quantity)
        },
    )
}

/// Additional load room after all capacity and shared-compartment commitments.
pub fn receiving_room(
    profile: Option<&CompiledShipLogistics>,
    endpoint: &ResourceEndpoint,
    stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
) -> Result<i64, String> {
    let Some(profile) = profile else {
        return Ok(i64::MAX);
    };
    let Some(store) = profile
        .0
        .config()
        .stores
        .iter()
        .find(|store| &store.endpoint == endpoint)
    else {
        return Ok(i64::MAX);
    };
    profile.0.receiving_headroom(stocks, &store.id)
}

fn refresh_replenishment(
    mut ships: Query<(
        &CompiledShipLogistics,
        &mut ShipLogistics,
        &ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (profile, mut state, config, boards) in &mut ships {
        let permitted = config.0.observations_for_system(&state.0.owner, &boards.0);
        let Some(SystemBlackboard::Logistics(board)) = permitted.get(&state.0.owner) else {
            continue;
        };
        let observations = board
            .stores
            .iter()
            .filter(|store| store.role == "local")
            .filter_map(|store| {
                Some(crate::resources::logistics::ReplenishmentObservation {
                    store: store.id.clone(),
                    occupied: store
                        .level
                        .checked_add(store.incoming)?
                        .checked_add(store.held)?,
                    target: store.target,
                    priority: store.priority,
                    replenishing: state.0.replenishing.contains(&store.id),
                })
            })
            .collect::<Vec<_>>();
        if let Err(error) = state.0.refresh_replenishment(&profile.0, &observations) {
            warn!(%error, "Invalid permitted stock demand");
        }
    }
}

pub(crate) fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&crate::entities::spawner::EntityUuid>,
        &ShipSystemControlSources,
        &ShipConfigComponent,
        &ShipLogistics,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, sources, config, logistics, boards, programs, mut runtimes, mut admitted) in
        &mut ships
    {
        let owner = &logistics.0.owner;
        if !crate::ai::host::ai_operates(&sources.0, owner) {
            continue;
        }
        let permitted = config.0.observations_for_system(owner, &boards.0);
        let Some(SystemBlackboard::Logistics(board)) = permitted.get(owner) else {
            continue;
        };
        let Some(runtime) = runtimes.0.get_mut(owner) else {
            continue;
        };
        let facts = BTreeMap::from([
            ("enabled".into(), board.enabled),
            ("delivery_available".into(), board.next_delivery.is_some()),
        ]);
        let channels = BTreeMap::from([("dispatch".into(), false)]);
        let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
            &sources.0,
            owner,
            programs.0.get(owner),
            &facts,
            &channels,
            runtime,
            tick.0,
        ) else {
            continue;
        };
        if result.intents.get("dispatch") == Some(&true) {
            if let Some(request) = &board.next_delivery {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    owner.clone(),
                    Control::DispatchDelivery {
                        source: request.source.clone(),
                        destination: request.destination.clone(),
                        amount: board
                            .planned_requests
                            .iter()
                            .find(|planned| {
                                planned.source == request.source
                                    && planned.destination == request.destination
                                    && planned.amount <= request.amount
                            })
                            .map_or(request.amount, |planned| planned.amount),
                    },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    tick: Res<crate::sim_tick::SimTick>,
    time: Res<Time>,
    mut runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    base_world: Option<Res<crate::world::config::WorldConfig>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
    mut ships: Query<(
        &crate::entities::spawner::EntityUuid,
        Option<&crate::ship_slots::AuthoredShipSlotId>,
        &CompiledShipLogistics,
        &mut ShipLogistics,
        &AdmittedCommands,
        &ShipSystemControlSources,
        &ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        Option<&crate::ship::access::InteriorAccess>,
        Option<&mut InfrastructureCondition>,
        Option<&mut TorpedoSystemResource>,
        Option<&mut crate::ship::power::ShipPowerSystem>,
    )>,
) {
    let mut order: Vec<_> = ships.iter().map(|(uuid, ..)| uuid.0.clone()).collect();
    order.sort();
    for uuid in order {
        let Some((
            _,
            slot,
            profile,
            mut owner,
            admitted,
            sources,
            config,
            boards,
            damage,
            access,
            mut inventory,
            mut torpedoes,
            mut power,
        )) = ships.iter_mut().find(|(id, ..)| id.0 == uuid)
        else {
            continue;
        };
        if let (Some(runtime), Some(base)) = (runtime.as_deref_mut(), base_world.as_deref()) {
            crate::world::activities::arm_for_actor(
                runtime,
                base,
                &uuid,
                slot.map(|id| id.0.as_str()),
                tick.0,
                |binding| crate::world::activities::delivery_binding_supported(binding, &profile.0),
            );
        }
        let mut accepted = Vec::new();
        let mut next = owner.0.clone();
        let prior_batches = next.batches.clone();
        let mut next_inventory = inventory.as_deref().cloned();
        let mut next_torpedoes = torpedoes.as_deref().cloned();
        let mut next_power = power.as_deref().cloned();
        let mut flags = Vec::new();
        let blocked = access
            .map(|access| access.blocked_connections(&profile.0.graph().config))
            .unwrap_or_default();
        let factor = damage.map_or(1.0, |damage| {
            damage.factor(&next.owner, phoenix_model::damage::DamageBinding::Operation)
        });
        let policy = sources.0.policy_for(&next.owner);
        let offline = !sources.0.entries().any(|(id, _)| id == &next.owner)
            || !(policy.accept_human_input || policy.operate_ai);
        // Return intent is applied before advancing, so a cancellation can never
        // deliver to the destination during the same tick it was recalled.
        let mut recalls = BTreeMap::new();
        let control_owner = next.owner.clone();
        for command in admitted.for_target(&control_owner.0) {
            if let Control::ReturnDelivery { job } = &command.payload {
                recalls.insert(
                    *job,
                    next.request_return(phoenix_model::job::JobId(*job)).is_ok(),
                );
            }
            if let Control::SetLogisticsEnabled { enabled } = &command.payload {
                next.set_enabled(*enabled);
            }
        }
        let advanced = next.tick(
            &profile.0,
            tick.0,
            time.delta_secs(),
            &blocked,
            |action| {
                let (quantity, edges) = transact(
                    &profile.0,
                    action,
                    next_inventory.as_mut(),
                    next_torpedoes.as_mut(),
                )?;
                flags.extend(edges);
                Ok(quantity)
            },
            |system, dt| {
                if offline || factor <= 0.0 {
                    return Ok(0.0);
                }
                next_power.as_mut().map_or(Ok(0.0), |power| {
                    power
                        .0
                        .work_continuous(system, dt, 1.0)
                        .map(|grant| grant.fraction * factor)
                })
            },
        );
        let completed = match advanced {
            Ok(result) => result.completed,
            Err(error) => {
                warn!(%uuid, %error, "Logistics step refused; staged stock and energy discarded");
                continue;
            }
        };
        if offline || factor <= 0.0 {
            next.blocked = next
                .batches
                .keys()
                .map(|id| (*id, crate::resources::logistics::DeliveryBlock::Disabled))
                .collect();
        }
        // New requests are committed after existing work. A newly dispatched
        // batch cannot advance in the same tick and no queued request owns stock.
        for command in &admitted.0 {
            let target = &command.target;
            let result = match &command.payload {
                Control::SetLogisticsEnabled { enabled } if target == &next.owner => {
                    next.set_enabled(*enabled);
                    Ok(())
                }
                Control::SetStockTarget { store, amount } => {
                    profile.0.store(store).and_then(|binding| {
                        let expected = binding.system.as_ref().unwrap_or(&next.owner);
                        if target != expected {
                            return Err("stock target belongs to another System".into());
                        }
                        next.set_target(&profile.0, store, *amount)
                    })
                }
                Control::SetStockPriority { store, priority } if target == &next.owner => {
                    next.set_priority(&profile.0, store, *priority)
                }
                Control::SetStockReserve { store, amount } if target == &next.owner => {
                    profile.0.store(store).and_then(|binding| {
                        if binding.role != StoreRole::Main {
                            return Err("local stores do not own main reserve floors".into());
                        }
                        next.set_reserve(&profile.0, store, *amount)
                    })
                }
                Control::DispatchDelivery {
                    source,
                    destination,
                    amount,
                } if target == &next.owner => {
                    if offline || factor <= 0.0 {
                        Err("handling machinery unavailable".into())
                    } else {
                        dispatch_order(
                            &profile.0,
                            &mut next,
                            tick.0,
                            &DeliveryRequest {
                                source: source.clone(),
                                destination: destination.clone(),
                                amount: *amount,
                            },
                            next_inventory.as_mut(),
                            next_torpedoes.as_mut(),
                            &mut flags,
                        )
                        .map(|job| {
                            accepted.push(job);
                        })
                    }
                }
                Control::ReturnDelivery { job } if target == &next.owner => {
                    if recalls.get(job) == Some(&true) {
                        Ok(())
                    } else {
                        Err("unknown active delivery".into())
                    }
                }
                _ => continue,
            };
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                command,
                if result.is_ok() {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
        // A Human's enabled stock targets are standing machinery orders. AI
        // controllers instead supply their own ordinary authored tree intent.
        // Missing/Offline/Simplified sources must never select this Human path.
        let human = sources.0.entries().any(|(id, source)| {
            id == &next.owner && *source == crate::ship::control_source::ControlSource::Human
        });
        if human && !offline && factor > 0.0 && next.enabled {
            let permitted = config.0.observations_for_system(&next.owner, &boards.0);
            if let Some(SystemBlackboard::Logistics(board)) = permitted.get(&next.owner) {
                if let Some(request) = &board.next_delivery {
                    if let Ok(job) = dispatch_order(
                        &profile.0,
                        &mut next,
                        tick.0,
                        request,
                        next_inventory.as_mut(),
                        next_torpedoes.as_mut(),
                        &mut flags,
                    ) {
                        accepted.push(job);
                    }
                }
            }
        }
        owner.0 = next;
        if let (Some(owner), Some(next)) = (inventory.as_deref_mut(), next_inventory) {
            *owner = next;
        }
        if let (Some(owner), Some(next)) = (torpedoes.as_deref_mut(), next_torpedoes) {
            *owner = next;
        }
        if let (Some(owner), Some(next)) = (power.as_deref_mut(), next_power) {
            *owner = next;
        }
        if let Some(runtime) = runtime.as_deref_mut() {
            for job in accepted {
                if let Some(batch) = owner.0.batches.get(&job) {
                    if let Ok(destination) = profile.0.store(&batch.original_destination) {
                        if let Err(error) = runtime.situations.accept_delivery(
                            &uuid,
                            &owner.0.owner,
                            &batch.source,
                            &batch.original_destination,
                            &destination.resource,
                            batch.amount,
                            job,
                            tick.0,
                        ) {
                            warn!(%uuid, %error, "Accepted delivery could not bind its activity");
                        }
                    }
                }
            }
            let outcomes = completed.into_iter().filter_map(|outcome| {
                let batch = prior_batches.get(&outcome.id)?;
                let destination = profile.0.store(&batch.original_destination).ok()?;
                Some((
                    outcome.id,
                    phoenix_model::activity::ActivityOutcome::Delivery {
                        source: batch.source.clone(),
                        destination: outcome.destination,
                        commodity: destination.resource.clone(),
                        amount: outcome.amount,
                        returned: outcome.returned,
                        outcome: outcome.outcome,
                    },
                ))
            });
            crate::world::activities::record(runtime, &uuid, &owner.0.owner, tick.0, outcomes);
            if let Some(inventory) = inventory.as_deref() {
                for capacity in inventory.0.capacities() {
                    runtime.flags.set_flag_value(&capacity.id, capacity.level);
                }
            }
            crate::infrastructure::server::mirror_flags(runtime, &flags, &uuid, &None);
        }
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn publish(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    base_world: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<(
        Option<&crate::entities::spawner::EntityUuid>,
        &CompiledShipLogistics,
        &ShipLogistics,
        &ShipConfigComponent,
        Option<&InfrastructureCondition>,
        Option<&TorpedoSystemResource>,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (actor, profile, state, config, inventory, torpedoes, mut boards) in &mut ships {
        let Ok(physical_stores) = endpoint_view(&profile.0, inventory, torpedoes) else {
            continue;
        };
        let permitted = config.0.observations_for_system(&state.0.owner, &boards.0);
        let mut stores = Vec::new();
        for binding in &profile.0.config().stores {
            let (level, available, incoming, held, target, headroom, bay_free_space) =
                match (&binding.role, &binding.endpoint) {
                    (StoreRole::Main, ResourceEndpoint::Capacity { id }) => {
                        let Some(stock) = inventory.and_then(|owner| owner.0.capacity_stock(id))
                        else {
                            continue;
                        };
                        (
                            stock.level,
                            stock.available(),
                            stock.incoming_total(),
                            0,
                            state.0.goals[&binding.id],
                            profile
                                .0
                                .receiving_headroom(&physical_stores, &binding.id)
                                .unwrap_or(0),
                            observed_bay_space(&profile.0, &physical_stores, &binding.compartment),
                        )
                    }
                    (StoreRole::Local, _) => {
                        let publication = stock_publication(
                            binding.system.as_ref().expect("compiled local owner"),
                        );
                        let Some(SystemBlackboard::Stock(board)) = permitted.get(&publication)
                        else {
                            continue;
                        };
                        let Some(local) = board.stores.iter().find(|local| local.id == binding.id)
                        else {
                            continue;
                        };
                        (
                            local.level,
                            local.available,
                            local.incoming,
                            local.held,
                            local.target,
                            local.headroom,
                            local.bay_free_space,
                        )
                    }
                    _ => continue,
                };
            stores.push(LogisticsStoreView {
                id: binding.id.clone(),
                resource: binding.resource.clone(),
                role: if binding.role == StoreRole::Main {
                    "main"
                } else {
                    "local"
                }
                .into(),
                system: binding.system.clone(),
                level,
                available,
                held,
                ceiling: profile.0.endpoints()[&binding.endpoint].ceiling,
                headroom,
                bay_free_space,
                incoming,
                target,
                reserve: state.0.floors[&binding.id],
                priority: state.0.priorities[&binding.id],
                compartment: binding.compartment.clone(),
            });
        }
        let next_delivery = if state.0.enabled
            && state.0.batches.len() < usize::from(profile.0.handling().max_active)
        {
            let observations = stores
                .iter()
                .filter(|s| s.role == "local")
                .filter_map(|s| {
                    Some(crate::resources::logistics::ReplenishmentObservation {
                        store: s.id.clone(),
                        occupied: s.level.checked_add(s.incoming)?.checked_add(s.held)?,
                        target: s.target,
                        priority: s.priority,
                        replenishing: state.0.replenishing.contains(&s.id),
                    })
                })
                .collect::<Vec<_>>();
            crate::resources::logistics::ordered_replenishment_candidates(
                &observations,
                profile.0.handling(),
            )
            .unwrap_or_default()
            .iter()
            .find_map(|candidate| {
                let local = stores.iter().find(|store| store.id == candidate.store)?;
                let binding = profile.0.store(&local.id).ok()?;
                let source = stores
                    .iter()
                    .find(|source| Some(&source.id) == binding.source.as_ref())?;
                let amount = (candidate.target - candidate.occupied)
                    .min(source.available)
                    .min(local.headroom)
                    .min(source.level.saturating_sub(source.reserve).max(0))
                    .min(profile.0.handling().max_batch);
                (amount > 0).then(|| DeliveryRequest {
                    source: source.id.clone(),
                    destination: local.id.clone(),
                    amount,
                })
            })
        } else {
            None
        };
        let deliveries = state
            .0
            .batches
            .values()
            .map(|batch| {
                use phoenix_model::interior::RouteLocation;
                let compartment = match &batch.navigation.location {
                    RouteLocation::Compartment { compartment } => compartment.clone(),
                    RouteLocation::Crossing { to, .. } => to.clone(),
                    RouteLocation::External { .. } => String::new(),
                };
                let duration = batch.amount as f32 / profile.0.handling().units_per_sec;
                DeliveryView {
                    id: batch.id.0,
                    source: batch.source.clone(),
                    destination: batch.destination.clone(),
                    quantity: batch.amount,
                    stage: match batch.stage {
                        crate::resources::logistics::DeliveryStage::Loading => "loading",
                        crate::resources::logistics::DeliveryStage::Travelling => "travelling",
                        crate::resources::logistics::DeliveryStage::Unloading => "unloading",
                    }
                    .into(),
                    compartment,
                    progress: if duration > 0.0 {
                        batch.work_progress / duration
                    } else {
                        0.0
                    },
                    return_requested: batch.return_requested,
                    blocked: state
                        .0
                        .blocked
                        .get(&batch.id)
                        .map(|reason| blocked_key(*reason).to_owned()),
                }
            })
            .collect();
        boards.0.insert(
            state.0.owner.clone(),
            SystemBlackboard::Logistics(LogisticsBlackboard {
                planned_requests: actor
                    .map(|actor| {
                        crate::world::planning::delivery_requests(
                            runtime.as_deref(),
                            base_world.as_deref(),
                            &actor.0,
                            &state.0.owner,
                            tick.as_ref().map_or(0, |tick| tick.0),
                        )
                    })
                    .unwrap_or_default(),
                enabled: state.0.enabled,
                max_batch: profile.0.handling().max_batch,
                max_active: profile.0.handling().max_active,
                stores,
                deliveries,
                next_delivery,
                refusal: state
                    .0
                    .blocked
                    .values()
                    .next()
                    .map(|reason| blocked_key(*reason).to_owned()),
            }),
        );
    }
}

/// Each local owner publishes only its real readings. Inventory consumes explicit feeds.
#[allow(clippy::type_complexity)]
fn publish_local_stocks(
    mut ships: Query<(
        &CompiledShipLogistics,
        &ShipLogistics,
        Option<&InfrastructureCondition>,
        Option<&TorpedoSystemResource>,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (profile, state, inventory, torpedoes, mut boards) in &mut ships {
        let Ok(stocks) = endpoint_view(&profile.0, inventory, torpedoes) else {
            continue;
        };
        let mut owners = BTreeMap::<crate::core::messages::SystemId, Vec<LocalStockView>>::new();
        for binding in &profile.0.config().stores {
            let Some(owner) = &binding.system else {
                continue;
            };
            if binding.role != StoreRole::Local {
                continue;
            }
            let value = &stocks[&binding.endpoint];
            owners
                .entry(owner.clone())
                .or_default()
                .push(LocalStockView {
                    id: binding.id.clone(),
                    resource: binding.resource.clone(),
                    level: value.stock.level,
                    available: value.stock.available(),
                    ceiling: value.stock.ceiling,
                    headroom: profile
                        .0
                        .receiving_headroom(&stocks, &binding.id)
                        .unwrap_or(0),
                    bay_free_space: observed_bay_space(&profile.0, &stocks, &binding.compartment),
                    incoming: value.stock.incoming_total(),
                    held: value.held,
                    target: state.0.goals[&binding.id],
                    compartment: binding.compartment.clone(),
                });
        }
        for (owner, stores) in owners {
            boards.0.insert(
                stock_publication(&owner),
                SystemBlackboard::Stock(SystemStockBlackboard {
                    system: owner,
                    stores,
                }),
            );
        }
    }
}

fn observed_bay_space(
    profile: &CompiledLogistics,
    stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
    compartment: &str,
) -> i64 {
    profile
        .config()
        .storage
        .iter()
        .find(|bay| bay.compartment == compartment)
        .and_then(|bay| {
            profile
                .occupied_space(stocks, compartment)
                .ok()
                .map(|used| bay.space.saturating_sub(used).max(0))
        })
        .unwrap_or(0)
}

fn blocked_key(reason: crate::resources::logistics::DeliveryBlock) -> &'static str {
    use crate::resources::logistics::DeliveryBlock;
    match reason {
        DeliveryBlock::Disabled => "logistics.blocked.disabled",
        DeliveryBlock::Access => "logistics.blocked.access",
        DeliveryBlock::ReturnCapacity => "logistics.blocked.return_capacity",
        DeliveryBlock::Receiving => "logistics.blocked.receiving",
    }
}

#[cfg(test)]
#[path = "logistics_server_tests.rs"]
pub(crate) mod tests;
