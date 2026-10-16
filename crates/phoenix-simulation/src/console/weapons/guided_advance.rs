//! Advance the existing actual guided vectors before any physical Damage.
use super::{shared::TorpedoTargetSnapshot, TorpedoSystemResource};
use crate::{core::messages::ServerMessage, lobby::Target, server_app::SimOutbox};
use bevy::prelude::*;

pub(crate) fn advance_torpedo_flights(
    time: Res<Time>,
    mut torpedo_sys_q: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &mut TorpedoSystemResource,
            Option<&mut crate::ship::power::ShipPowerSystem>,
            Option<&mut super::guidance_link::ShipGuidanceKnowledge>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
        ),
        Or<(
            With<crate::server_app::Ship>,
            With<super::flight_carrier::WeaponFlightCarrier>,
        )>,
    >,
    (mut fallback, ships): (
        ResMut<TorpedoSystemResource>,
        Query<(), With<crate::server_app::Ship>>,
    ),
    damage_query: Query<&crate::ship::compartment_damage::ShipDamage>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    snapshot: Res<TorpedoTargetSnapshot>,
    mut outbox: ResMut<SimOutbox>,
    mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Projectile as usize }>,
) {
    let id_mint = mint.as_deref();
    let dt = time.delta_secs();
    let guidance = &snapshot.guidance;
    let mut any_ship_component = false;
    let mut shooter_order: Vec<((String, bevy::ecs::entity::EntityIndex), Entity)> = torpedo_sys_q
        .iter()
        .map(|(entity, uuid, _, _, _, _)| {
            (
                (
                    uuid.map(|u| u.0.clone()).unwrap_or_default(),
                    entity.index(),
                ),
                entity,
            )
        })
        .collect();
    shooter_order.sort();

    for shooter in shooter_order.into_iter().map(|(_, entity)| entity) {
        let Ok((_, _, mut torpedo_sys, mut power, mut knowledge, controls)) =
            torpedo_sys_q.get_mut(shooter)
        else {
            continue;
        };
        any_ship_component = true;
        let machinery = damage_query.get(shooter).ok();
        let can_work = |system: &crate::core::messages::SystemId| {
            machinery.is_none_or(|damage| {
                damage.factor(system, phoenix_model::damage::DamageBinding::Operation) > 0.0
            })
        };
        let burst_origins: std::collections::BTreeMap<_, _> = torpedo_sys
            .0
            .burst_states
            .iter()
            .map(|burst| (burst.tube_id.clone(), burst.launch_uuid.clone()))
            .collect();
        let magazine = crate::ship::system_registry::torpedo_magazine_system_id();
        let magazine_operation = if controls.is_none_or(|sources| sources.0.is_offline(&magazine)) {
            0.0
        } else {
            machinery.map_or(1.0, |damage| {
                damage.factor(&magazine, phoenix_model::damage::DamageBinding::Operation)
            })
        };
        let result = torpedo_sys.0.advance_with_loader_work(
            dt,
            guidance,
            magazine_operation,
            &mut |work| {
                use crate::weapons::torpedo::TorpedoWork;
                let Some(power) = power.as_deref_mut() else {
                    return 0.0;
                };
                match work {
                    TorpedoWork::Progress { tube, dt } => {
                        crate::ship::system_registry::torpedo_tube_system_id(tube)
                            .filter(|id| can_work(id))
                            .and_then(|id| power.0.work_continuous(&id, dt, 1.0).ok())
                            .map_or(0.0, |grant| grant.fraction)
                    }
                    TorpedoWork::Transfer => u8::from(
                        can_work(&crate::ship::system_registry::torpedo_magazine_system_id())
                            && matches!(
                                power.0.try_work_discrete(
                                    &crate::ship::system_registry::torpedo_magazine_system_id()
                                ),
                                Ok(true)
                            ),
                    ) as f32,
                    TorpedoWork::Launch { tube } => u8::from(
                        !power.0.weapons_restrained()
                            && crate::ship::system_registry::torpedo_tube_system_id(tube)
                                .is_some_and(|id| {
                                    can_work(&id)
                                        && matches!(power.0.try_work_discrete(&id), Ok(true))
                                }),
                    ) as f32,
                }
            },
            &mut || {
                crate::world_id::mint_live_id_with(
                    id_mint,
                    crate::world_id::IdNamespace::Projectile,
                )
            },
        );
        for (tube, uuid, x, y, z, heading) in result.burst_launched {
            let operation_factor = machinery.map_or(1.0, |damage| {
                crate::ship::system_registry::torpedo_tube_system_id(&tube).map_or(0.0, |system| {
                    damage.factor(&system, phoenix_model::damage::DamageBinding::Operation)
                })
            });
            let multiplier = operation_factor
                * power.as_deref_mut().map_or(1.0, |p| {
                    crate::ship::system_registry::torpedo_tube_system_id(&tube)
                        .map_or(1.0, |id| p.0.fire_strike_weapon(&id.0))
                });
            if let Some(torpedo) = torpedo_sys.0.in_flight.iter_mut().find(|t| t.uuid == uuid) {
                // Snapshot the same combined strike/Operation multiplier for burst rounds.
                torpedo.strike_damage_bonus = multiplier - 1.0;
                if let (Some(knowledge), Some(tick)) = (knowledge.as_deref_mut(), tick.as_deref()) {
                    if let Some(command) = burst_origins
                        .get(&tube)
                        .and_then(|id| knowledge.0.launch_commands.get(id))
                        .cloned()
                    {
                        super::guidance_link::record_emitted(
                            knowledge,
                            torpedo,
                            command.target,
                            command.sampled_tick,
                            tick.0,
                        );
                    }
                }
            }
            outbox.push_reliable((
                Target::All,
                ServerMessage::TorpedoLaunched {
                    uuid,
                    tube,
                    x,
                    y,
                    z,
                    heading,
                },
            ));
        }
    }
    if !any_ship_component && ships.is_empty() {
        let result = fallback
            .0
            .advance_with_work(dt, guidance, &mut |_| 0.0, &mut || {
                crate::world_id::mint_live_id_with(
                    id_mint,
                    crate::world_id::IdNamespace::Projectile,
                )
            });
        for (tube, uuid, x, y, z, heading) in result.burst_launched {
            outbox.push_reliable((
                Target::All,
                ServerMessage::TorpedoLaunched {
                    uuid,
                    tube,
                    x,
                    y,
                    z,
                    heading,
                },
            ));
        }
    }
}
