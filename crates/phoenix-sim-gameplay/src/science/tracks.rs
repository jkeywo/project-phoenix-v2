//! One lifecycle for paid broad observations and their retained predictions.
//! Actual source IDs remain private routing keys, never association evidence.
use phoenix_model::observation::{ObservationMetadata, ObservationSource, ObservationSubject};
pub use phoenix_model::sensing_tracks::{
    ObservedElevation, SensorTrackSnapshot, TrackAssessment, TrackIdentity, TrackStage,
};
use phoenix_model::{identity::SystemId, sensing::BroadConfig};
use serde::{Deserialize, Serialize};

pub fn shield_observation_sector(observer: [f32; 2], subject: [f32; 2]) -> u16 {
    let degrees =
        crate::simmath::atan2(observer[0] - subject[0], subject[1] - observer[1]).to_degrees();
    (degrees.rem_euclid(360.0).round() as u16) % 360
}
/// A measured ray can be compatible with an uncertain position without being
/// aligned to its centre. This reads only one published record's geometry.
pub fn shield_ray_compatible(
    observer: [f32; 2],
    subject: [f32; 2],
    position_error: f32,
    incoming_sector: u16,
) -> bool {
    if incoming_sector >= 360
        || !position_error.is_finite()
        || position_error < 0.0
        || observer
            .iter()
            .chain(subject.iter())
            .any(|value| !value.is_finite())
    {
        return false;
    }
    let dx = f64::from(observer[0]) - f64::from(subject[0]);
    let dz = f64::from(subject[1]) - f64::from(observer[1]);
    let distance = (dx.powi(2) + dz.powi(2)).sqrt();
    if distance <= f64::from(position_error) {
        return false;
    }
    let bearing = crate::simmath::atan2((dx / distance) as f32, (dz / distance) as f32)
        .to_degrees()
        .rem_euclid(360.0);
    let difference = (bearing - f32::from(incoming_sector)).abs();
    let allowance =
        crate::simmath::asin((f64::from(position_error) / distance) as f32).to_degrees() + 1.0;
    difference.min(360.0 - difference) <= allowance
}
use std::collections::{BTreeMap, BTreeSet};

/// Physical producer input, already limited by actual coverage/visibility.
/// `support` is this path's attenuation, independent of other contact counts.
#[derive(Clone, Debug)]
pub struct BroadMeasurement {
    pub source: String,
    /// Present only for already-permitted substituted reports. Receipt and
    /// sample times remain distinct from the instrument's processing tick.
    pub observation: Option<ObservationMetadata>,
    pub selectable: bool,
    pub position: [f32; 2],
    /// Raw physical height at the same sample; caller cannot supply velocity.
    pub elevation: Option<ObservedElevation>,
    pub support: f32,
    pub identity: Option<TrackIdentity>,
    pub classification: Option<String>,
    pub assessment: Option<TrackAssessment>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BroadTrack {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<super::sharing::ReceivedRadarSample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observer_position: Option<[f32; 2]>,
    pub contact_id: String,
    /// Deliberate links to held evidence, not observed identity or confidence.
    #[serde(default)]
    pub attached_references: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationMetadata>,
    pub selectable: bool,
    pub observed_tick: u64,
    pub position: [f32; 2],
    pub elevation: Option<ObservedElevation>,
    pub velocity: Option<[f32; 2]>,
    pub velocity_error: f32,
    pub position_error: f32,
    pub supported_secs: f64,
    pub refresh_secs: f64,
    pub identity: Option<TrackIdentity>,
    pub identity_tick: Option<u64>,
    pub classification: Option<String>,
    pub classification_tick: Option<u64>,
    pub assessment: Option<TrackAssessment>,
    pub assessment_tick: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BroadTrackState {
    #[serde(default)]
    pub share_own_faction: bool,
    #[serde(default = "phoenix_model::sensing_sharing::receive_by_default")]
    pub receive_shared: bool,
    pub owner: SystemId,
    pub last_tick: Option<u64>,
    pub last_sample_tick: Option<u64>,
    pub sample_elapsed: f64,
    pub next_contact: u64,
    pub tracks: BTreeMap<String, BroadTrack>,
    /// Current measured-source routing only. Cleared whenever a contact is lost.
    pub current: BTreeMap<String, String>,
}
impl BroadTrackState {
    pub fn new(profile: &BroadConfig) -> Self {
        Self {
            share_own_faction: false,
            receive_shared: true,
            owner: profile.system.clone(),
            last_tick: None,
            last_sample_tick: None,
            sample_elapsed: 0.0,
            next_contact: 1,
            tracks: BTreeMap::new(),
            current: BTreeMap::new(),
        }
    }
    /// Returns only the most recently supported physical association. This is
    /// actuator routing, not public identity evidence or a search over truth.
    pub fn resolve_current(&self, contact: &str) -> Option<&str> {
        if self
            .tracks
            .get(contact)?
            .observation
            .as_ref()
            .is_some_and(|observation| observation.subject == ObservationSubject::Unknown)
        {
            return None;
        }
        self.current
            .iter()
            .find_map(|(source, id)| (id == contact).then_some(source.as_str()))
            .or_else(|| {
                self.tracks
                    .get(contact)?
                    .received
                    .as_ref()
                    .filter(|_| self.receive_shared)
                    .filter(|received| Some(received.last_delivery_tick) == self.last_tick)?
                    .route
                    .as_deref()
            })
    }
    pub fn contact_for_source(&self, source: &str) -> Option<&str> {
        if let Some(id) = self.current.get(source) {
            return self.resolve_current(id).map(|_| id.as_str());
        }
        let mut contacts = self
            .current_routes()
            .filter(|(route, _)| *route == source)
            .map(|(_, id)| id);
        let first = contacts.next()?;
        contacts.next().is_none().then_some(first)
    }
    pub fn current_routes(&self) -> impl Iterator<Item = (&str, &str)> {
        // Build this call's reverse lookup once. Entry order preserves the
        // original resolver's first lexicographic source, even before validation
        // rejects multiple physical sources bound to one contact.
        let mut sources = BTreeMap::new();
        for (source, contact) in &self.current {
            sources.entry(contact.as_str()).or_insert(source.as_str());
        }
        self.tracks.iter().filter_map(move |(id, track)| {
            if !route_subject_known(track) {
                return None;
            }
            sources
                .get(id.as_str())
                .copied()
                .or_else(|| self.received_route(track))
                .map(|source| (source, id.as_str()))
        })
    }
    fn received_route<'a>(&'a self, track: &'a BroadTrack) -> Option<&'a str> {
        track
            .received
            .as_ref()
            .filter(|_| self.receive_shared)
            .filter(|received| Some(received.last_delivery_tick) == self.last_tick)?
            .route
            .as_deref()
    }
    pub fn validate(&self, profile: &BroadConfig, tick: u64, fixed_dt: f32) -> Result<(), String> {
        profile_valid(profile, fixed_dt)?;
        if self.owner != profile.system
            || self.next_contact == 0
            || self.next_contact == u64::MAX
            || self.last_tick.is_some_and(|last| last > tick)
            || self
                .last_sample_tick
                .is_some_and(|sample| self.last_tick.is_none_or(|last| sample > last))
            || !self.sample_elapsed.is_finite()
            || self.sample_elapsed < 0.0
            || self.sample_elapsed >= f64::from(profile.sample_interval_secs)
            || self.last_sample_tick.is_none()
                && (self.tracks.values().any(|track| track.received.is_none())
                    || !self.current.is_empty())
            || self.last_tick.is_none()
                && (self.last_sample_tick.is_some()
                    || self.next_contact != 1
                    || self.sample_elapsed != 0.0)
        {
            return Err("invalid broad observation owner or clock frontier".into());
        }
        let mut ids = BTreeSet::new();
        for (id, track) in &self.tracks {
            let prefix = format!("{}:{}:contact-", self.owner.0.len(), self.owner.0);
            let sequence = id.strip_prefix(&prefix).and_then(|n| {
                n.parse::<u64>()
                    .ok()
                    .filter(|sequence| sequence.to_string() == n)
            });
            if track
                .attached_references
                .iter()
                .any(|(reference, attached)| {
                    reference.is_empty()
                        || reference.len() > 128
                        || reference.chars().any(char::is_control)
                        || *attached > tick
                })
                || &track.contact_id != id
                || sequence.is_none_or(|n| n == 0 || n >= self.next_contact)
                || !ids.insert(id)
                || (if track.received.is_some() {
                    self.last_tick
                } else {
                    self.last_sample_tick
                })
                .is_none_or(|sample| track.observed_tick > sample)
                || track
                    .observer_position
                    .is_some_and(|pose| pose.iter().any(|x| !x.is_finite()))
                || !track.position.iter().all(|value| value.is_finite())
                || track.elevation.as_ref().is_some_and(|elevation| {
                    elevation.validate().is_err()
                        || track.observation.is_some()
                        || track.observed_tick == 0 && elevation.velocity.is_some()
                        || track.received.is_none() && elevation.error != profile.position_error
                })
                || track
                    .velocity
                    .is_some_and(|velocity| !velocity.iter().all(|value| value.is_finite()))
                || !track.velocity_error.is_finite()
                || track.velocity_error < 0.0
                || !track.position_error.is_finite()
                || track.position_error < 0.0
                || track.received.is_none() && track.position_error != profile.position_error
                || !track.supported_secs.is_finite()
                || track.supported_secs < 0.0
                || track.supported_secs > f64::from(profile.assessment_secs)
                || !track.refresh_secs.is_finite()
                || track.refresh_secs < 0.0
                || track.refresh_secs > track.supported_secs
                || [
                    (track.identity.is_some(), track.identity_tick),
                    (track.classification.is_some(), track.classification_tick),
                    (track.assessment.is_some(), track.assessment_tick),
                ]
                .into_iter()
                .any(|(has, tick)| {
                    has != tick.is_some() || tick.is_some_and(|tick| tick > track.observed_tick)
                })
                || track.supported_secs < f64::from(profile.identity_secs)
                    && track.identity.is_some()
                    && track.observation.is_none()
                    && track.received.is_none()
                || track
                    .assessment
                    .as_ref()
                    .and_then(|assessment| assessment.protection.as_ref())
                    .is_some_and(|protection| protection.validate().is_err())
                || track.identity.as_ref().is_some_and(|identity| {
                    identity.name.is_empty()
                        || identity.references.iter().any(|reference| {
                            reference.is_empty() || reference.chars().any(char::is_control)
                        })
                        || identity
                            .references
                            .windows(2)
                            .any(|pair| pair[0] >= pair[1])
                })
                || track.supported_secs < f64::from(profile.classification_secs)
                    && track.classification.is_some()
                    && track.received.is_none()
                || track.supported_secs < f64::from(profile.assessment_secs)
                    && track.assessment.is_some()
                    && track.received.is_none()
                || track.received.as_ref().is_some_and(|received| {
                    !super::sharing::provenance_valid(&received.provenance, tick)
                        || received.provenance.observed_tick != track.observed_tick
                        || self
                            .last_tick
                            .is_none_or(|last| received.last_delivery_tick > last)
                        || received
                            .provenance
                            .received_tick
                            .is_none_or(|first| first > received.last_delivery_tick)
                        || received
                            .route
                            .as_ref()
                            .is_some_and(|route| !valid_source(route))
                        || received.route.is_some() && (!received.available || !track.selectable)
                        || track.observer_position != received.provenance.observer_position
                        || track.velocity_error != received.provenance.velocity_error
                        || track.supported_secs != 0.0
                        || track.refresh_secs != 0.0
                })
                || track.observation.as_ref().is_some_and(|observation| {
                    !report_metadata_valid(observation, tick)
                        || observation.observed_tick != Some(track.observed_tick)
                        || observation.provenance.system.as_ref()
                            != Some(
                                track.received.as_ref().map_or(&self.owner, |received| {
                                    &received.provenance.origin.system
                                }),
                            )
                        || (observation.subject
                            != (ObservationSubject::Entity { uuid: id.clone() })
                            && observation.subject != ObservationSubject::Unknown)
                        || observation.subject == ObservationSubject::Unknown && track.selectable
                        || track.classification.is_some()
                        || track.assessment.is_some()
                        || track
                            .identity
                            .as_ref()
                            .is_some_and(|identity| identity.faction.is_some())
                        || track.supported_secs != 0.0
                        || track.refresh_secs != 0.0
                        || track.velocity.is_some()
                        || track.velocity_error != 0.0
                        || track.identity_tick
                            != track.identity.as_ref().map(|_| track.observed_tick)
                })
            {
                return Err("invalid retained broad measurement or finding frontier".into());
            }
        }
        let mut associations = BTreeSet::new();
        for (source, id) in &self.current {
            if !valid_source(source)
                || self
                    .tracks
                    .get(id)
                    .is_none_or(|track| track.received.is_some())
                || !associations.insert(id)
            {
                return Err("invalid current broad contact association".into());
            }
            if source.starts_with("ghost/")
                != self.tracks[id]
                    .observation
                    .as_ref()
                    .is_some_and(|observation| observation.subject == ObservationSubject::Unknown)
            {
                return Err("invalid synthetic observation route".into());
            }
        }
        Ok(())
    }
    /// One fixed-step paid observation opportunity, with no contact-count cost.
    /// The caller supplies a real grant already paid by this broad System.
    pub fn advance(
        &mut self,
        profile: &BroadConfig,
        tick: u64,
        fixed_dt: f32,
        paid_fraction: f32,
        measurements: &[BroadMeasurement],
    ) -> Result<(), String> {
        self.validate(profile, tick, fixed_dt)?;
        let mut next = self.clone();
        next.advance_inner(profile, tick, fixed_dt, paid_fraction, measurements)?;
        next.validate(profile, tick, fixed_dt)?;
        *self = next;
        Ok(())
    }
    /// Runtime-only path for an installed profile and canonical state already
    /// validated at construction/restore. Physical inputs still validate before
    /// mutation; the shared lifecycle avoids cloning the complete held history.
    pub fn advance_live(
        &mut self,
        profile: &BroadConfig,
        tick: u64,
        fixed_dt: f32,
        paid_fraction: f32,
        measurements: &[BroadMeasurement],
    ) -> Result<(), String> {
        debug_assert!(self.validate(profile, tick, fixed_dt).is_ok());
        self.advance_inner(profile, tick, fixed_dt, paid_fraction, measurements)?;
        debug_assert!(self.validate(profile, tick, fixed_dt).is_ok());
        Ok(())
    }
    fn advance_inner(
        &mut self,
        profile: &BroadConfig,
        tick: u64,
        fixed_dt: f32,
        paid_fraction: f32,
        measurements: &[BroadMeasurement],
    ) -> Result<(), String> {
        if !paid_fraction.is_finite() || !(0.0..=1.0).contains(&paid_fraction) {
            return Err("invalid paid broad work fraction".into());
        }
        let mut sources = BTreeSet::new();
        for sample in measurements {
            if !valid_source(&sample.source)
                || !sources.insert(&sample.source)
                || !sample.position.iter().all(|value| value.is_finite())
                || sample.elevation.as_ref().is_some_and(|elevation| {
                    elevation.validate().is_err()
                        || elevation.velocity.is_some()
                        || elevation.error != profile.position_error
                        || sample.observation.is_some()
                })
                || !sample.support.is_finite()
                || !(0.0..=1.0).contains(&sample.support)
                || sample.observation.as_ref().is_some_and(|observation| {
                    !report_metadata_valid(observation, tick)
                        || observation.provenance.system.as_ref() != Some(&self.owner)
                        || sample.classification.is_some()
                        || sample.assessment.is_some()
                        || (observation.subject == ObservationSubject::Unknown && sample.selectable)
                        || sample
                            .identity
                            .as_ref()
                            .is_some_and(|identity| identity.faction.is_some())
                })
            {
                return Err("invalid or duplicate broad measurement".into());
            }
            if sample.source.starts_with("ghost/")
                != sample
                    .observation
                    .as_ref()
                    .is_some_and(|observation| observation.subject == ObservationSubject::Unknown)
            {
                return Err("invalid synthetic observation route".into());
            }
        }
        if self.last_tick == Some(tick) {
            return Ok(());
        }
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err("broad work cannot rewind".into());
        }
        let next = self;
        next.last_tick = Some(tick);
        next.sample_elapsed += f64::from(fixed_dt);
        let due = next.last_sample_tick.is_none()
            || next.sample_elapsed >= f64::from(profile.sample_interval_secs);
        next.sample_elapsed %= f64::from(profile.sample_interval_secs);
        let mut samples = prepared_samples(measurements, paid_fraction, profile.position_error);
        // New anonymous identities follow measured geometry, not a hidden
        // UUID ordering. Private routing breaks only indistinguishable ties.
        samples.sort_by(|(a, a_position), (b, b_position)| {
            a_position[0]
                .total_cmp(&b_position[0])
                .then_with(|| a_position[1].total_cmp(&b_position[1]))
                .then_with(|| a.source.cmp(&b.source))
        });
        // Two unresolved returns within their measured discs are not secretly
        // disambiguated by the physical source key, including while crossing.
        let confused: BTreeSet<_> = samples
            .iter()
            .filter(|(a, a_position)| {
                samples.iter().any(|(b, b_position)| {
                    a.source != b.source
                        && distance(*a_position, *b_position)
                            <= f64::from(profile.position_error) * 2.0
                })
            })
            .map(|(sample, _)| sample.source.as_str())
            .collect();
        next.current.retain(|source, _| {
            samples.iter().any(|(sample, _)| &sample.source == source)
                && !confused.contains(source.as_str())
        });
        if due && !samples.is_empty() {
            let claimed: BTreeSet<_> = next.current.values().cloned().collect();
            let candidates: BTreeMap<_, Vec<_>> = samples
                .iter()
                .filter(|(sample, _)| !next.current.contains_key(&sample.source))
                .map(|(sample, position)| {
                    let candidates = next
                        .tracks
                        .iter()
                        .filter(|(id, track)| {
                            track.received.is_none()
                                && !claimed.contains(*id)
                                && (tick - track.observed_tick) as f64 * f64::from(fixed_dt)
                                    <= f64::from(profile.track_retention_secs)
                                && {
                                    let (predicted, error) = prediction(track, tick, fixed_dt);
                                    distance(predicted, *position)
                                        <= f64::from(error)
                                            + f64::from(profile.position_error)
                                            + (tick - track.observed_tick) as f64
                                                * f64::from(fixed_dt)
                                                * f64::from(profile.drift_error_per_sec)
                                }
                        })
                        .map(|(id, _)| id.clone())
                        .collect();
                    (sample.source.clone(), candidates)
                })
                .collect();
            for (sample, position) in &samples {
                if next.current.contains_key(&sample.source) {
                    continue;
                }
                let candidates_for_source = &candidates[&sample.source];
                let association = (candidates_for_source.len() == 1
                    && !confused.contains(sample.source.as_str()))
                .then(|| candidates_for_source[0].clone())
                .filter(|id| candidates.values().filter(|list| list.contains(id)).count() == 1);
                let id = if let Some(id) = association {
                    next.tracks
                        .get_mut(&id)
                        .expect("retained association")
                        .refresh_secs = 0.0;
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
                    next.tracks.insert(
                        id.clone(),
                        BroadTrack {
                            received: None,
                            observer_position: None,
                            contact_id: id.clone(),
                            attached_references: BTreeMap::new(),
                            observation: None,
                            selectable: sample.selectable,
                            observed_tick: tick,
                            position: if sample.observation.is_some() {
                                measured_position(sample.position, profile.position_error)
                            } else {
                                *position
                            },
                            elevation: None,
                            velocity: None,
                            velocity_error: 0.0,
                            position_error: profile.position_error,
                            supported_secs: 0.0,
                            refresh_secs: 0.0,
                            identity: None,
                            identity_tick: None,
                            classification: None,
                            classification_tick: None,
                            assessment: None,
                            assessment_tick: None,
                        },
                    );
                    id
                };
                next.current.insert(sample.source.clone(), id);
            }
        }
        let mut identity_counts = BTreeMap::<&str, usize>::new();
        for (sample, _) in &samples {
            if let Some(identity) = &sample.identity {
                *identity_counts.entry(&identity.name).or_default() += 1;
            }
        }
        for (sample, position) in &samples {
            let Some(id) = next.current.get(&sample.source) else {
                continue;
            };
            let track = next.tracks.get_mut(id).expect("checked association");
            if let Some(observation) = &sample.observation {
                let mut observation = observation.clone();
                if observation.subject != ObservationSubject::Unknown {
                    observation.subject = ObservationSubject::Entity { uuid: id.clone() };
                }
                // Reprocessing a held report buys no new identity evidence,
                // renews no clock, and cannot learn current velocity or facts.
                if track.observation.as_ref() != Some(&observation) {
                    track.position = sample.position;
                    track.observed_tick = observation.observed_tick.expect("validated report time");
                    track.velocity = None;
                    track.elevation = None;
                    track.velocity_error = 0.0;
                    track.supported_secs = 0.0;
                    track.refresh_secs = 0.0;
                    track.identity.clone_from(&sample.identity);
                    track.identity_tick = track.identity.as_ref().map(|_| track.observed_tick);
                    track.classification = None;
                    track.classification_tick = None;
                    track.assessment = None;
                    track.assessment_tick = None;
                    track.observation = Some(observation);
                }
                track.selectable = sample.selectable;
                continue;
            }
            if track.observation.is_some() {
                if !due {
                    continue;
                }
                // A real new measurement must reacquire its own findings; it
                // does not inherit permissions from a substituted report.
                track.observation = None;
                track.supported_secs = 0.0;
                track.refresh_secs = 0.0;
                track.identity = None;
                track.identity_tick = None;
                // A displaced reported point and a fresh physical return are
                // not two measurements of a physical velocity.
                track.position = *position;
                track.observed_tick = tick;
                track.velocity = None;
                track.elevation = None;
                track.velocity_error = 0.0;
            }
            let ambiguous_identity = sample.identity.as_ref().is_some_and(|identity| {
                identity_counts
                    .get(identity.name.as_str())
                    .copied()
                    .unwrap_or(0)
                    > 1
            });
            if ambiguous_identity {
                if let Some(identity) = &mut track.identity {
                    identity.references.clear();
                }
            }
            track.supported_secs = (track.supported_secs
                + f64::from(fixed_dt) * f64::from(paid_fraction) * f64::from(sample.support))
            .min(f64::from(profile.assessment_secs));
            track.refresh_secs = (track.refresh_secs
                + f64::from(fixed_dt) * f64::from(paid_fraction) * f64::from(sample.support))
            .min(f64::from(profile.assessment_secs));
            if !due {
                continue;
            }
            let position = *position;
            let elevation = sample.elevation.map(|sample| {
                let rounded = measured_position([sample.height, 0.0], sample.error)[0];
                // Casting a distant grid centre to f32 can move it beyond the
                // promised error. The representable input remains a valid centre.
                let height = if (f64::from(rounded) - f64::from(sample.height)).abs()
                    <= f64::from(sample.error)
                {
                    rounded
                } else {
                    sample.height
                };
                let velocity =
                    track
                        .elevation
                        .filter(|_| tick > track.observed_tick)
                        .map(|previous| {
                            bounded_float(
                                (f64::from(height) - f64::from(previous.height))
                                    / ((tick - track.observed_tick) as f64 * f64::from(fixed_dt)),
                            )
                        });
                ObservedElevation {
                    height,
                    error: sample.error,
                    velocity,
                }
            });
            if tick > track.observed_tick {
                let elapsed = (tick - track.observed_tick) as f64 * f64::from(fixed_dt);
                track.velocity = Some(
                    [
                        (f64::from(position[0]) - f64::from(track.position[0])) / elapsed,
                        (f64::from(position[1]) - f64::from(track.position[1])) / elapsed,
                    ]
                    .map(bounded_float),
                );
                track.velocity_error =
                    bounded_float(2.0 * f64::from(profile.position_error) / elapsed);
            }
            track.position = position;
            track.elevation = elevation;
            track.selectable = sample.selectable;
            track.observed_tick = tick;
            if track.refresh_secs >= f64::from(profile.identity_secs) && sample.identity.is_some() {
                let mut identity = sample.identity.clone().expect("present identity");
                if ambiguous_identity {
                    identity.references.clear();
                } else if let Some(held) = track
                    .identity
                    .as_ref()
                    .filter(|held| held.name == identity.name && held.faction == identity.faction)
                {
                    identity.references.extend(held.references.iter().cloned());
                }
                identity.references.sort();
                identity.references.dedup();
                track.identity = Some(identity);
                track.identity_tick = sample.identity.as_ref().map(|_| tick);
            }
            if track.refresh_secs >= f64::from(profile.classification_secs)
                && sample.classification.is_some()
            {
                track.classification.clone_from(&sample.classification);
                track.classification_tick = sample.classification.as_ref().map(|_| tick);
            }
            if track.refresh_secs >= f64::from(profile.assessment_secs)
                && sample.assessment.is_some()
            {
                track.assessment.clone_from(&sample.assessment);
                track.assessment_tick = sample.assessment.as_ref().map(|_| tick);
            }
        }
        if due && !samples.is_empty() {
            next.last_sample_tick = Some(tick);
        }
        let current: BTreeSet<_> = next.current.values().cloned().collect();
        next.tracks.retain(|id, track| {
            current.contains(id)
                || (tick - track.observed_tick) as f64 * f64::from(fixed_dt)
                    <= f64::from(profile.track_retention_secs)
        });
        Ok(())
    }
    pub fn projection(
        &self,
        profile: &BroadConfig,
        tick: u64,
        fixed_dt: f32,
    ) -> Result<Vec<SensorTrackSnapshot>, String> {
        self.validate(profile, tick, fixed_dt)?;
        Ok(self.projection_live(profile, tick, fixed_dt))
    }
    /// Projection of installed, validated runtime state. External/saved inputs
    /// must pass `validate` (or use `projection`) before reaching this path.
    pub fn projection_live(
        &self,
        profile: &BroadConfig,
        tick: u64,
        fixed_dt: f32,
    ) -> Vec<SensorTrackSnapshot> {
        debug_assert!(self.validate(profile, tick, fixed_dt).is_ok());
        let current: BTreeSet<_> = self.current.values().collect();
        self.tracks
            .values()
            .map(|track| {
                let (position, error) = prediction(track, tick, fixed_dt);
                let age = (tick - track.observed_tick) as f64 * f64::from(fixed_dt);
                SensorTrackSnapshot {
                    samples: track
                        .received
                        .as_ref()
                        .map(|received| vec![received.provenance.clone()])
                        .unwrap_or_default(),
                    references: Vec::new(),
                    contact_id: track.contact_id.clone(),
                    elevation: track.elevation,
                    observation: track.observation.clone().map(|mut observation| {
                        observation.subject = ObservationSubject::Entity {
                            uuid: track.contact_id.clone(),
                        };
                        observation
                    }),
                    selectable: track.selectable
                        && route_subject_known(track)
                        && (current.contains(&track.contact_id)
                            || self.received_route(track).is_some()),
                    observed_tick: track.observed_tick,
                    position,
                    velocity: track.velocity,
                    position_error: bounded_float(
                        f64::from(error)
                            + age
                                * f64::from(
                                    track
                                        .received
                                        .as_ref()
                                        .map_or(profile.drift_error_per_sec, |received| {
                                            received.provenance.drift_error_per_sec
                                        }),
                                ),
                    ),
                    stage: if let Some(received) = &track.received {
                        received.stage
                    } else if track.observation.is_some() {
                        if track.identity.is_some() {
                            TrackStage::Identified
                        } else {
                            TrackStage::Return
                        }
                    } else if track.supported_secs >= f64::from(profile.assessment_secs) {
                        TrackStage::Assessed
                    } else if track.supported_secs >= f64::from(profile.classification_secs) {
                        TrackStage::Classified
                    } else if track.supported_secs >= f64::from(profile.identity_secs) {
                        TrackStage::Identified
                    } else {
                        TrackStage::Return
                    },
                    identity: track.identity.clone(),
                    identity_tick: track.identity_tick,
                    classification: track.classification.clone(),
                    classification_tick: track.classification_tick,
                    assessment: track.assessment.clone(),
                    assessment_tick: track.assessment_tick,
                    predicted: if let Some(received) = &track.received {
                        !self.receive_shared
                            || !received.available
                            || Some(received.last_delivery_tick) != self.last_tick
                    } else {
                        !current.contains(&track.contact_id)
                    },
                }
            })
            .collect()
    }
}
fn route_subject_known(track: &BroadTrack) -> bool {
    !track
        .observation
        .as_ref()
        .is_some_and(|observation| observation.subject == ObservationSubject::Unknown)
}
fn report_metadata_valid(observation: &ObservationMetadata, tick: u64) -> bool {
    observation.is_valid()
        && observation.provenance.kind == ObservationSource::SensorsReport
        && !observation.provenance.label.is_empty()
        && observation
            .observed_tick
            .is_some_and(|observed| observed <= tick)
        && observation.received_tick.is_none_or(|received| {
            observation
                .observed_tick
                .is_some_and(|observed| observed <= received)
                && received <= tick
        })
}
/// One transient grid point per supported input; ordering and physical inputs
/// remain separate so reports keep their original (unquantized) evidence.
fn prepared_samples(
    measurements: &[BroadMeasurement],
    paid_fraction: f32,
    error: f32,
) -> Vec<(&BroadMeasurement, [f32; 2])> {
    measurements
        .iter()
        .filter(|sample| paid_fraction > 0.0 && sample.support > 0.0)
        .map(|sample| (sample, sample_position(sample, error)))
        .collect()
}
fn sample_position(sample: &BroadMeasurement, error: f32) -> [f32; 2] {
    if sample.observation.is_some() {
        sample.position
    } else {
        measured_position(sample.position, error)
    }
}
fn profile_valid(profile: &BroadConfig, fixed_dt: f32) -> Result<(), String> {
    phoenix_model::sensing::SensingConfig {
        control: Default::default(),
        broad: vec![profile.clone()],
        ..Default::default()
    }
    .validate()?;
    if !fixed_dt.is_finite() || fixed_dt <= 0.0 {
        return Err("broad sensing needs a positive fixed tick duration".into());
    }
    Ok(())
}
fn valid_source(value: &str) -> bool {
    if let Some(value) = value.strip_prefix("ghost/") {
        let Some((length, id)) = value.split_once(':') else {
            return false;
        };
        return valid_source_id(id) && length == id.len().to_string();
    }
    valid_source_id(value)
}
fn valid_source_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
/// Private sample key scoped by the existing observing instrument owner.
pub fn synthetic_source(id: &str) -> Result<String, String> {
    if !valid_source_id(id) {
        return Err("invalid synthetic observation key".into());
    }
    Ok(format!("ghost/{}:{id}", id.len()))
}
fn bounded_float(value: f64) -> f32 {
    value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32
}
fn distance(a: [f32; 2], b: [f32; 2]) -> f64 {
    let x = f64::from(a[0]) - f64::from(b[0]);
    let z = f64::from(a[1]) - f64::from(b[1]);
    // Compare squared distance where possible in callers; this helper's sqrt
    // is pure platform floating arithmetic, never a source of random evidence.
    (x * x + z * z).sqrt()
}
pub fn measured_position(position: [f32; 2], error: f32) -> [f32; 2] {
    if error == 0.0 {
        return position;
    }
    let cell = f64::from(error) * std::f64::consts::SQRT_2;
    position.map(|value| bounded_float(((f64::from(value) / cell).floor() + 0.5) * cell))
}
fn prediction(track: &BroadTrack, tick: u64, fixed_dt: f32) -> ([f32; 2], f32) {
    let age = (tick - track.observed_tick) as f64 * f64::from(fixed_dt);
    let velocity = track.velocity.unwrap_or([0.0; 2]);
    (
        [
            bounded_float(f64::from(track.position[0]) + age * f64::from(velocity[0])),
            bounded_float(f64::from(track.position[1]) + age * f64::from(velocity[1])),
        ],
        bounded_float(f64::from(track.position_error) + age * f64::from(track.velocity_error)),
    )
}
#[cfg(test)]
#[path = "tracks_tests.rs"]
mod tests;
