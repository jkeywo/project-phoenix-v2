//! Ordinary fitted fabrication over the same actual stocks, energy and Console boundary.
use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload as Control};
use crate::infrastructure::InfrastructureCondition;
use crate::resources::logistics_server::{endpoint_view, replace_endpoints, ShipLogistics};
use crate::resources::production::*;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::prelude::*;
use phoenix_model::production_ui::*;
use std::collections::BTreeMap;

#[derive(Component, Clone)]
pub struct CompiledShipProduction(pub CompiledProduction);
#[derive(Component, Clone, Debug)]
pub struct ShipProduction(pub ProductionState);

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    app.declare_state::<CompiledShipProduction>(
        StateClass::Derived,
        "finite-production-with-coupled-work-requirements",
    )
    .declare_state::<ShipProduction>(
        StateClass::Folded,
        "finite-production-with-coupled-work-requirements",
    )
    .register_admitted_consumer(ConsumerMatcher::kind(FABRICATION_KIND).with_feedback(
        FeedbackAddress::DeclaredKindOnly,
        &[
            Payload::SetProductionEnabled,
            Payload::SetProductionTarget,
            Payload::SelectProductionRecipe,
            Payload::StartProduction,
        ],
    ));
    app.add_systems(
        FixedUpdate,
        operate_ai
            .in_set(crate::sim_sets::SimSet::Input)
            .after(crate::resources::logistics_server::operate_ai)
            .run_if(crate::ai::cadence::ai_tick_ready),
    );
    app.add_systems(
        FixedUpdate,
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::resources::logistics_server::tick),
    );
    app.add_systems(
        FixedUpdate,
        publish
            .in_set(crate::sim_sets::SimSet::Publish)
            .after(crate::resources::logistics_server::publish),
    );
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = crate::resources::production_profile::compile_entity_production(config)
        .expect("validated production")
    {
        let state = ProductionState::new(&profile);
        commands.insert((CompiledShipProduction(profile), ShipProduction(state)));
    }
}

/// Reconstruct a temporary planning projection strictly from the permitted report.
/// Claim names are synthetic because only their reported totals are known; this
/// projection is never committed or used as execution stock.
fn observed_inputs(
    profile: &CompiledProduction,
    board: &phoenix_model::logistics_ui::LogisticsBlackboard,
) -> Option<(EndpointMap, BTreeMap<String, i64>)> {
    let mut stocks = EndpointMap::new();
    let mut floors = BTreeMap::new();
    for store in &profile.logistics().config().stores {
        let observed = board.stores.iter().find(|row| row.id == store.id)?;
        if observed.level < 0
            || observed.available < 0
            || observed.available > observed.level
            || observed.incoming < 0
            || observed.held < 0
        {
            return None;
        }
        let mut stock = crate::resources::Stock::new(observed.level, observed.ceiling).ok()?;
        if observed.available < observed.level {
            stock
                .reserved
                .insert("observed".into(), observed.level - observed.available);
        }
        if observed.incoming > 0 {
            stock.incoming.insert("observed".into(), observed.incoming);
        }
        stocks.insert(
            store.endpoint.clone(),
            crate::resources::logistics::EndpointStock {
                stock,
                held: observed.held,
            },
        );
        floors.insert(store.id.clone(), observed.reserve);
    }
    Some((stocks, floors))
}
fn permitted_available(
    profile: &CompiledProduction,
    state: &ProductionState,
    tick: u64,
    config: &crate::ship::config::ShipConfig,
    boards: &std::collections::HashMap<crate::core::messages::SystemId, SystemBlackboard>,
) -> bool {
    let permitted = config.observations_for_system(&state.owner, boards);
    let Some(SystemBlackboard::Logistics(board)) =
        permitted.get(&profile.logistics().handling().system)
    else {
        return false;
    };
    let Some((stocks, floors)) = observed_inputs(profile, board) else {
        return false;
    };
    state
        .start_available(profile, tick, &floors, &stocks)
        .unwrap_or(false)
}

#[allow(clippy::type_complexity)]
fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&crate::entities::spawner::EntityUuid>,
        &ShipSystemControlSources,
        &ShipConfigComponent,
        &ShipProduction,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, sources, config, state, boards, programs, mut runtimes, mut admitted) in &mut ships {
        if !crate::ai::host::ai_operates(&sources.0, &state.0.owner) {
            continue;
        }
        let permitted = config.0.observations_for_system(&state.0.owner, &boards.0);
        let Some(SystemBlackboard::Production(board)) = permitted.get(&state.0.owner) else {
            continue;
        };
        let Some(runtime) = runtimes.0.get_mut(&state.0.owner) else {
            continue;
        };
        let facts = BTreeMap::from([
            ("enabled".into(), board.enabled),
            ("production_available".into(), board.can_start),
        ]);
        let channels = BTreeMap::from([("start".into(), false)]);
        let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
            &sources.0,
            &state.0.owner,
            programs.0.get(&state.0.owner),
            &facts,
            &channels,
            runtime,
            tick.0,
        ) else {
            continue;
        };
        if result.intents.get("start") == Some(&true) {
            crate::command_admission::ai_emit::emit_ai_command(
                uuid,
                state.0.owner.clone(),
                Control::StartProduction {},
                sources,
                &sessions,
                Some(config),
                &mut admitted,
            );
        }
    }
}
fn pay_work(
    system: &crate::core::messages::SystemId,
    seconds: f32,
    available: bool,
    factor: f32,
    power: Option<&mut crate::ship::power::ShipPowerSystem>,
) -> Result<f32, String> {
    if !available || factor <= 0.0 {
        return Ok(0.0);
    }
    power.map_or(Ok(0.0), |owner| {
        owner
            .0
            .work_continuous(system, seconds, 1.0)
            .map(|grant| grant.fraction * factor)
    })
}

#[allow(clippy::type_complexity)]
fn tick(
    time: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    mut runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
    mut ships: Query<(
        &crate::entities::spawner::EntityUuid,
        &CompiledShipProduction,
        &mut ShipProduction,
        &ShipLogistics,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &AdmittedCommands,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        Option<&mut InfrastructureCondition>,
        Option<&mut TorpedoSystemResource>,
        Option<&mut crate::ship::power::ShipPowerSystem>,
    )>,
) {
    let mut order: Vec<_> = ships.iter().map(|(id, ..)| id.0.clone()).collect();
    order.sort();
    for id in order {
        let Some((
            _,
            profile,
            mut owner,
            logistics,
            config,
            sources,
            boards,
            admitted,
            damage,
            mut inventory,
            mut torpedoes,
            mut power,
        )) = ships.iter_mut().find(|(uuid, ..)| uuid.0 == id)
        else {
            continue;
        };
        let mut next = owner.0.clone();
        let mut next_inventory = inventory.as_deref().cloned();
        let mut next_torpedoes = torpedoes.as_deref().cloned();
        let mut next_power = power.as_deref().cloned();
        let Ok(mut stocks) = endpoint_view(
            profile.0.logistics(),
            next_inventory.as_ref(),
            next_torpedoes.as_ref(),
        ) else {
            continue;
        };
        let policy = sources.0.policy_for(&next.owner);
        let available = sources.0.entries().any(|(system, _)| system == &next.owner)
            && (policy.accept_human_input || policy.operate_ai);
        let human = sources.0.entries().any(|(system, source)| {
            system == &next.owner && *source == crate::ship::control_source::ControlSource::Human
        });
        let factor = damage.map_or(1.0, |damage| {
            damage.factor(&next.owner, phoenix_model::damage::DamageBinding::Operation)
        });
        let mut starts = Vec::new();
        let mut replies = Vec::new();
        let control_owner = next.owner.clone();
        for command in admitted.for_target(&control_owner.0) {
            let result = match &command.payload {
                Control::SetProductionEnabled { enabled } => {
                    next.set_enabled(*enabled);
                    Ok(())
                }
                Control::SetProductionTarget { amount } => next.set_target(&profile.0, *amount),
                Control::SelectProductionRecipe { recipe, target } => {
                    next.select_recipe(&profile.0, recipe.as_deref(), *target, &mut stocks)
                }
                Control::StartProduction {} => {
                    starts.push(command);
                    continue;
                }
                _ => continue,
            };
            replies.push((command, result.is_ok()));
        }
        let advanced = next.tick(
            &profile.0,
            tick.0,
            time.delta_secs(),
            &mut stocks,
            |system, dt| pay_work(system, dt, available, factor, next_power.as_mut()),
        );
        let Ok(advanced) = advanced else {
            warn!(%id, "Production step refused; staged stock and energy discarded");
            continue;
        };
        let remaining = (time.delta_secs() - advanced.work_offered_secs).max(0.0);
        let maintain = human
            && available
            && next.enabled
            && permitted_available(&profile.0, &next, tick.0, &config.0, &boards.0);
        if maintain || !starts.is_empty() {
            let result = if available {
                next.try_start(
                    &profile.0,
                    tick.0,
                    remaining,
                    &logistics.0.floors,
                    &mut stocks,
                    |system, dt| pay_work(system, dt, available, factor, next_power.as_mut()),
                )
            } else {
                Ok(false)
            };
            let applied = matches!(result, Ok(true));
            for command in starts {
                replies.push((command, applied));
            }
            if result.is_err() {
                continue;
            }
        }
        let Ok(flags) = replace_endpoints(
            profile.0.logistics(),
            stocks,
            next_inventory.as_mut(),
            next_torpedoes.as_mut(),
        ) else {
            continue;
        };
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
            if let Some(inventory) = inventory.as_deref() {
                for capacity in inventory.0.capacities() {
                    runtime.flags.set_flag_value(&capacity.id, capacity.level);
                }
            }
            crate::infrastructure::server::mirror_flags(runtime, &flags, &id, &None);
        }
        for (command, applied) in replies {
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
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

#[allow(clippy::type_complexity)]
fn publish(
    tick: Res<crate::sim_tick::SimTick>,
    mut ships: Query<(
        &CompiledShipProduction,
        &ShipProduction,
        &ShipConfigComponent,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (profile, state, config, mut boards) in &mut ships {
        let can_start = permitted_available(&profile.0, &state.0, tick.0, &config.0, &boards.0);
        let recipes = profile
            .0
            .config()
            .recipes
            .iter()
            .map(|recipe| ProductionRecipeView {
                id: recipe.id.clone(),
                inputs: recipe.inputs.clone(),
                outputs: recipe.outputs.clone(),
                work_secs: recipe.work_secs,
                ceiling: profile.0.logistics().endpoints()[&profile
                    .0
                    .logistics()
                    .store(&recipe.outputs[0].store)
                    .expect("compiled recipe")
                    .endpoint]
                    .ceiling,
            })
            .collect();
        let active = state.0.active.as_ref().map(|batch| ProductionJobView {
            id: batch.id.0,
            recipe: batch.recipe.clone(),
            progress: batch.progress
                / profile
                    .0
                    .recipe(&batch.recipe)
                    .expect("validated active recipe")
                    .work_secs,
        });
        boards.0.insert(
            state.0.owner.clone(),
            SystemBlackboard::Production(ProductionBlackboard {
                enabled: state.0.enabled,
                selected_recipe: state.0.selected_recipe.clone(),
                target: state.0.target,
                recipes,
                active,
                can_start,
                blocked: state.0.blocked.map(|reason| {
                    match reason {
                        ProductionBlock::Disabled => "production.blocked.disabled",
                        ProductionBlock::Target => "production.blocked.target",
                        ProductionBlock::Inputs => "production.blocked.inputs",
                        ProductionBlock::OutputCapacity => "production.blocked.output_capacity",
                        ProductionBlock::Work => "production.blocked.work",
                    }
                    .into()
                }),
            }),
        );
    }
}
