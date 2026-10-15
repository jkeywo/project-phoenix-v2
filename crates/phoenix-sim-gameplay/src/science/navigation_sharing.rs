//! Navigation's single held-sharing frontier, adjacent to its ordinary waypoint.
//! Inputs are already permitted fixed points; no entity or command port lives here.
use super::sharing::{origin_handle, sample_id};
pub use phoenix_model::navigation_sharing::*;
use phoenix_model::{identity::SystemId, sensing_sharing::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedNavigationObservation {
    pub observation: NavigationSharedObservation,
    pub last_delivery_tick: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationSharingState {
    pub owner: SystemId,
    pub share_own_faction: bool,
    pub receive: bool,
    pub last_tick: Option<u64>,
    pub local: BTreeMap<String, NavigationSharedObservation>,
    pub received: BTreeMap<String, ReceivedNavigationObservation>,
}
#[derive(Clone, Debug)]
pub struct NavigationSamplePoint {
    pub kind: NavigationSharedKind,
    pub position: [f32; 2],
    /// A measured anchor retains its actual observation clock and pose.
    /// Own free points and static charts use the first publication opportunity.
    pub observed_tick: Option<u64>,
    pub observer_position: Option<[f32; 2]>,
}

fn text(value: &str, bound: usize) -> bool {
    !value.is_empty() && value.len() <= bound && !value.chars().any(char::is_control)
}
pub fn validate_config(config: &crate::entities::config::EntityConfig) -> Result<(), String> {
    let Some(sharing) = config
        .navigation_console
        .as_ref()
        .and_then(|nav| nav.sharing.as_ref())
    else {
        return Ok(());
    };
    let ship = config
        .ship_config
        .as_ref()
        .ok_or("Navigation sharing needs actual System composition")?;
    let system = ship
        .system(&sharing.system)
        .ok_or("Navigation sharing owner is not fitted")?;
    if system.kind != crate::ship::system_registry::NAVIGATION_KIND
        || system
            .console
            .as_ref()
            .is_none_or(|id| !ship.consoles.iter().any(|console| &console.id == id))
    {
        return Err(
            "Navigation sharing requires an actual Navigation System and owning Console".into(),
        );
    }
    NavigationSharingState::new(sharing).validate(0)?;
    let radar = &config.navigation_console.as_ref().unwrap().system_chart;
    if !radar.range.is_finite() || radar.range <= 0.0 {
        return Err("Navigation sharing requires positive finite chart coverage".into());
    }
    Ok(())
}
pub fn item_key(kind: &NavigationSharedKind) -> String {
    match kind {
        NavigationSharedKind::Waypoint => "waypoint".into(),
        NavigationSharedKind::Landmark { id, .. } => format!("chart/{}:{id}", id.len()),
    }
}
pub fn observation_valid(observation: &NavigationSharedObservation, tick: u64) -> bool {
    let p = &observation.provenance;
    let source = p.origin.source.strip_prefix("radar-origin-");
    source.is_some_and(|source| {
        source.len() == 16
            && source
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) && text(&p.origin.system.0, 128)
        && p.origin.contact == item_key(&observation.kind)
        && p.sample_id == sample_id(&p.origin, observation.published_tick)
        && p.observed_tick <= observation.published_tick
        && observation.published_tick <= tick
        && p.received_tick
            .is_none_or(|received| received >= observation.published_tick && received <= tick)
        && p.observer_position
            .is_none_or(|pose| pose.iter().all(|value| value.is_finite()))
        && p.velocity_error == 0.0
        && p.drift_error_per_sec == 0.0
        && observation.position.iter().all(|value| value.is_finite())
        && match &observation.kind {
            NavigationSharedKind::Waypoint => true,
            NavigationSharedKind::Landmark { id, name } => text(id, 128) && text(name, 1024),
        }
}

impl NavigationSharingState {
    pub fn new(config: &RadarSharingConfig) -> Self {
        Self {
            owner: config.system.clone(),
            share_own_faction: config.share_own_faction,
            receive: config.receive,
            last_tick: None,
            local: BTreeMap::new(),
            received: BTreeMap::new(),
        }
    }
    pub fn validate(&self, tick: u64) -> Result<(), String> {
        if !text(&self.owner.0, 128)
            || self.last_tick.is_some_and(|last| last > tick)
            || self.last_tick.is_none() && (!self.local.is_empty() || !self.received.is_empty())
            || self.local.iter().any(|(key, sample)| {
                key != &item_key(&sample.kind)
                    || sample.provenance.origin.system != self.owner
                    || sample.provenance.received_tick.is_some()
                    || !observation_valid(sample, self.last_tick.unwrap_or(0))
            })
            || self.received.iter().any(|(key, entry)| {
                key != &sample_id(&entry.observation.provenance.origin, 0)
                    || !observation_valid(&entry.observation, self.last_tick.unwrap_or(0))
                    || entry
                        .observation
                        .provenance
                        .received_tick
                        .is_none_or(|first| first > entry.last_delivery_tick)
                    || entry.last_delivery_tick > self.last_tick.unwrap_or(0)
            })
        {
            return Err("invalid Navigation sharing frontier".into());
        }
        Ok(())
    }
    /// Snapshot preflight binds the saved frontier to its actual owning ship
    /// and accepted Navigation System, without restoring authored initial intent.
    pub fn validate_owner(&self, ship: &str, owner: &SystemId, tick: u64) -> Result<(), String> {
        self.validate(tick)?;
        let origin = origin_handle(ship);
        if &self.owner != owner
            || self
                .local
                .values()
                .any(|sample| sample.provenance.origin.source != origin)
            || self
                .received
                .values()
                .any(|entry| entry.observation.provenance.origin.source == origin)
        {
            return Err("Navigation sharing frontier belongs to a different owner".into());
        }
        Ok(())
    }

    /// Replace the currently publishable local points. Held unchanged samples
    /// retain their original time and measuring pose; disappearance is not a new
    /// observation or a revocation of history already received elsewhere.
    pub fn sample(
        &mut self,
        ship: &str,
        tick: u64,
        pose: [f32; 2],
        points: &[NavigationSamplePoint],
    ) -> Result<(), String> {
        self.validate(tick)?;
        if self.last_tick.is_some_and(|last| last > tick)
            || pose.iter().any(|value| !value.is_finite())
        {
            return Err("invalid Navigation sample clock or pose".into());
        }
        let mut local = BTreeMap::new();
        for point in points {
            let kind = &point.kind;
            let position = &point.position;
            let key = item_key(kind);
            if local.contains_key(&key) {
                return Err("duplicate Navigation sample item".into());
            }
            let observation = self
                .local
                .get(&key)
                .filter(|sample| sample.position == *position && sample.kind == *kind)
                .cloned()
                .unwrap_or_else(|| {
                    let origin = RadarSampleOrigin {
                        source: origin_handle(ship),
                        system: self.owner.clone(),
                        contact: key.clone(),
                    };
                    let observed = point.observed_tick.unwrap_or(tick);
                    NavigationSharedObservation {
                        provenance: RadarSampleProvenance {
                            sample_id: sample_id(&origin, tick),
                            origin,
                            observed_tick: observed,
                            observer_position: point
                                .observed_tick
                                .map_or(Some(pose), |_| point.observer_position),
                            velocity_error: 0.0,
                            drift_error_per_sec: 0.0,
                            received_tick: None,
                        },
                        published_tick: tick,
                        position: *position,
                        kind: kind.clone(),
                    }
                });
            if !observation_valid(&observation, tick) {
                return Err("invalid Navigation sample".into());
            }
            local.insert(key, observation);
        }
        let mut next = self.clone();
        next.local = local;
        next.last_tick = Some(tick);
        next.validate(tick)?;
        *self = next;
        Ok(())
    }

    /// Exports originals only. A relay never converts receipt into a new source.
    pub fn exports(&self) -> Vec<NavigationSharedObservation> {
        self.local
            .values()
            .cloned()
            .chain(
                self.received
                    .values()
                    .map(|entry| entry.observation.clone()),
            )
            .collect()
    }

    pub fn accept(
        &mut self,
        ship: &str,
        tick: u64,
        samples: &[NavigationSharedObservation],
    ) -> Result<(), String> {
        self.validate(tick)?;
        if !self.receive {
            return Ok(());
        }
        let mut next = self.clone();
        for sample in samples {
            if !observation_valid(sample, tick) {
                return Err("invalid received Navigation observation".into());
            }
            if sample.provenance.origin.source == origin_handle(ship) {
                continue;
            }
            let key = sample_id(&sample.provenance.origin, 0);
            if next
                .received
                .get(&key)
                .is_some_and(|entry| entry.observation.published_tick > sample.published_tick)
            {
                continue;
            }
            let first = next
                .received
                .get(&key)
                .filter(|entry| {
                    entry.observation.provenance.sample_id == sample.provenance.sample_id
                })
                .and_then(|entry| entry.observation.provenance.received_tick)
                .unwrap_or(tick);
            let mut observation = sample.clone();
            observation.provenance.received_tick = Some(first);
            next.received.insert(
                key,
                ReceivedNavigationObservation {
                    observation,
                    last_delivery_tick: tick,
                },
            );
        }
        next.last_tick = Some(tick);
        next.validate(tick)?;
        *self = next;
        Ok(())
    }
    pub fn projection(&self) -> Vec<NavigationSharedObservation> {
        self.received
            .values()
            .map(|entry| entry.observation.clone())
            .collect()
    }
    pub fn sharing_view(&self, can_operate: bool) -> RadarSharingView {
        RadarSharingView {
            owner: self.owner.clone(),
            share_own_faction: self.share_own_faction,
            receive: self.receive,
            received_samples: self.received.len().min(u32::MAX as usize) as u32,
            can_operate,
        }
    }
}

#[cfg(test)]
#[path = "navigation_sharing_tests.rs"]
mod tests;
