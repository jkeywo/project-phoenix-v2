// Test-only e13490cc publication path, retaining original whole-inbox copies.
use super::*;

pub(super) fn publish(
    inbox: Option<Res<CommsInboxRes>>,
    runtime: Option<Res<CommsRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    objective_instances: Option<Res<crate::world::server::ObjectiveInstanceManagerRes>>,
    mut ships: Query<
        (
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            Option<&crate::lockstep::FleetSlotOf>,
            Option<&EntityUuid>,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship::compartment_damage::ShipDamage>,
            &mut crate::server_app::ShipSystemBlackboards,
            Option<&crate::ship::components::HumanSeekingHosts>,
            Option<&crate::comms::radio::RadioProfile>,
            Option<&crate::comms::capture::ShipCommsCapture>,
            Option<&crate::comms::traffic::ShipCommsTraffic>,
            Option<&crate::ship::state::ShipPhysics>,
            Option<&Transform>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        Or<(
            With<crate::server_app::Ship>,
            With<crate::comms::radio::RadioProfile>,
        )>,
    >,
    paths: crate::regions::signal_path::RegionSignalQuery,
    content: Option<Res<WorldContentRuntime>>,
    base: Option<Res<crate::world::config::WorldConfig>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
) {
    for (
        is_local,
        slot,
        uuid,
        alert,
        hull,
        damage,
        mut boards,
        hosts,
        radio,
        captured,
        traffic,
        physics,
        transform,
        modifiers,
        config,
    ) in &mut ships
    {
        let key = radio.map_or_else(crate::ship::system_registry::comms_system_id, |radio| {
            radio.0.system.clone()
        });
        let mut bb = CommsBlackboard::default();
        if !is_local && slot.is_none() && radio.is_some() {
            // Same actor's current paid public endpoint measurement, not the
            // LocalShip roster/range projection or crew-only plot context.
            if let Some(SystemBlackboard::Comms(previous)) = boards.0.get(&key) {
                bb.contacts = previous.contacts.clone();
            }
        }
        if let Some(config) = config {
            let permitted = config.0.observations_for_system(&key, &boards.0);
            let mut stored = std::collections::BTreeMap::new();
            for board in boards.0.keys().filter_map(|source| permitted.get(source)) {
                if let SystemBlackboard::Bay(bay) = board {
                    for craft in &bay.stored {
                        stored.insert(craft.id.clone(), craft.clone());
                    }
                }
            }
            bb.reception_craft = stored.into_values().collect();
        }
        bb.media_capability = radio.and_then(|profile| profile.0.media);
        bb.traffic =
            traffic
                .zip(radio)
                .zip(tick.as_deref())
                .and_then(|((traffic, radio), tick)| {
                    crate::comms::traffic::view(traffic, &radio.0, tick.0)
                });
        bb.captured = captured
            .zip(radio)
            .and_then(|(captured, radio)| crate::comms::capture::views(captured, &radio.0));
        bb.receiver_interference = if modifiers.is_some_and(|modifiers| {
            modifiers.has_flag(&crate::core::messages::FlagKind::CommsJammed)
        }) {
            Some(true)
        } else {
            physics
                .map(|pose| Vec3::new(pose.x, pose.y, pose.z))
                .or_else(|| transform.map(|transform| transform.translation))
                .and_then(|point| {
                    paths.comms_receiver_interference(
                        point,
                        uuid.zip(radio)
                            .map(|(uuid, radio)| (uuid.0.as_str(), &radio.0.system)),
                    )
                })
        };
        bb.channels = radio
            .into_iter()
            .flat_map(|radio| &radio.0.channels)
            .map(|channel| phoenix_model::comms_channel::CommsChannelView {
                id: channel.id.clone(),
                range: channel.range,
                work_units: channel.work_units,
                protected: channel.key.is_some(),
            })
            .collect();
        // Ordinary non-roster radio equipment receives its own addressed text
        // view too. Legacy scenario plot access retains its existing boundary.
        bb.messages = inbox
            .as_ref()
            .map(|inbox| {
                inbox
                    .0
                    .messages()
                    .into_iter()
                    .filter(|m| m.is_for_ship(uuid.map(|id| id.0.as_str())))
                    .filter(|m| is_local || slot.is_some() || m.transmission.is_some())
                    .collect()
            })
            .unwrap_or_default();
        if is_local || slot.is_some() {
            if let Some(runtime) = runtime.as_deref() {
                for message in &mut bb.messages {
                    // Structured decisions are local; original receipt never
                    // becomes an oracle of current remote listener viability.
                    if message.request_progress.is_some() {
                        continue;
                    }
                    message.sender_in_range = slot.map_or_else(
                        || {
                            crate::comms::server::current_sender_in_range(
                                runtime,
                                &message.sender_uuid,
                            )
                        },
                        |slot| {
                            crate::comms::server::sender_in_range_for_slot(
                                runtime,
                                Some(slot.0),
                                &message.sender_uuid,
                            )
                        },
                    );
                    for response in &mut message.responses {
                        response.available =
                            message.request_progress.is_some() || message.sender_in_range;
                    }
                }
                bb.contacts = runtime.contacts.clone();
                for contact in &mut bb.contacts {
                    contact.in_range = slot.map_or_else(
                        || crate::comms::server::current_sender_in_range(runtime, &contact.uuid),
                        |slot| {
                            crate::comms::server::sender_in_range_for_slot(
                                runtime,
                                Some(slot.0),
                                &contact.uuid,
                            )
                        },
                    );
                    contact.is_urgent = bb.messages.iter().any(|m| {
                        m.sender_uuid == contact.uuid
                            && m.effective_priority().is_urgent()
                            && !m.is_read
                    });
                }
            }
            let conditions = crate::objectives::WorldConditions {
                red_alert: alert.is_some_and(|a| a.0),
                hull_fraction: crate::ship::compartment_damage::structural_fraction(damage, hull)
                    .unwrap_or(1.0),
                attacked: false,
            };
            bb.objectives = objectives
                .as_ref()
                .map(|o| {
                    let ship_id = uuid.map_or("", |id| id.0.as_str());
                    let scored = o.0.scored_pool_for(&conditions, ship_id);
                    let snapshots: Vec<_> = scored
                        .into_iter()
                        .filter(crate::objectives::is_visible_objective)
                        .map(|s| s.snapshot)
                        .collect();
                    objective_instances
                        .as_ref()
                        .map_or(snapshots.clone(), |instances| {
                            instances.0.project_snapshots_for_ship(ship_id, snapshots)
                        })
                })
                .unwrap_or_default();
        }
        for message in &mut bb.messages {
            crate::comms::requests::redact_private_progress(message);
        }
        if let (Some(content), Some(base), Some(uuid), Some(tick)) =
            (content.as_deref(), base.as_deref(), uuid, tick.as_deref())
        {
            bb.planned_requests = crate::world::activities::communication_requests(
                Some(content),
                Some(base),
                &uuid.0,
                &key,
                tick.0,
                |recipient| bb.contacts.iter().any(|contact| contact.uuid == recipient),
            );
        }
        bb.host_station = hosts.and_then(|h| h.0.get(&key).cloned());
        boards.0.insert(key.clone(), SystemBlackboard::Comms(bb));
    }
}
