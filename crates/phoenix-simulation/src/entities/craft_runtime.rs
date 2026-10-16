//! Actual admitted bay work and one derived ready-Job physical handoff.
use super::*;
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload as Control},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::ecs::entity_disabling::Disabled;
use phoenix_sim_gameplay::ship::{composition::craft_bay_composition, craft::BayOperation};

#[derive(Clone)]
pub(crate) struct BayContext {
    pub uuid: String,
    pub config: phoenix_model::craft::BayConfig,
    pub cost: f32,
    pub operation: f32,
    pub available: bool,
    pub online: bool,
    pub pose: Option<([f32; 3], Quat)>,
}

pub(crate) fn context(world: &World, parent: Entity, owner: &SystemId) -> Option<BayContext> {
    let ship = &world.get::<ShipConfigComponent>(parent)?.0;
    let system = ship.system(owner)?;
    if system.kind != crate::ship::system_registry::CRAFT_BAY_KIND {
        return None;
    }
    let binding = craft_bay_composition(&system.components)?;
    let config = binding.config.clone();
    let sources = world.get::<ShipSystemControlSources>(parent);
    let source_ok = sources.is_some_and(|sources| {
        sources.0.entries().any(|(id, _)| id == owner)
            && !sources.0.is_offline(owner)
            && sources.0.source_for(owner) != crate::ship::control_source::ControlSource::Offline
    });
    let operation = world.get::<ShipDamage>(parent).map_or(0.0, |damage| {
        damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
    });
    let fitting_ok = match (
        world.get::<crate::resources::fitting_server::CompiledShipFitting>(parent),
        world.get::<crate::resources::fitting_server::ShipFitting>(parent),
    ) {
        (Some(profile), Some(state)) => {
            !crate::resources::fitting_server::unavailable(profile, state, owner)
        }
        (None, None) => true,
        _ => false,
    };
    // Existing physical stripping claims make the same Bay unavailable to
    // handling and crew handoff; no independent readiness owner is introduced.
    let recovery_ok = !world
        .try_query_filtered::<&crate::resources::equipment_recovery_server::ShipEquipmentRecovery, Allow<Disabled>>()
        .is_some_and(|mut query| {
            query.iter(world).any(|owner_state| {
                owner_state.job.as_ref().is_some_and(|job| {
                    job.outcome.is_none()
                        && job.activity.target_uuid
                            == world
                                .get::<EntityUuid>(parent)
                                .map_or("", |id| id.0.as_str())
                        && &job.activity.source_system == owner
                })
            })
        });
    let consumer = world.get::<ShipPowerSystem>(parent).and_then(|power| {
        power
            .0
            .config()
            .consumers
            .iter()
            .find(|consumer| &consumer.system == owner)
    });
    let cost = consumer.map_or(0.0, |consumer| consumer.continuous_cost_per_sec);
    let pose = parent_pose(world, parent, config.maximum_parent_speed);
    let access_ok = world
        .get::<crate::ship::access::InteriorLayout>(parent)
        .is_some_and(|layout| {
            world
                .get::<crate::ship::access::InteriorAccess>(parent)
                .is_none_or(|access| {
                    let blocked = access.blocked_connections(&layout.0);
                    !layout.0.connections.iter().any(|edge| {
                        (edge.from == config.compartment || edge.to == config.compartment)
                            && blocked.contains(&edge.id)
                    })
                })
        });
    Some(BayContext {
        uuid: world.get::<EntityUuid>(parent)?.0.clone(),
        config,
        cost,
        operation,
        online: source_ok,
        available: !world.entity(parent).contains::<Disabled>()
            && source_ok
            && fitting_ok
            && recovery_ok
            && access_ok
            && pose.is_some()
            && operation > 0.0
            && cost.is_finite()
            && cost > 0.0,
        pose,
    })
}

fn parent_pose(world: &World, parent: Entity, maximum_speed: f32) -> Option<([f32; 3], Quat)> {
    if let Some(physics) = world.get::<ShipPhysics>(parent) {
        let speed_squared = [
            physics.forward_speed,
            physics.lateral_speed,
            physics.vertical_speed,
        ]
        .into_iter()
        .map(|speed| f64::from(speed).powi(2))
        .sum::<f64>();
        let point = [physics.x, physics.y, physics.z];
        return (point.iter().all(|v| v.is_finite())
            && physics.yaw.is_finite()
            && physics.roll.is_finite()
            && speed_squared.is_finite()
            && speed_squared <= f64::from(maximum_speed).powi(2))
        .then_some((
            point,
            Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll),
        ));
    }
    // Only an accepted fixed physical parent can omit a motion owner.
    let body = world.get::<super::super::spawner::ColliderSection>(parent)?;
    if body.0.movable {
        return None;
    }
    let transform = world.get::<Transform>(parent)?;
    (transform.translation.is_finite()
        && transform.rotation.is_finite()
        && transform.rotation.is_normalized())
    .then_some((transform.translation.to_array(), transform.rotation))
}

pub(super) fn entity_for(world: &World, uuid: &str) -> Option<Entity> {
    let mut query = world.try_query_filtered::<(Entity, &EntityUuid), Allow<Disabled>>()?;
    let mut matches = query.iter(world).filter(|(_, id)| id.0 == uuid);
    let entity = matches.next()?.0;
    matches.next().is_none().then_some(entity)
}

pub(super) fn recovery_route(
    world: &World,
    parent: Entity,
    owner: &SystemId,
    contact: &str,
) -> Option<String> {
    let config = world.get::<ShipConfigComponent>(parent)?;
    let boards = world.get::<crate::server_app::ShipSystemBlackboards>(parent)?;
    // A newly supported recovery capability always needs actual current Broad
    // permission. Missing configured observations never admit guessed UUIDs.
    let tracks = world.get::<crate::science::broad::BroadTracks>(parent)?;
    let route =
        crate::science::contact_routing::resolve(&config.0, boards, Some(tracks), owner, contact)?;
    let observer = &world.get::<EntityUuid>(parent)?.0;
    if world
        .get_resource::<crate::world::server::WorldContentRuntime>()
        .is_some_and(|runtime| {
            crate::gm_contact::mode(&runtime.contact_overrides, observer, &route)
                == crate::gm_contact::ContactMode::Conceal
        })
    {
        return None;
    }
    Some(route)
}

pub(super) fn alias(
    world: &World,
    parent: Entity,
    owner: &SystemId,
    route: &str,
) -> Option<String> {
    let config = world.get::<ShipConfigComponent>(parent)?;
    let boards = world.get::<crate::server_app::ShipSystemBlackboards>(parent)?;
    let tracks = world.get::<crate::science::broad::BroadTracks>(parent)?;
    let contact = crate::science::contact_routing::alias_for_source(
        &config.0,
        boards,
        Some(tracks),
        owner,
        route,
    )?;
    (recovery_route(world, parent, owner, &contact).as_deref() == Some(route)).then_some(contact)
}

fn capacity(
    world: &World,
    parent: &str,
    owner: &SystemId,
    limit: u64,
    exclude_job: bool,
) -> Option<u64> {
    let mut occupied = 0_u64;
    if let Some(mut bodies) =
        world.try_query_filtered::<(&PhysicalProbe, &ProbeCustody), Allow<Disabled>>()
    {
        for (probe, custody) in bodies.iter(world) {
            if matches!(&custody.0, CraftLocation::Carried{parent_uuid,bay} if parent_uuid==parent && bay==owner)
            {
                occupied = occupied.checked_add(probe.0.space)?;
            }
        }
    }
    if let Some(mut bays) =
        world.try_query_filtered::<(&EntityUuid, &ShipBayJobs), Allow<Disabled>>()
    {
        for (uuid, jobs) in bays.iter(world) {
            if uuid.0 != parent {
                continue;
            }
            let Some(job) = jobs
                .0
                .get(owner)
                .and_then(|state| state.job.as_ref())
                .filter(|job| {
                    job.outcome.is_none() && job.activity.operation == BayOperation::Recover
                })
            else {
                continue;
            };
            if !exclude_job {
                let craft = entity_for(world, &job.activity.craft_uuid)?;
                occupied = occupied.checked_add(world.get::<PhysicalProbe>(craft)?.0.space)?;
            }
        }
    }
    (occupied <= limit).then_some(occupied)
}

fn claimed_elsewhere(world: &World, parent: Entity, owner: &SystemId, craft: &str) -> bool {
    world
        .try_query_filtered::<(Entity, &ShipBayJobs), Allow<Disabled>>()
        .is_some_and(|mut query| {
            query.iter(world).any(|(entity, jobs)| {
                jobs.0.iter().any(|(id, state)| {
                    (entity != parent || id != owner)
                        && state.job.as_ref().is_some_and(|job| {
                            job.outcome.is_none() && job.activity.craft_uuid == craft
                        })
                })
            })
        })
}

fn eligible(
    world: &World,
    parent: Entity,
    owner: &SystemId,
    context: &BayContext,
    target: Entity,
    operation: BayOperation,
    exclude_job: bool,
) -> bool {
    let Some(uuid) = world.get::<EntityUuid>(target) else {
        return false;
    };
    let Some(probe) = world.get::<PhysicalProbe>(target) else {
        return false;
    };
    if !context.available
        || claimed_elsewhere(world, parent, owner, &uuid.0)
        || world
            .get::<crate::console::repair::server::ShipRepairTeams>(parent)
            .is_some_and(|teams| teams.0.claims_personnel_bay(owner))
    {
        return false;
    }
    match operation {
        BayOperation::Launch => matches!(world.get::<ProbeCustody>(target),
            Some(ProbeCustody(CraftLocation::Carried{parent_uuid,bay})) if parent_uuid==&context.uuid && bay==owner),
        BayOperation::Recover => {
            if world.entity(target).contains::<Disabled>()
                || !matches!(
                    world.get::<ProbeCustody>(target),
                    Some(ProbeCustody(CraftLocation::Deployed {}))
                )
                || alias(world, parent, owner, &uuid.0).is_none()
            {
                return false;
            }
            let Some(physics) = world.get::<ShipPhysics>(target) else {
                return false;
            };
            let point = [physics.x, physics.y, physics.z];
            let Some((origin, _)) = context.pose else {
                return false;
            };
            let distance_squared = point
                .into_iter()
                .zip(origin)
                .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
                .sum::<f64>();
            distance_squared.is_finite()
                && distance_squared <= f64::from(context.config.recovery_range).powi(2)
                && capacity(
                    world,
                    &context.uuid,
                    owner,
                    context.config.capacity_space,
                    exclude_job,
                )
                .and_then(|used| used.checked_add(probe.0.space))
                .is_some_and(|used| used <= context.config.capacity_space)
        }
    }
}

pub(super) fn accepted_receptions(
    world: &World,
    parent: Entity,
    owner: &SystemId,
    now: u64,
) -> Vec<(phoenix_model::craft::CraftReceptionOrder, String)> {
    let (Some(config), Some(boards), Some(uuid), Some(inbox)) = (
        world.get::<ShipConfigComponent>(parent),
        world.get::<crate::server_app::ShipSystemBlackboards>(parent),
        world.get::<EntityUuid>(parent),
        world.get_resource::<crate::comms::server::CommsInboxRes>(),
    ) else {
        return Vec::new();
    };
    crate::comms::reception::accepted(config, boards, owner, &uuid.0, inbox, now)
}
fn order_permitted(
    world: &World,
    parent: Entity,
    owner: &SystemId,
    route: &str,
    reception: Option<&phoenix_model::craft::CraftReceptionOrder>,
    now: u64,
) -> bool {
    reception.is_none_or(|order| {
        order.craft() == Some(route)
            && accepted_receptions(world, parent, owner, now)
                .iter()
                .any(|(actual, _)| actual == order)
    })
}

pub(crate) fn handle(world: &mut World) {
    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let mut query = world.query::<(Entity, &EntityUuid, &ShipBayJobs, &AdmittedCommands)>();
    let mut commands: Vec<_> = query
        .iter(world)
        .flat_map(|(entity, uuid, _, admitted)| {
            admitted
                .0
                .iter()
                .cloned()
                .map(move |command| (uuid.0.clone(), entity, command))
        })
        .collect();
    commands.sort_by(|a, b| a.0.cmp(&b.0)); // Stable sort preserves each admitted batch order.
    for (_, parent, command) in commands {
        if !matches!(
            command.payload,
            Control::LaunchCraft { .. }
                | Control::RecoverCraft { .. }
                | Control::CancelCraftHandling
        ) {
            continue;
        }
        let owner = &command.target;
        let Some(context) = context(world, parent, owner) else {
            continue;
        };
        let mut state = world
            .get::<ShipBayJobs>(parent)
            .and_then(|jobs| jobs.0.get(owner))
            .cloned();
        let applied = state.as_mut().is_some_and(|state| match &command.payload {
            Control::CancelCraftHandling => context.online && state.cancel(),
            Control::LaunchCraft { craft } => entity_for(world, craft).is_some_and(|target| {
                eligible(
                    world,
                    parent,
                    owner,
                    &context,
                    target,
                    BayOperation::Launch,
                    true,
                ) && state
                    .begin(
                        owner,
                        craft,
                        BayOperation::Launch,
                        &context.config,
                        context.cost,
                        now,
                    )
                    .is_ok()
            }),
            Control::RecoverCraft { contact, reception } => {
                recovery_route(world, parent, owner, contact)
                    .and_then(|route| entity_for(world, &route).map(|target| (route, target)))
                    .is_some_and(|(route, target)| {
                        order_permitted(world, parent, owner, &route, reception.as_ref(), now)
                            && eligible(
                                world,
                                parent,
                                owner,
                                &context,
                                target,
                                BayOperation::Recover,
                                true,
                            )
                            && state
                                .begin_order(
                                    owner,
                                    &route,
                                    BayOperation::Recover,
                                    &context.config,
                                    context.cost,
                                    now,
                                    reception.as_ref(),
                                )
                                .is_ok()
                    })
            }
            _ => false,
        });
        if applied {
            world
                .get_mut::<ShipBayJobs>(parent)
                .unwrap()
                .0
                .insert(owner.clone(), state.unwrap());
        }
        let outcome = if applied {
            crate::core::messages::ActionFeedbackOutcome::Applied
        } else {
            crate::core::messages::ActionFeedbackOutcome::Refused
        };
        if let Some(feedback) =
            crate::command_admission::feedback::admitted_action_feedback(&command, outcome)
        {
            if let Some(mut outbox) =
                world.get_resource_mut::<Messages<crate::lobby::OutboundMessage>>()
            {
                outbox.write(feedback);
            }
        }
    }
}

fn jobs(world: &mut World) -> Vec<(Entity, SystemId)> {
    let mut query = world.query::<(Entity, &EntityUuid, &ShipBayJobs)>();
    let mut rows: Vec<_> = query
        .iter(world)
        .flat_map(|(entity, uuid, jobs)| {
            jobs.0
                .keys()
                .map(move |owner| (uuid.0.clone(), entity, owner.clone()))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
    rows.into_iter()
        .map(|(_, entity, owner)| (entity, owner))
        .collect()
}

/// Modifiers, after actual Dock motion. Only the SAME actual Job receives work.
pub(crate) fn tick(world: &mut World) {
    let Some(now) = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map(|tick| tick.0)
    else {
        return;
    };
    let Some(dt) = world
        .get_resource::<Time<Fixed>>()
        .map(|fixed| fixed.timestep().as_secs_f32())
    else {
        return;
    };
    for (parent, owner) in jobs(world) {
        let mut state = world.get::<ShipBayJobs>(parent).unwrap().0[&owner].clone();
        let Some(job) = state.job.as_ref().filter(|job| job.outcome.is_none()) else {
            continue;
        };
        let context = context(world, parent, &owner);
        let enabled = context
            .as_ref()
            .zip(entity_for(world, &job.activity.craft_uuid))
            .is_some_and(|(context, target)| {
                order_permitted(
                    world,
                    parent,
                    &owner,
                    &job.activity.craft_uuid,
                    job.activity.reception.as_ref(),
                    now,
                ) && eligible(
                    world,
                    parent,
                    &owner,
                    context,
                    target,
                    job.activity.operation,
                    true,
                )
            });
        let operation = if enabled {
            context.as_ref().unwrap().operation
        } else {
            0.0
        };
        let mut power = world.get::<ShipPowerSystem>(parent).cloned();
        let result = state.advance(&owner, now, dt, operation, |rate, seconds| {
            let amount = f64::from(rate) * f64::from(seconds);
            if !amount.is_finite() || amount <= 0.0 || amount > f64::from(f32::MAX) {
                return 0.0;
            }
            let mut rounded = amount as f32;
            if f64::from(rounded) < amount {
                rounded = f32::from_bits(rounded.to_bits() + 1);
            }
            power
                .as_mut()
                .and_then(|power| power.0.spend_continuous(&owner, rounded).ok())
                .map_or(0.0, |grant| {
                    (f64::from(grant.granted) / amount).min(1.0) as f32
                })
        });
        if result.is_ok() {
            if let (Some(mut actual), Some(paid)) =
                (world.get_mut::<ShipPowerSystem>(parent), power)
            {
                actual.0 = paid.0;
            }
            world
                .get_mut::<ShipBayJobs>(parent)
                .unwrap()
                .0
                .insert(owner, state);
        }
    }
}

/// End Physics, before ordinary Rapier SyncBackend removal and initialization.
/// A ready prior-boundary Job is the only pending handoff; no queue/phase exists.
pub(crate) fn commit(world: &mut World) {
    let Some(now) = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map(|tick| tick.0)
    else {
        return;
    };
    for (parent, owner) in jobs(world) {
        let mut state = world.get::<ShipBayJobs>(parent).unwrap().0[&owner].clone();
        let Some(job) = state.job.as_ref().filter(|job| {
            job.outcome.is_none() && job.activity.ready() && !job.paused && now > job.execution_tick
        }) else {
            continue;
        };
        let Some(context) = context(world, parent, &owner) else {
            continue;
        };
        let Some(target) = entity_for(world, &job.activity.craft_uuid) else {
            continue;
        };
        if !eligible(
            world,
            parent,
            &owner,
            &context,
            target,
            job.activity.operation,
            true,
        ) {
            continue;
        }
        let receipt = if let Some(order) = &job.activity.reception {
            if !order_permitted(
                world,
                parent,
                &owner,
                &job.activity.craft_uuid,
                Some(order),
                now,
            ) {
                continue;
            }
            let Some(roster) =
                world.get_resource::<phoenix_sim_gameplay::personnel::PersonnelRoster>()
            else {
                continue;
            };
            let mut people = Vec::new();
            for (origin, record) in &roster.origins {
                for (officer, state) in &record.state.officers {
                    if matches!(&state.location,phoenix_model::personnel::OfficerLocation::AboardCraft{craft,..}
                        if craft == &job.activity.craft_uuid)
                    {
                        people.push(phoenix_model::personnel::PersonRef {
                            origin: origin.clone(),
                            officer: officer.clone(),
                        });
                    }
                }
            }
            people.sort();
            Some(phoenix_model::craft::CraftReceptionReceipt {
                order: order.clone(),
                scope: phoenix_model::activity::ActivityJob {
                    actor: context.uuid.clone(),
                    owner: owner.clone(),
                    job: job.id,
                },
                craft: job.activity.craft_uuid.clone(),
                handoff_tick: now,
                people,
            })
        } else {
            None
        };
        let completed_copy = if let Some(receipt) = &receipt {
            let Some(inbox) = world.get_resource::<crate::comms::server::CommsInboxRes>() else {
                continue;
            };
            let Some(message) = crate::comms::reception::completed_copy(inbox, receipt, now) else {
                continue;
            };
            Some(message)
        } else {
            None
        };
        let operation = job.activity.operation;
        let release_offset = job.activity.profile.release_offset;
        if !state.complete_receiving(receipt) {
            continue;
        }
        let transition = match operation {
            BayOperation::Recover => set_carried(world, target, &context.uuid, &owner),
            BayOperation::Launch => {
                let Some((origin, rotation)) = context.pose else {
                    continue;
                };
                let Some(point) = release_point(origin, rotation, &release_offset) else {
                    continue;
                };
                let yaw = -rotation.to_euler(EulerRot::YXZ).0;
                if !clear(world, target, point, yaw) {
                    continue;
                }
                set_deployed(world, target, point, yaw)
            }
        };
        if transition.is_ok() {
            if let Some(message) = completed_copy {
                let mut inbox = world.resource_mut::<crate::comms::server::CommsInboxRes>();
                let message_id = message.id.clone();
                *inbox
                    .0
                    .request_copy_mut(&message_id, &context.uuid)
                    .expect("staged exact local receiving copy in this exclusive commit") = message;
                if let Some(mut runtime) =
                    world.get_resource_mut::<crate::comms::server::CommsRuntime>()
                {
                    runtime.needs_broadcast = true;
                }
            }
            world
                .get_mut::<ShipBayJobs>(parent)
                .unwrap()
                .0
                .insert(owner, state);
        }
    }
}

pub(super) fn release_point(
    origin: [f32; 3],
    rotation: Quat,
    offset: &[f32; 3],
) -> Option<[f32; 3]> {
    if !rotation.is_finite() || !rotation.is_normalized() {
        return None;
    }
    let [x, y, z, w] = rotation.to_array().map(f64::from);
    let [ox, oy, oz] = offset.map(f64::from);
    let point = [
        f64::from(origin[0])
            + (1.0 - 2.0 * (y * y + z * z)) * ox
            + 2.0 * (x * y - z * w) * oy
            + 2.0 * (x * z + y * w) * oz,
        f64::from(origin[1])
            + 2.0 * (x * y + z * w) * ox
            + (1.0 - 2.0 * (x * x + z * z)) * oy
            + 2.0 * (y * z - x * w) * oz,
        f64::from(origin[2])
            + 2.0 * (x * z - y * w) * ox
            + 2.0 * (y * z + x * w) * oy
            + (1.0 - 2.0 * (x * x + y * y)) * oz,
    ];
    point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX))
        .then(|| point.map(|v| v as f32))
}

fn clear(world: &World, craft: Entity, point: [f32; 3], yaw: f32) -> bool {
    use bevy_rapier3d::{na, parry, prelude::Collider};
    let Some(body) = world.get::<Collider>(craft) else {
        return false;
    };
    let rotation = Quat::from_rotation_y(-yaw);
    let iso = |point: Vec3, rotation: Quat| {
        na::Isometry3::from_parts(
            na::Translation3::new(point.x, point.y, point.z),
            na::UnitQuaternion::from_quaternion(na::Quaternion::new(
                rotation.w, rotation.x, rotation.y, rotation.z,
            )),
        )
    };
    let pose = iso(Vec3::from_array(point), rotation);
    let Some(mut bodies) =
        world.try_query::<(Entity, &Collider, &Transform, Option<&ShipPhysics>)>()
    else {
        return false;
    };
    bodies
        .iter(world)
        .all(|(entity, other, transform, physics)| {
            if entity == craft {
                return true;
            }
            let (point, rotation) =
                physics.map_or((transform.translation, transform.rotation), |physics| {
                    (
                        Vec3::new(physics.x, physics.y, physics.z),
                        Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll),
                    )
                });
            if !point.is_finite() || !rotation.is_finite() {
                return false;
            }
            parry::query::intersection_test(
                &pose,
                body.raw.as_ref(),
                &iso(point, rotation),
                other.raw.as_ref(),
            )
            .is_ok_and(|hit| !hit)
        })
}
