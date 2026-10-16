use super::*;
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn tick_reference(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    measurements: GuidanceMeasurements,
    paths: crate::regions::signal_path::RegionSignalQuery,
    mut owners: Query<
        (
            Entity,
            &EntityUuid,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &ShipIllumination,
            &ShipPhysics,
            &mut ShipPowerSystem,
            Option<&ShipDamage>,
            Option<&crate::weapons::mounts::MountProfiles>,
            Option<&crate::weapons::mounts::ShipMounts>,
            &mut IlluminationEmissions,
        ),
        With<Ship>,
    >,
    mut flights: Query<&mut crate::console::weapons::TorpedoSystemResource>,
) {
    let dt = fixed.timestep().as_secs_f32();
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let mut order: Vec<_> = owners.iter().map(|row| (row.1 .0.clone(), row.0)).collect();
    order.sort();
    let mut emissions = Vec::new();
    for (_, entity) in order {
        let Ok((
            _,
            _,
            config,
            sources,
            state,
            own,
            mut power,
            damage,
            profiles,
            mounts,
            mut readout,
        )) = owners.get_mut(entity)
        else {
            continue;
        };
        readout.0.clear();
        for owner in state.0.keys() {
            if !state.enabled(&config.0, owner)
                || sources.0.is_offline(owner)
                || sources.0.source_for(owner)
                    == crate::ship::control_source::ControlSource::Offline
                || !sources.0.entries().any(|(id, _)| id == owner)
            {
                continue;
            }
            let Some(system) = config.0.system(owner) else {
                continue;
            };
            let Some(binding) = illumination_composition(&system.components) else {
                continue;
            };
            let Some(reading) = state
                .target(&config.0, owner)
                .and_then(|target| measurements.sample(entity, owner, target))
            else {
                continue;
            };
            let origin = [own.x, own.y, own.z];
            let distance = (DVec3::from_array(reading.position.map(f64::from))
                - DVec3::from_array(origin.map(f64::from)))
            .length();
            let geometry = crate::weapons::mounts::geometry(
                profiles,
                mounts,
                owner,
                &binding.config.mount,
                [reading.position[0], reading.position[2]],
                [own.x, own.z],
                own.yaw,
            );
            if !geometry.in_arc
                || !geometry.in_range
                || distance > f64::from(binding.config.mount.range)
            {
                continue;
            }
            let path = paths.transmission(
                Vec3::from_array(origin),
                Vec3::from_array(reading.position),
                crate::regions::signal_path::SignalClass::Sensors,
            );
            if !path.is_finite() || path < binding.config.minimum_transmission || path > 1.0 {
                continue;
            }
            let factor = damage.map_or(1.0, |damage| {
                damage.factor(owner, phoenix_model::damage::DamageBinding::Operation)
            });
            if !factor.is_finite() || factor <= 0.0 {
                continue;
            }
            // One fixed interval of emission is staged against the real owner.
            // A partial grant cannot produce a full-quality free observation.
            let mut paid = power.0.clone();
            if !paid
                .work_continuous(owner, dt, 1.0 / factor)
                .is_ok_and(|grant| grant.fraction >= 1.0 && grant.granted > 0.0)
            {
                continue;
            }
            power.0 = paid;
            readout.0.insert(owner.clone());
            emissions.push(Emission {
                origin,
                config: *binding.config,
                observation: reading,
            });
        }
    }
    for emission in emissions {
        for mut provider in &mut flights {
            for round in &mut provider.0.in_flight {
                if round.target_uuid.as_deref() != Some(emission.observation.target_uuid.as_str()) {
                    continue;
                }
                let Some(receiver) = round
                    .profile
                    .receiver
                    .filter(|receiver| receiver.accept_external_illumination)
                else {
                    continue;
                };
                if measurements.concealed(&round.uuid, &emission.observation.target_uuid)
                    || round.source_uuid.as_ref().is_some_and(|source| {
                        measurements.concealed(source, &emission.observation.target_uuid)
                    })
                {
                    continue;
                }
                let position = [round.x, round.y, round.z];
                let path = paths.transmission(
                    Vec3::from_array(emission.origin),
                    Vec3::from_array(position),
                    crate::regions::signal_path::SignalClass::Comms,
                );
                // Only the emitter's physical envelope is replaced here. Age
                // remains exactly the emitted receiver's frozen original bound.
                let transmitter = phoenix_model::guided_guidance::GuidanceReceiverProfile {
                    range: emission.config.link_range,
                    minimum_transmission: emission.config.minimum_transmission,
                    ..receiver
                };
                if link_reaches(&transmitter, position, emission.origin, path)
                    && receiver_accepts(
                        &receiver,
                        position,
                        emission.origin,
                        path,
                        &emission.observation,
                        tick.0,
                        fixed.timestep().as_secs_f64(),
                    )
                {
                    round.assist_guidance(&emission.observation);
                }
            }
        }
    }
}
