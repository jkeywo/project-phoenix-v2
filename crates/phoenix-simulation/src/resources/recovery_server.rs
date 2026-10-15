//! Fitted recovery tools use permitted contacts and the ordinary finite transfer owner.
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::core::messages::{
    AdmittedCommands, SystemBlackboard, SystemControlPayload as Control, SystemId,
};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::infrastructure::InfrastructureCondition;
use crate::resources::logistics_server::{CompiledShipLogistics, ShipLogistics};
use crate::resources::recovery::*;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use phoenix_model::recovery_ui::*;

#[path = "recovery_contacts.rs"]
pub(super) mod contacts;

#[derive(Component, Clone, Debug)]
pub struct CompiledShipRecovery(pub RecoveryToolConfig);
#[derive(Component, Clone, Debug)]
pub struct ShipRecovery(pub RecoveryState);
#[derive(Component, Clone, Debug)]
pub struct RecoverableSource(pub RecoverableSourceConfig);

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    app.declare_state::<CompiledShipRecovery>(
        StateClass::Derived,
        "actual-source-recovery-and-bounded-processing",
    )
    .declare_state::<RecoverableSource>(
        StateClass::Derived,
        "actual-source-recovery-and-bounded-processing",
    )
    .declare_state::<ShipRecovery>(
        StateClass::Folded,
        "actual-source-recovery-and-bounded-processing",
    )
    .register_admitted_consumer(ConsumerMatcher::kind(RECOVERY_KIND).with_feedback(
        FeedbackAddress::DeclaredKindOnly,
        &[
            Payload::SetRecoveryEnabled,
            Payload::SetRecoverySelection,
            Payload::RecoverEquipment,
            Payload::CancelEquipmentRecovery,
        ],
    ));
    app.add_systems(
        FixedUpdate,
        operate_ai
            .in_set(crate::sim_sets::SimSet::Input)
            .run_if(crate::ai::cadence::ai_tick_ready),
    );
    app.add_systems(
        FixedUpdate,
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::infrastructure::server::tick_infrastructure_condition)
            .before(crate::resources::server::tick_resource_transfers),
    );
    app.add_systems(
        FixedUpdate,
        settle
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::resources::server::tick_resource_transfers)
            .before(crate::resources::logistics_server::tick),
    );
    app.add_systems(
        FixedUpdate,
        publish
            .in_set(crate::sim_sets::SimSet::Publish)
            .after(crate::resources::logistics_server::publish)
            .after(crate::sim_sets::FixedStep::PublishSensorRadarBlackboard)
            .after(crate::sim_sets::FixedStep::PublishTacticalRadarBlackboard),
    );
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = &config.recovery {
        commands.insert((
            CompiledShipRecovery(profile.clone()),
            ShipRecovery(RecoveryState::new(profile).expect("validated recovery tool")),
        ));
    }
}

pub(crate) fn install_source(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(source) = &config.recoverable {
        commands.insert(RecoverableSource(source.clone()));
    }
}

/// A feed confers information only. Co-hosted Consoles never pool contacts.
pub(crate) fn known_contacts(
    config: &crate::ship::config::ShipConfig,
    owner: &SystemId,
    boards: &std::collections::HashMap<SystemId, SystemBlackboard>,
    retained_target: Option<&str>,
) -> BTreeSet<String> {
    let mut known = BTreeSet::new();
    // Acquisition requires an explicit feed. Once acquired, the fitted tool
    // retains its admitted target and can keep measuring it within tool range.
    known.extend(retained_target.map(str::to_owned));
    let permitted = config.observations_for_system(owner, boards);
    for (source, board) in boards {
        if permitted.get(source).is_none() {
            continue;
        }
        match board {
            SystemBlackboard::SensorRadar(board) => known.extend(board.selected_target.clone()),
            SystemBlackboard::TacticalRadar(board) => {
                known.extend(board.selected_target.clone());
                known.extend(board.blips.iter().map(|blip| blip.uuid.clone()));
            }
            _ => {}
        }
    }
    known
}

fn local_destinations(
    profile: &RecoveryToolConfig,
    config: &crate::ship::config::ShipConfig,
    logistics: &CompiledShipLogistics,
    layout: &crate::ship::access::InteriorLayout,
    boards: &std::collections::HashMap<SystemId, SystemBlackboard>,
) -> Vec<phoenix_model::logistics_ui::LogisticsStoreView> {
    let permitted = config.observations_for_system(&profile.system, boards);
    let Some(SystemBlackboard::Logistics(board)) = permitted.get(&logistics.0.handling().system)
    else {
        return Vec::new();
    };
    let Some(compartment) = layout.0.placements.get(&profile.system) else {
        return Vec::new();
    };
    board
        .stores
        .iter()
        .filter(|store| {
            store.role == "main"
                && &store.compartment == compartment
                && profile.resources.contains(&store.resource)
        })
        .cloned()
        .collect()
}

#[allow(clippy::type_complexity)]
fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&EntityUuid>,
        &ShipSystemControlSources,
        &ShipRecovery,
        Option<&crate::resources::equipment_recovery_server::ShipEquipmentRecovery>,
        &ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, sources, state, equipment, config, boards, programs, mut runtimes, mut admitted) in
        &mut ships
    {
        if !crate::ai::host::ai_operates(&sources.0, &state.0.owner) {
            continue;
        }
        let permitted = config.0.observations_for_system(&state.0.owner, &boards.0);
        let Some(SystemBlackboard::Recovery(board)) = permitted.get(&state.0.owner) else {
            continue;
        };
        let Some(runtime) = runtimes.0.get_mut(&state.0.owner) else {
            continue;
        };
        let equipment_active =
            equipment.is_some_and(crate::resources::equipment_recovery_server::active);
        let ongoing = board.enabled
            && board.resource.is_some()
            && board.destination_store.is_some()
            && !matches!(
                board.blocked.as_deref(),
                Some("recovery.blocked.goal_met" | "recovery.blocked.no_target")
            );
        let facts = BTreeMap::from([
            ("enabled".into(), board.enabled),
            (
                "recovery_available".into(),
                board.next_recovery.is_some() || equipment_active || ongoing,
            ),
        ]);
        let channels = BTreeMap::from([("recover".into(), false)]);
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
        if equipment_active {
            let enabled = result.intents.get("recover") == Some(&true);
            if state.0.enabled != enabled {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    state.0.owner.clone(),
                    Control::SetRecoveryEnabled { enabled },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
            continue;
        }
        let choice = board
            .next_recovery
            .as_ref()
            .filter(|_| result.intents.get("recover") == Some(&true));
        if let Some(choice) = choice {
            let mut payloads = Vec::new();
            if board.target_uuid.as_ref() != Some(&choice.target_uuid)
                || board.resource.as_ref() != Some(&choice.resource)
                || board.destination_store.as_ref() != Some(&choice.destination_store)
            {
                payloads.push(Control::SetRecoverySelection {
                    target_uuid: Some(choice.target_uuid.clone()),
                    resource: Some(choice.resource.clone()),
                    destination_store: Some(choice.destination_store.clone()),
                });
            }
            if !board.enabled {
                payloads.push(Control::SetRecoveryEnabled { enabled: true });
            }
            for payload in payloads {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    state.0.owner.clone(),
                    payload,
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        } else if state.0.enabled && (!ongoing || result.intents.get("recover") != Some(&true)) {
            crate::command_admission::ai_emit::emit_ai_command(
                uuid,
                state.0.owner.clone(),
                Control::SetRecoveryEnabled { enabled: false },
                sources,
                &sessions,
                Some(config),
                &mut admitted,
            );
        }
    }
}

/// Source snapshots are ephemeral observations of actual endpoint owners. They
/// are never saved, replicated as inventories, or used by the AI directly.
struct SourceView {
    position: Vec3,
    name: String,
    config: RecoverableSourceConfig,
    available: bool,
    stocks: BTreeMap<String, i64>,
}

type SourceQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        &'static Transform,
        Option<&'static EntityName>,
        &'static RecoverableSource,
        Option<&'static crate::ship::disablement::DisablementObservation>,
        Option<&'static InfrastructureCondition>,
        Option<&'static TorpedoSystemResource>,
        Option<&'static CompiledShipLogistics>,
        Option<&'static ShipLogistics>,
    ),
>;

fn sources(query: &SourceQuery) -> BTreeMap<String, SourceView> {
    query
        .iter()
        .map(
            |(uuid, position, name, source, disabled, inventory, torpedoes, profile, state)| {
                let available = source.0.availability == RecoveryAvailability::Always
                    || disabled.is_some_and(|observation| observation.disabled);
                let stocks = source
                    .0
                    .resources
                    .iter()
                    .map(|(resource, endpoint)| {
                        let available = crate::resources::server::endpoint_stock(
                            endpoint, inventory, torpedoes,
                        )
                        .map(|(stock, _)| {
                            crate::resources::server::source_available(
                                &stock, endpoint, profile, state,
                            )
                        })
                        .unwrap_or(0);
                        (resource.clone(), available)
                    })
                    .collect();
                (
                    uuid.0.clone(),
                    SourceView {
                        position: position.translation,
                        name: name
                            .map(|name| name.0.clone())
                            .unwrap_or_else(|| uuid.0.clone()),
                        config: source.0.clone(),
                        available,
                        stocks,
                    },
                )
            },
        )
        .collect()
}

fn candidate(
    profile: &RecoveryToolConfig,
    source: &SourceView,
    contact: &contacts::Contact,
    position: Vec3,
) -> Option<RecoveryCandidate> {
    let distance = contact.position.map_or_else(
        || position.distance(source.position),
        |measured| {
            ((f64::from(position.x) - f64::from(measured[0]))
                .hypot(f64::from(position.z) - f64::from(measured[1]))) as f32
        },
    );
    if !distance.is_finite() || distance > profile.range {
        return None;
    }
    Some(RecoveryCandidate {
        uuid: contact.handle.clone(),
        name: if contact.position.is_some() {
            contact
                .name
                .clone()
                .unwrap_or_else(|| "console.common.unknown".into())
        } else {
            source.name.clone()
        },
        distance,
        resources: source
            .config
            .resources
            .keys()
            .filter(|resource| profile.resources.contains(resource))
            .cloned()
            .collect(),
    })
}

fn selection_limit(
    state: &RecoveryState,
    profile: &RecoveryToolConfig,
    position: Vec3,
    known: &BTreeSet<String>,
    sources: &BTreeMap<String, SourceView>,
    destinations: &[phoenix_model::logistics_ui::LogisticsStoreView],
) -> Result<i64, &'static str> {
    let Some(target) = state.target.as_ref() else {
        return Err("no_target");
    };
    if !known.contains(target) {
        return Err("unknown_target");
    }
    let source = sources.get(target).ok_or("source_unavailable")?;
    let distance = position.distance(source.position);
    if !distance.is_finite() || distance > profile.range {
        return Err("out_of_range");
    }
    if !source.available {
        return Err("source_unavailable");
    }
    let resource = state.resource.as_ref().ok_or("unsupported_resource")?;
    let stock = source.stocks.get(resource).ok_or("unsupported_resource")?;
    let destination = destinations
        .iter()
        .find(|store| {
            Some(&store.id) == state.destination_store.as_ref() && &store.resource == resource
        })
        .ok_or("unsupported_resource")?;
    if *stock <= 0 {
        return Err("no_stock");
    }
    if destination.headroom <= 0 {
        return Err("no_room");
    }
    let deficit = destination
        .target
        .saturating_sub(destination.level)
        .saturating_sub(destination.incoming)
        .saturating_sub(destination.held)
        .max(0);
    if deficit == 0 {
        return Err("goal_met");
    }
    Ok((*stock).min(destination.headroom).min(deficit))
}

#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    time: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    source_query: SourceQuery,
    mut transfers: ResMut<EffectQueue<crate::resources::server::ResourceTransfer>>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut ships: Query<(
        &EntityUuid,
        &Transform,
        &CompiledShipRecovery,
        &mut ShipRecovery,
        &CompiledShipLogistics,
        &ShipLogistics,
        Option<&InfrastructureCondition>,
        Option<&TorpedoSystemResource>,
        &crate::ship::access::InteriorLayout,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &AdmittedCommands,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        (
            Option<&mut crate::ship::power::ShipPowerSystem>,
            Option<&crate::resources::equipment_recovery_server::ShipEquipmentRecovery>,
            Option<&crate::science::broad::BroadTracks>,
            Option<&crate::science::broad::SensingControls>,
        ),
    )>,
) {
    let sources = sources(&source_query);
    let mut order: Vec<_> = ships.iter().map(|(uuid, ..)| uuid.0.clone()).collect();
    order.sort();
    for id in order {
        let Some((
            _,
            position,
            profile,
            mut owner,
            logistics,
            logistics_state,
            inventory,
            torpedoes,
            layout,
            config,
            control,
            boards,
            admitted,
            damage,
            (mut power, equipment, tracks, sensing),
        )) = ships.iter_mut().find(|(uuid, ..)| uuid.0 == id)
        else {
            continue;
        };
        let picture = contacts::Picture::capture(
            &config.0,
            &profile.0.system,
            &boards.0,
            tracks,
            sensing,
            &id,
            runtime.as_deref(),
        );
        let known = picture.known_for_work(owner.0.target.as_deref());
        let destinations = local_destinations(&profile.0, &config.0, logistics, layout, &boards.0);
        let mut next = owner.0.clone();
        let system = next.owner.clone();
        for command in admitted.for_target(&system.0) {
            let result = match &command.payload {
                Control::SetRecoveryEnabled { enabled } => {
                    next.set_enabled(*enabled);
                    Ok(())
                }
                Control::SetRecoverySelection {
                    target_uuid,
                    resource,
                    destination_store,
                } => {
                    let actual_target = target_uuid
                        .as_deref()
                        .and_then(|handle| picture.resolve(handle))
                        .filter(|contact| {
                            sources
                                .get(&contact.source)
                                .and_then(|source| {
                                    candidate(&profile.0, source, contact, position.translation)
                                })
                                .is_some()
                        })
                        .map(|contact| contact.source.as_str());
                    let mut selected = next.clone();
                    selected
                        .select(
                            &profile.0,
                            actual_target,
                            resource.as_deref(),
                            destination_store.as_deref(),
                        )
                        .and_then(|()| {
                            if target_uuid.is_none()
                                || (actual_target
                                    .is_some_and(|target| target != id && known.contains(target))
                                    && destinations.iter().any(|store| {
                                        Some(&store.id) == destination_store.as_ref()
                                            && Some(&store.resource) == resource.as_ref()
                                    })
                                    && actual_target
                                        .and_then(|target| sources.get(target))
                                        .is_some_and(|source| {
                                            position.translation.distance(source.position)
                                                <= profile.0.range
                                                && source
                                                    .config
                                                    .resources
                                                    .contains_key(resource.as_ref().unwrap())
                                        }))
                            {
                                next = selected;
                                Ok(())
                            } else {
                                Err("selection is not in this Console's permitted recovery view"
                                    .into())
                            }
                        })
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
        let policy = control.0.policy_for(&system);
        let available = control.0.entries().any(|(id, _)| id == &system)
            && (policy.accept_human_input || policy.operate_ai);
        let factor = damage.map_or(1.0, |damage| {
            damage.factor(&system, phoenix_model::damage::DamageBinding::Operation)
        });
        let observed_limit = if available && next.target.as_ref() != Some(&id) {
            selection_limit(
                &next,
                &profile.0,
                position.translation,
                &known,
                &sources,
                &destinations,
            )
            .unwrap_or(0)
        } else {
            0
        };
        // An observation proposes work; actual current receiving occupancy and
        // stock goals gate payment independently, including same-tick edits.
        let actual_limit = (|| -> Result<i64, String> {
            let destination = logistics
                .0
                .store(next.destination_store.as_deref().ok_or("no destination")?)?;
            let stocks = crate::resources::logistics_server::endpoint_view(
                &logistics.0,
                inventory,
                torpedoes,
            )?;
            let actual = &stocks[&destination.endpoint];
            let goal = logistics_state
                .0
                .goals
                .get(&destination.id)
                .copied()
                .ok_or("missing goal")?;
            let deficit = goal
                .saturating_sub(actual.stock.level)
                .saturating_sub(actual.stock.incoming_total())
                .saturating_sub(actual.held)
                .max(0);
            Ok(deficit.min(logistics.0.receiving_headroom(&stocks, &destination.id)?))
        })()
        .unwrap_or(0);
        let equipment_active = equipment
            .is_some_and(crate::resources::equipment_recovery_server::active)
            || admitted
                .for_target(&system.0)
                .any(|command| matches!(command.payload, Control::RecoverEquipment { .. }));
        let limit = if equipment_active {
            0
        } else {
            observed_limit.min(actual_limit)
        };
        let mut next_power = power.as_deref().cloned();
        let result = next.advance(
            &profile.0,
            tick.0,
            time.delta_secs(),
            limit,
            |system, seconds| {
                if factor <= 0.0 {
                    return Ok(0.0);
                }
                next_power.as_mut().map_or(Ok(0.0), |power| {
                    power
                        .0
                        .work_continuous(system, seconds, 1.0)
                        .map(|grant| grant.fraction * factor)
                })
            },
        );
        let Ok(offered) = result else {
            continue;
        };
        if offered > 0 {
            let target = next.target.as_ref().expect("work has selected target");
            let resource = next.resource.as_ref().expect("work has selected resource");
            let destination = logistics
                .0
                .store(
                    next.destination_store
                        .as_deref()
                        .expect("work has destination"),
                )
                .expect("permitted managed destination");
            transfers
                .0
                .push(crate::resources::server::ResourceTransfer {
                    operator: id.clone(),
                    source: target.clone(),
                    source_endpoint: sources[target].config.resources[resource].clone(),
                    destination: id.clone(),
                    destination_endpoint: destination.endpoint.clone(),
                    amount: offered,
                    receipt: Some(crate::resources::server::TransferReceiptKey {
                        system,
                        tick: tick.0,
                    }),
                });
        }
        owner.0 = next;
        if let (Some(owner), Some(next)) = (power.as_deref_mut(), next_power) {
            *owner = next;
        }
    }
}

fn settle(
    mut receipts: ResMut<EffectQueue<crate::resources::server::ResourceTransferReceipt>>,
    mut owners: Query<(&EntityUuid, &mut ShipRecovery)>,
) {
    for receipt in std::mem::take(&mut receipts.0) {
        if let Some((_, mut state)) = owners
            .iter_mut()
            .find(|(uuid, state)| uuid.0 == receipt.operator && state.0.owner == receipt.key.system)
        {
            state
                .0
                .record_receipt(receipt.key.tick, receipt.moved)
                .expect("recovery settles its unique bounded transfer receipt before fold");
        }
    }
}

#[allow(clippy::type_complexity)]
fn publish(
    source_query: SourceQuery,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut ships: Query<(
        &EntityUuid,
        &Transform,
        &CompiledShipRecovery,
        &ShipRecovery,
        &CompiledShipLogistics,
        &crate::ship::access::InteriorLayout,
        &ShipConfigComponent,
        &mut crate::server_app::ShipSystemBlackboards,
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
    )>,
) {
    let sources = sources(&source_query);
    for (uuid, position, profile, state, logistics, layout, config, mut boards, tracks, sensing) in
        &mut ships
    {
        let picture = contacts::Picture::capture(
            &config.0,
            &profile.0.system,
            &boards.0,
            tracks,
            sensing,
            &uuid.0,
            runtime.as_deref(),
        );
        let known = picture.known_for_work(state.0.target.as_deref());
        let destinations = local_destinations(&profile.0, &config.0, logistics, layout, &boards.0);
        let candidates: Vec<_> = picture
            .contacts
            .values()
            .filter(|contact| contact.source != uuid.0)
            .filter_map(|contact| {
                sources
                    .get(&contact.source)
                    .and_then(|source| candidate(&profile.0, source, contact, position.translation))
            })
            .filter(|candidate| !candidate.resources.is_empty())
            .collect();
        // Preserve selected work where possible; only ordinary AI chooses a new
        // contact when that work cannot meet a configured stock goal.
        let mut choices = candidates
            .iter()
            .flat_map(|candidate| {
                destinations.iter().filter_map(|store| {
                    if !candidate.resources.contains(&store.resource) {
                        return None;
                    }
                    let mut selected = state.0.clone();
                    let actual = &picture.resolve(&candidate.uuid)?.source;
                    selected
                        .select(
                            &profile.0,
                            Some(actual),
                            Some(&store.resource),
                            Some(&store.id),
                        )
                        .ok()?;
                    selection_limit(
                        &selected,
                        &profile.0,
                        position.translation,
                        &known,
                        &sources,
                        &destinations,
                    )
                    .ok()?;
                    Some(RecoverySelection {
                        target_uuid: candidate.uuid.clone(),
                        resource: store.resource.clone(),
                        destination_store: store.id.clone(),
                    })
                })
            })
            .collect::<Vec<_>>();
        let public_target = state
            .0
            .target
            .as_deref()
            .and_then(|source| picture.handle_for_source(source));
        if state.0.enabled
            && public_target.is_none()
            && selection_limit(
                &state.0,
                &profile.0,
                position.translation,
                &known,
                &sources,
                &destinations,
            )
            .is_ok()
        {
            // Loss of the radar handle does not withdraw previously admitted
            // physical work. Its own progress remains readable and cancellable.
            choices.clear();
        }
        choices.sort_by_key(|choice| {
            (
                Some(&choice.target_uuid) != public_target.as_ref()
                    || Some(&choice.resource) != state.0.resource.as_ref()
                    || Some(&choice.destination_store) != state.0.destination_store.as_ref(),
                destinations
                    .iter()
                    .find(|store| store.id == choice.destination_store)
                    .map_or(u16::MAX, |store| store.priority),
                choice.target_uuid.clone(),
                choice.resource.clone(),
                choice.destination_store.clone(),
            )
        });
        let blocked = if !state.0.enabled {
            Some("disabled")
        } else {
            selection_limit(
                &state.0,
                &profile.0,
                position.translation,
                &known,
                &sources,
                &destinations,
            )
            .err()
        };
        boards.0.insert(
            state.0.owner.clone(),
            SystemBlackboard::Recovery(RecoveryBlackboard {
                enabled: state.0.enabled,
                target_uuid: public_target,
                resource: state.0.resource.clone(),
                destination_store: state.0.destination_store.clone(),
                fraction: state.0.fraction,
                candidates,
                destinations,
                next_recovery: choices.into_iter().next(),
                last_moved: state
                    .0
                    .last_outcome
                    .as_ref()
                    .map_or(0, |outcome| outcome.moved),
                blocked: blocked.map(|reason| format!("recovery.blocked.{reason}")),
            }),
        );
    }
}

#[cfg(test)]
#[path = "recovery_server_tests.rs"]
mod tests;
