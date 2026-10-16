//! Opt-in late marker reports. Existing tracks and radio machinery remain the
//! sole owners; no inbox, target route, retry queue or controller is introduced.
use super::marking::MarkingBeaconProfile;
use crate::{
    comms::paid_radio::{route_available, CurrentPaidEndpoints},
    entities::spawner::EntityUuid,
    science::broad::{BroadProfile, BroadTracks},
    ship::power::ShipPowerSystem,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_model::{
    console::ConsoleObservationKind,
    marking::{MarkingLocation, MarkingReportFrontier},
};
use phoenix_sim_gameplay::{
    science::sharing::{export_samples, receive_samples, SharedRadarSample},
    weapons::marking::ShipMarkingAttachment,
};

pub(super) fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::sim_sets::FixedStep::ProjectMarkingAfterDock)
            .after(crate::science::broad::tick)
            // Legacy sharing stages all its sources before mutating recipients.
            // A new marker receipt does not alter that existing same-tick batch.
            .after(crate::science::sharing::deliver),
    );
}

struct Emission {
    source: crate::comms::radio::Endpoint,
    channel: String,
    sample: SharedRadarSample,
}

#[allow(clippy::type_complexity)]
fn tick(
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    endpoints: CurrentPaidEndpoints,
    paths: crate::regions::signal_path::RegionSignalQuery,
    mut owners: ParamSet<(
        Query<(
            Entity,
            &EntityUuid,
            &MarkingBeaconProfile,
            &mut ShipMarkingAttachment,
            &BroadProfile,
            &BroadTracks,
            &ShipConfigComponent,
            &mut ShipPowerSystem,
        )>,
        Query<(
            &BroadProfile,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &mut BroadTracks,
        )>,
    )>,
) {
    let now = tick.0;
    let dt = fixed.timestep().as_secs_f32();
    let endpoints = endpoints.collect(now);
    let mut emitted = Vec::new();
    {
        let mut sources = owners.p0();
        let mut ordered: Vec<_> = sources
            .iter()
            .map(|row| (row.1 .0.clone(), row.0))
            .collect();
        ordered.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, entity) in ordered {
            let Ok((_, uuid, marker, mut attachment, profiles, tracks, config, mut power)) =
                sources.get_mut(entity)
            else {
                continue;
            };
            let MarkingLocation::Attached { subject, .. } = &attachment.0.location else {
                continue;
            };
            let Some(source) = endpoints.iter().find(|source| {
                source.entity == entity
                    && source.radio.system == marker.0.radio
                    && source.listening > 0.0
            }) else {
                continue;
            };
            let Some(channel) = source.radio.channels.iter().find(|channel| {
                channel.id == marker.0.channel
                    && channel
                        .key
                        .as_ref()
                        .is_none_or(|key| source.radio.keys.contains(key))
            }) else {
                continue;
            };
            let permitted = config
                .0
                .console_for_system(&marker.0.radio)
                .is_some_and(|console| {
                    console.feeds.iter().any(|feed| {
                        feed.source == marker.0.sensor
                            && feed.kind == ConsoleObservationKind::SensorRadar
                    })
                });
            let (Some(profile), Some(state)) = (
                profiles.0.get(&marker.0.sensor),
                tracks.0.get(&marker.0.sensor),
            ) else {
                continue;
            };
            if !permitted || state.last_sample_tick != Some(now) {
                continue;
            }
            let Ok(samples) = export_samples(&uuid.0, state, &profile.profile, now, dt) else {
                continue;
            };
            let newest = samples
                .into_iter()
                .filter(|sample| {
                    sample.provenance.received_tick.is_none()
                        && sample.provenance.observed_tick == now
                        && sample.route.as_deref() == Some(subject.as_str())
                        && attachment.0.report.as_ref().is_none_or(|frontier| {
                            sample.provenance.observed_tick > frontier.observed_tick
                        })
                })
                .min_by(|a, b| a.provenance.sample_id.cmp(&b.provenance.sample_id));
            let Some(mut sample) = newest else { continue };
            if !power
                .0
                .try_work_discrete_count(&marker.0.radio, channel.work_units as usize)
                .unwrap_or(false)
            {
                continue;
            }
            // The source pays and stamps even when nobody can receive. Receipt
            // topology cannot disclose receiver presence or trigger hidden retries.
            attachment.0.report = Some(MarkingReportFrontier {
                sample: sample.provenance.sample_id.clone(),
                observed_tick: sample.provenance.observed_tick,
                emitted_tick: now,
            });
            sample.route = None;
            sample.record.observation = None;
            sample.record.identity = None;
            sample.record.identity_tick = None;
            sample.record.classification = None;
            sample.record.classification_tick = None;
            sample.record.assessment = None;
            sample.record.assessment_tick = None;
            emitted.push(Emission {
                source: source.clone(),
                channel: channel.id.clone(),
                sample,
            });
        }
    }
    let mut recipients = owners.p1();
    for receiver in &endpoints {
        let Ok((profiles, config, controls, mut tracks)) = recipients.get_mut(receiver.entity)
        else {
            continue;
        };
        let samples: Vec<_> = emitted
            .iter()
            .filter(|emission| {
                emission.source.uuid != receiver.uuid
                    && emission.source.faction.is_some()
                    && emission.source.faction == receiver.faction
                    && route_available(
                        &endpoints,
                        &emission.source,
                        receiver,
                        &emission.channel,
                        &paths,
                    )
            })
            .map(|emission| emission.sample.clone())
            .collect();
        if samples.is_empty() {
            continue;
        }
        for (owner, state) in &mut tracks.0 {
            if !state.receive_shared
                || !controls.0.entries().any(|(id, _)| id == owner)
                || controls.0.is_offline(owner)
                || controls.0.source_for(owner)
                    == crate::ship::control_source::ControlSource::Offline
                || config.0.system(owner).is_none_or(|system| {
                    system.kind != crate::ship::system_registry::SENSOR_RADAR_KIND
                })
            {
                continue;
            }
            let Some(profile) = profiles.0.get(owner) else {
                continue;
            };
            if let Err(error) =
                receive_samples(&receiver.uuid, state, &profile.profile, now, dt, &samples)
            {
                warn!("marker report refused malformed original observation: {error}");
            }
        }
    }
}

#[cfg(test)]
#[path = "marking_report_tests.rs"]
mod tests;
