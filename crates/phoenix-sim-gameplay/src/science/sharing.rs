//! Origin-preserving receipt and derived geometry for the ordinary track owner.
use super::tracks::{BroadTrack, BroadTrackState, SensorTrackSnapshot};
use phoenix_model::sensing::BroadConfig;
pub use phoenix_model::sensing_sharing::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedRadarSample {
    pub provenance: RadarSampleProvenance,
    pub last_delivery_tick: u64,
    /// Private mechanical route. Neither acquisition nor association evidence.
    pub route: Option<String>,
    pub stage: super::tracks::TrackStage,
    pub available: bool,
}

#[derive(Clone, Debug)]
pub struct SharedRadarSample {
    pub provenance: RadarSampleProvenance,
    pub record: SensorTrackSnapshot,
    pub route: Option<String>,
}

/// Stable public origin token; no mutable identifier ledger or raw actor ID.
pub fn origin_handle(ship: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in ship.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("radar-origin-{hash:016x}")
}
pub fn sample_id(origin: &RadarSampleOrigin, tick: u64) -> String {
    format!(
        "{}:{}{}:{}{}:{}:{tick}",
        origin.source.len(),
        origin.source,
        origin.system.0.len(),
        origin.system.0,
        origin.contact.len(),
        origin.contact
    )
}
fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
pub fn provenance_valid(value: &RadarSampleProvenance, tick: u64) -> bool {
    value.valid_at(tick)
}

/// Export originals, never a fused projection as a new independent sample.
pub fn export_samples(
    ship: &str,
    state: &BroadTrackState,
    profile: &BroadConfig,
    tick: u64,
    dt: f32,
) -> Result<Vec<SharedRadarSample>, String> {
    let mut records = state.projection(profile, tick, dt)?;
    let mut output = Vec::new();
    for record in &mut records {
        let track = &state.tracks[&record.contact_id];
        let provenance = track
            .received
            .as_ref()
            .map(|received| received.provenance.clone())
            .unwrap_or_else(|| {
                let origin = RadarSampleOrigin {
                    source: origin_handle(ship),
                    system: state.owner.clone(),
                    contact: record.contact_id.clone(),
                };
                RadarSampleProvenance {
                    sample_id: sample_id(&origin, track.observed_tick),
                    origin,
                    observed_tick: track.observed_tick,
                    observer_position: track.observer_position,
                    velocity_error: track.velocity_error,
                    drift_error_per_sec: profile.drift_error_per_sec,
                    received_tick: None,
                }
            });
        // Prediction is presentation only. Relays copy the held raw sample.
        record.position = track.position;
        record.position_error = track.position_error;
        record.samples = vec![provenance.clone()];
        output.push(SharedRadarSample {
            provenance,
            route: state.resolve_current(&record.contact_id).map(str::to_owned),
            record: record.clone(),
        });
    }
    Ok(output)
}

pub fn record_observer_pose(state: &mut BroadTrackState, tick: u64, pose: [f32; 2]) {
    for id in state.current.values() {
        if let Some(track) = state.tracks.get_mut(id) {
            if track.observed_tick == tick && track.observation.is_none() {
                track.observer_position = Some(pose);
            }
        }
    }
}

/// Normal radar consumers use this projection; exports always copy the original
/// canonical samples instead. In particular, fusion is never retransmitted as
/// another observation made by the relaying hull.
pub fn projection(
    ship: &str,
    state: &BroadTrackState,
    profile: &BroadConfig,
    tick: u64,
    dt: f32,
) -> Result<Vec<SensorTrackSnapshot>, String> {
    state.validate(profile, tick, dt)?;
    projection_live(ship, state, profile, tick, dt)
}

/// Installed runtime owner; public ingress retains the validating projection.
pub fn projection_live(
    ship: &str,
    state: &BroadTrackState,
    profile: &BroadConfig,
    tick: u64,
    dt: f32,
) -> Result<Vec<SensorTrackSnapshot>, String> {
    let mut records = state.projection_live(profile, tick, dt);
    let source = origin_handle(ship);
    for record in &mut records {
        if record.samples.is_empty() {
            let track = &state.tracks[&record.contact_id];
            let origin = RadarSampleOrigin {
                source: source.clone(),
                system: state.owner.clone(),
                contact: record.contact_id.clone(),
            };
            record.samples.push(RadarSampleProvenance {
                sample_id: sample_id(&origin, track.observed_tick),
                origin,
                observed_tick: track.observed_tick,
                observer_position: track.observer_position,
                velocity_error: track.velocity_error,
                drift_error_per_sec: profile.drift_error_per_sec,
                received_tick: None,
            });
        }
    }
    refine_geometry(&mut records);
    Ok(records)
}

/// Transactionally receive into the existing canonical track map. Repeated
/// delivery changes link availability, never evidence age or support duration.
pub fn receive_samples(
    ship: &str,
    state: &mut BroadTrackState,
    profile: &BroadConfig,
    tick: u64,
    dt: f32,
    samples: &[SharedRadarSample],
) -> Result<(), String> {
    state.validate(profile, tick, dt)?;
    if !state.receive_shared {
        return Ok(());
    }
    if samples.is_empty() {
        // No receipt, allocation or pruning occurs in the empty transaction.
        // Advancing this frontier still expires last-tick mechanical routes.
        state.last_tick = Some(tick);
        return Ok(());
    }
    let mut next = state.clone();
    for incoming in samples {
        let p = &incoming.provenance;
        let record = &incoming.record;
        if !provenance_valid(p, tick)
            || record.observed_tick != p.observed_tick
            || record.position.iter().any(|x| !x.is_finite())
            || record.elevation.as_ref().is_some_and(|elevation| {
                elevation.validate().is_err()
                    || record.observation.is_some()
                    || record.observed_tick == 0 && elevation.velocity.is_some()
            })
            || !record.position_error.is_finite()
            || record.position_error < 0.0
            || record
                .velocity
                .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
            || incoming.route.as_ref().is_some_and(|route| !text(route))
        {
            return Err("invalid shared radar sample".into());
        }
        if p.origin.source == origin_handle(ship) {
            continue;
        }
        if incoming.route.as_deref() == Some(ship) {
            continue;
        }
        if (tick - p.observed_tick) as f64 * f64::from(dt) > f64::from(profile.track_retention_secs)
        {
            continue;
        }
        let existing = next.tracks.iter().find_map(|(id, track)| {
            track
                .received
                .as_ref()
                .filter(|received| received.provenance.origin == p.origin)
                .map(|_| id.clone())
        });
        if existing
            .as_ref()
            .is_some_and(|id| next.tracks[id].observed_tick > p.observed_tick)
        {
            continue;
        }
        let id = if let Some(id) = existing {
            id
        } else {
            let Some(following) = next
                .next_contact
                .checked_add(1)
                .filter(|next| *next < u64::MAX)
            else {
                continue;
            };
            let id = format!(
                "{}:{}:contact-{}",
                next.owner.0.len(),
                next.owner.0,
                next.next_contact
            );
            next.next_contact = following;
            id
        };
        let first_receipt = next
            .tracks
            .get(&id)
            .and_then(|track| track.received.as_ref())
            .filter(|received| received.provenance.sample_id == p.sample_id)
            .and_then(|received| received.provenance.received_tick)
            .unwrap_or(tick);
        let mut provenance = p.clone();
        provenance.received_tick = Some(first_receipt);
        let mut observation = record.observation.clone();
        if let Some(observation) = &mut observation {
            observation.subject =
                phoenix_model::observation::ObservationSubject::Entity { uuid: id.clone() };
        }
        next.tracks.insert(
            id.clone(),
            BroadTrack {
                attached_references: next
                    .tracks
                    .get(&id)
                    .map(|held| held.attached_references.clone())
                    .unwrap_or_default(),
                contact_id: id,
                observation,
                received: Some(ReceivedRadarSample {
                    provenance,
                    last_delivery_tick: tick,
                    route: (!record.predicted && record.selectable)
                        .then(|| incoming.route.clone())
                        .flatten(),
                    stage: record.stage,
                    available: !record.predicted,
                }),
                observer_position: p.observer_position,
                selectable: record.selectable,
                observed_tick: record.observed_tick,
                position: record.position,
                elevation: record.elevation,
                velocity: record.velocity,
                velocity_error: p.velocity_error,
                position_error: record.position_error,
                supported_secs: 0.0,
                refresh_secs: 0.0,
                identity: record.identity.clone(),
                identity_tick: record.identity_tick,
                classification: record.classification.clone(),
                classification_tick: record.classification_tick,
                assessment: record.assessment.clone(),
                assessment_tick: record.assessment_tick,
            },
        );
    }
    next.last_tick = Some(tick);
    next.validate(profile, tick, dt)?;
    *state = next;
    Ok(())
}

/// Empty delivery on a validated runtime owner changes only the processing
/// frontier. Nonempty packets keep the full transactional ingress boundary.
pub fn receive_samples_live(
    ship: &str,
    state: &mut BroadTrackState,
    profile: &BroadConfig,
    tick: u64,
    dt: f32,
    samples: &[SharedRadarSample],
) -> Result<(), String> {
    if samples.is_empty() {
        debug_assert!(state.validate(profile, tick, dt).is_ok());
        if state.receive_shared {
            state.last_tick = Some(tick);
        }
        return Ok(());
    }
    receive_samples(ship, state, profile, tick, dt, samples)
}

/// Conservative two-disc intersection. Useful independent geometry can reduce
/// position uncertainty; no identity or condition is inferred by this function.
pub fn refine_geometry(records: &mut Vec<SensorTrackSnapshot>) {
    // Fusion needs two current originals from different observers. Ordinary
    // single-observer pictures cannot change, so do not deep-clone their rows.
    let mut origins = records
        .iter()
        .filter(|record| !record.predicted)
        .filter_map(|record| record.samples.first().map(|sample| &sample.origin.source));
    let Some(first) = origins.next() else {
        return;
    };
    if origins.all(|source| source == first) {
        return;
    }
    let originals = records.to_vec();
    let mut removed = BTreeSet::new();
    for record in records.iter_mut() {
        if record.predicted || removed.contains(&record.contact_id) {
            continue;
        }
        let mut contributors: BTreeSet<_> = record
            .samples
            .iter()
            .map(|sample| sample.sample_id.clone())
            .collect();
        for other in &originals {
            if other.predicted
                || removed.contains(&other.contact_id)
                || other.contact_id == record.contact_id
                || record.observed_tick != other.observed_tick
                || !compatible_elevation(record, other)
            {
                continue;
            }
            let (Some(a), Some(b)) = (record.samples.first(), other.samples.first()) else {
                continue;
            };
            if a.origin.source == b.origin.source || contributors.contains(&b.sample_id) {
                continue;
            }
            let (Some(a_pose), Some(b_pose)) = (a.observer_position, b.observer_position) else {
                continue;
            };
            let baseline = (f64::from(a_pose[0]) - f64::from(b_pose[0]))
                .hypot(f64::from(a_pose[1]) - f64::from(b_pose[1]));
            if baseline <= f64::from(record.position_error + other.position_error) {
                continue;
            }
            let dx = f64::from(other.position[0]) - f64::from(record.position[0]);
            let dz = f64::from(other.position[1]) - f64::from(record.position[1]);
            let d = dx.hypot(dz);
            let (r1, r2) = (
                f64::from(record.position_error),
                f64::from(other.position_error),
            );
            if d <= 0.0 || d >= r1 + r2 {
                continue;
            }
            // Both returns must have one compatible partner. A-B-C overlap
            // chains cannot turn B's ambiguous association into A's precision.
            let unique_partner = |subject: &SensorTrackSnapshot| {
                originals
                    .iter()
                    .filter(|candidate| {
                        candidate.contact_id != subject.contact_id
                            && compatible_elevation(subject, candidate)
                            && !candidate.predicted
                            && candidate.observed_tick == subject.observed_tick
                            && (f64::from(candidate.position[0]) - f64::from(subject.position[0]))
                                .hypot(
                                    f64::from(candidate.position[1])
                                        - f64::from(subject.position[1]),
                                )
                                < f64::from(subject.position_error + candidate.position_error)
                    })
                    .count()
                    == 1
            };
            if !unique_partner(record) || !unique_partner(other) {
                continue;
            }
            let along = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            if !(0.0..=d).contains(&along) {
                continue;
            }
            let radius = (r1 * r1 - along * along)
                .max(0.0)
                .sqrt()
                .max((along - (-r1).max(d - r2)).abs())
                .max((r1.min(d + r2) - along).abs());
            if radius >= r1.min(r2) {
                continue;
            }
            let center = [
                f64::from(record.position[0]) + dx * along / d,
                f64::from(record.position[1]) + dz * along / d,
            ];
            let position = center.map(|value| value as f32);
            let radius = radius
                + (center[0] - f64::from(position[0])).hypot(center[1] - f64::from(position[1]));
            let mut bounded = radius as f32;
            if f64::from(bounded) < radius {
                bounded = bounded.next_up();
            }
            if !bounded.is_finite() || f64::from(bounded) >= r1.min(r2) {
                continue;
            }
            record.position = position;
            record.position_error = bounded;
            for sample in &other.samples {
                if contributors.insert(sample.sample_id.clone()) {
                    record.samples.push(sample.clone());
                }
            }
            removed.insert(other.contact_id.clone());
        }
    }
    records.retain(|record| !removed.contains(&record.contact_id));
}

fn compatible_elevation(a: &SensorTrackSnapshot, b: &SensorTrackSnapshot) -> bool {
    match (a.elevation, b.elevation) {
        (Some(a), Some(b)) => {
            (f64::from(a.height) - f64::from(b.height)).abs()
                <= f64::from(a.error) + f64::from(b.error)
        }
        _ => true,
    }
}

#[cfg(test)]
#[path = "sharing_tests.rs"]
mod tests;
