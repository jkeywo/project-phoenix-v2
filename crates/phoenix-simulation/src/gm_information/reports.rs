//! Bounded sample-and-hold reports over an observer's allowed Sensors picture.
//! There is no pre-enable history and no current-truth read during projection.
use crate::{
    core::messages::{EntitySnapshot, ModifierSlot},
    entities::spawner::*,
    gm_contact::{self, ContactMode},
    ship::state::ShipPhysics,
    world::server::WorldContentRuntime,
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Immutable mirror of the selected entity's authored Sensors radar. Capture
/// must not depend on which ship this peer happens to present as LocalShip.
#[derive(Component, Clone, Debug)]
pub struct SensorsObservationConfig(pub crate::radar_config::RadarConfig);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportPolicy {
    pub delay_ticks: u32,
    pub position_step_mm: u32,
    pub hide_identity: bool,
}
impl ReportPolicy {
    pub fn changes_report(&self) -> bool {
        self.delay_ticks != 0 || self.position_step_mm != 0 || self.hide_identity
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReportSample {
    pub observed_tick: u64,
    pub position_mm: [i64; 3],
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReportState {
    pub policy: ReportPolicy,
    pub next_tick: Option<u64>,
    pub pending: Option<ReportSample>,
    pub presented: Option<ReportSample>,
}
impl ReportState {
    pub fn new(policy: ReportPolicy) -> Self {
        Self {
            policy,
            next_tick: None,
            pending: None,
            presented: None,
        }
    }
    pub fn clear_samples(&mut self) {
        self.next_tick = None;
        self.pending = None;
        self.presented = None;
    }
    /// On each explicit interval, release the old allowed sample and capture
    /// the next. A missing observation releases a missing report at that same
    /// cadence, rather than retaining a lost target indefinitely.
    pub fn advance(&mut self, tick: u64, observe: impl FnOnce() -> Option<ReportSample>) {
        if self.policy.delay_ticks == 0 {
            self.presented = observe();
            self.pending = None;
            return;
        }
        if self.next_tick.is_some_and(|next| tick < next) {
            return;
        }
        self.presented = self.pending.take();
        self.pending = observe();
        self.next_tick = Some(tick.saturating_add(u64::from(self.policy.delay_ticks)));
    }
}
pub type Reports = BTreeMap<String, BTreeMap<String, ReportState>>;

/// Public observation metadata, with no GM identity, policy or palette key.
pub use phoenix_model::wire::SensorReport;
pub fn projection(
    reports: &Reports,
    observer: &str,
    tick: u64,
) -> BTreeMap<String, Option<SensorReport>> {
    reports
        .get(observer)
        .into_iter()
        .flat_map(|rows| rows.iter())
        .map(|(target, state)| {
            (
                target.clone(),
                state.presented.as_ref().map(|sample| SensorReport {
                    observed_tick: sample.observed_tick,
                    age_ticks: tick.saturating_sub(sample.observed_tick),
                    source: "console.sensors.report_source".into(),
                    name: sample.name.clone(),
                    position_mm: sample.position_mm,
                }),
            )
        })
        .collect()
}
pub fn contains(reports: &Reports, observer: &str, target: &str) -> bool {
    reports
        .get(observer)
        .is_some_and(|rows| rows.contains_key(target))
}
pub fn set(
    reports: &mut Reports,
    observer: &str,
    target: &str,
    policy: Option<ReportPolicy>,
) -> bool {
    super::pair_map::set(
        reports,
        observer,
        target,
        policy,
        ReportState::new,
        |state| &state.policy,
    )
}

struct Observation {
    name: Option<String>,
    position: [f32; 3],
    tags: Vec<String>,
    point: bool,
    radar: Option<crate::radar_config::RadarConfig>,
    range_multiplier: f32,
    observer: bool,
}

/// Fixed Publish runs this before either Sensors publisher. Positions come
/// from canonical ShipPhysics for ships, never their render interpolation.
pub fn advance(world: &mut World) {
    if world
        .get_resource::<WorldContentRuntime>()
        .is_none_or(|runtime| runtime.contact_information.reports.is_empty())
    {
        return;
    }
    let tick = world.resource::<crate::sim_tick::SimTick>().0;
    let mut query = world.query::<(
        &EntityUuid,
        Option<&EntityName>,
        Option<&EntityId>,
        Option<&Transform>,
        Option<&ShipPhysics>,
        Option<&EntityTagsSection>,
        Option<&RadarAppearanceSection>,
        Option<&SensorsObservationConfig>,
        Option<&crate::modifiers::ShipModifiers>,
        Has<crate::lockstep::FleetSlotOf>,
        Has<crate::server_app::Ship>,
    )>();
    let observations: BTreeMap<String, Observation> = query
        .iter(world)
        .map(
            |(
                id,
                name,
                fallback,
                transform,
                physics,
                tags,
                radar,
                config,
                modifiers,
                fleet,
                ship,
            )| {
                let position = physics
                    .map(|p| [p.x, p.y, p.z])
                    .or_else(|| transform.map(|t| t.translation.to_array()))
                    .unwrap_or([f32::NAN; 3]);
                (
                    id.0.clone(),
                    Observation {
                        name: name
                            .map(|name| name.0.clone())
                            .or_else(|| fallback.map(|id| id.0.clone())),
                        position,
                        tags: tags.map(|tags| tags.0.clone()).unwrap_or_default(),
                        point: radar.is_some_and(|radar| {
                            radar.0.icon.is_some() || radar.0.region_colour.is_some()
                        }),
                        radar: config.map(|config| config.0.clone()),
                        range_multiplier: modifiers
                            .map_or(1.0, |m| m.get(&ModifierSlot::SensorRadarRange)),
                        observer: fleet && ship,
                    },
                )
            },
        )
        .collect();
    let legacy_by_observer: BTreeMap<String, Vec<_>> = world
        .get_resource::<crate::world::server::ObjectiveManagerRes>()
        .map(|manager| {
            observations
                .iter()
                .filter(|(_, value)| value.observer)
                .map(|(id, _)| (id.clone(), manager.0.snapshots_for(id)))
                .collect()
        })
        .unwrap_or_default();
    let observer_targets: BTreeMap<String, Vec<String>> = legacy_by_observer
        .into_iter()
        .map(|(id, rows)| {
            let rows = world
                .get_resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .map(|instances| instances.0.project_snapshots_for_ship(&id, rows.clone()))
                .unwrap_or(rows);
            (id, rows.into_iter().flat_map(|row| row.targets).collect())
        })
        .collect();
    let mut runtime = world.resource_mut::<WorldContentRuntime>();
    let WorldContentRuntime {
        contact_information,
        contact_overrides,
        contact_classifications,
        ..
    } = &mut *runtime;
    contact_information.reports.retain(|observer, rows| {
        let Some(own) = observations.get(observer).filter(|value| value.observer) else {
            return false;
        };
        rows.retain(|target, state| {
            let Some(entity) = observations.get(target) else {
                return false;
            };
            let mode = gm_contact::mode(contact_overrides, observer, target);
            if mode == ContactMode::Conceal {
                state.clear_samples();
                return true;
            }
            let policy = state.policy.clone();
            state.advance(tick, || {
                if !entity
                    .position
                    .iter()
                    .chain(own.position.iter())
                    .all(|value| value.is_finite())
                {
                    return None;
                }
                let ordinary = own.radar.as_ref().is_some_and(|radar| {
                    crate::simmath::hypot(
                        entity.position[0] - own.position[0],
                        entity.position[2] - own.position[2],
                    ) <= radar.range * own.range_multiplier
                        && entity
                            .tags
                            .iter()
                            .any(|tag| radar.shows.iter().any(|show| show.as_str() == tag))
                        && entity.point
                        && (!entity.tags.iter().any(|tag| tag == "objective_marker")
                            || observer_targets
                                .get(observer)
                                .is_some_and(|targets| targets.contains(target)))
                });
                if mode != ContactMode::Reveal && !ordinary {
                    return None;
                }
                let supplied = contact_classifications
                    .get(observer)
                    .and_then(|rows| rows.get(target));
                let name = if policy.hide_identity {
                    "console.sensors.basic_contact".into()
                } else if let Some(supplied) = supplied {
                    supplied.label.clone()
                } else if ordinary {
                    entity
                        .name
                        .clone()
                        .unwrap_or_else(|| "console.sensors.basic_contact".into())
                } else {
                    "console.sensors.basic_contact".into()
                };
                let position_mm = entity.position.map(|position| {
                    let value = (f64::from(position) * 1000.0) as i64;
                    let step = i64::from(policy.position_step_mm);
                    if step == 0 {
                        value
                    } else {
                        value.div_euclid(step).saturating_mul(step)
                    }
                });
                Some(ReportSample {
                    observed_tick: tick,
                    position_mm,
                    name,
                })
            });
            true
        });
        !rows.is_empty()
    });
}

pub fn viewscreen(
    entities: &[EntitySnapshot],
    reports: &Reports,
    overrides: &gm_contact::ContactOverrides,
    observer: &str,
    tick: u64,
    x: f32,
    z: f32,
    range: f32,
) -> Vec<EntitySnapshot> {
    let rows = projection(reports, observer, tick);
    let mut result: Vec<_> = entities
        .iter()
        .filter(|entity| !rows.contains_key(&entity.uuid))
        .cloned()
        .collect();
    for (target, report) in rows {
        if gm_contact::mode(overrides, observer, &target) == ContactMode::Conceal {
            continue;
        }
        if let Some(report) = report {
            let snapshot = EntitySnapshot {
                uuid: target.clone(),
                name: Some(report.name.clone()),
                position: Some(report.position_mm.map(|value| value as f32 / 1000.0)),
                ..Default::default()
            };
            let mut points = gm_contact::viewscreen_contacts(
                &[snapshot],
                &BTreeMap::from([(target, ContactMode::Reveal)]),
                x,
                z,
                range,
                &Default::default(),
            );
            points[0].name = Some(report.name);
            result.extend(points);
        }
    }
    result
}

#[cfg(test)]
#[path = "reports_tests.rs"]
mod tests;
