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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorReport {
    pub observed_tick: u64,
    pub age_ticks: u64,
    pub source: String,
    pub name: String,
    pub position_mm: [i64; 3],
}
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
    if reports
        .get(observer)
        .and_then(|rows| rows.get(target))
        .map(|state| &state.policy)
        == policy.as_ref()
    {
        return false;
    }
    if let Some(policy) = policy {
        reports
            .entry(observer.into())
            .or_default()
            .insert(target.into(), ReportState::new(policy));
    } else if let Some(rows) = reports.get_mut(observer) {
        rows.remove(target);
        if rows.is_empty() {
            reports.remove(observer);
        }
    }
    true
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
    let observer_targets: BTreeMap<String, Vec<String>> = world
        .get_resource::<crate::world::server::ObjectiveManagerRes>()
        .map(|manager| {
            observations
                .iter()
                .filter(|(_, value)| value.observer)
                .map(|(id, _)| {
                    (
                        id.clone(),
                        manager
                            .0
                            .snapshots_for(id)
                            .into_iter()
                            .flat_map(|row| row.targets)
                            .collect(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
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
mod tests {
    use super::*;
    fn policy(delay_ticks: u32) -> ReportPolicy {
        ReportPolicy {
            delay_ticks,
            position_step_mm: 0,
            hide_identity: false,
        }
    }
    fn sample(tick: u64) -> Option<ReportSample> {
        Some(ReportSample {
            observed_tick: tick,
            name: "observed".into(),
            position_mm: [tick as i64, 0, 0],
        })
    }
    #[test]
    fn delayed_report_withholds_initial_truth_and_releases_absence_at_the_next_interval() {
        let mut state = ReportState::new(policy(4));
        state.advance(10, || sample(10));
        assert!(state.presented.is_none());
        state.advance(13, || panic!("no sample between authored boundaries"));
        state.advance(14, || sample(14));
        assert_eq!(state.presented, sample(10));
        state.advance(18, || None);
        assert_eq!(state.presented, sample(14));
        state.advance(22, || None);
        assert!(state.presented.is_none());
        state.clear_samples();
        state.advance(25, || sample(25));
        assert!(state.presented.is_none());
    }
    #[test]
    fn reconfiguring_clears_samples_but_identical_policy_keeps_pending_work_and_roundtrips() {
        let mut reports = Reports::new();
        assert!(set(&mut reports, "a", "b", Some(policy(4))));
        reports
            .get_mut("a")
            .unwrap()
            .get_mut("b")
            .unwrap()
            .advance(10, || sample(10));
        assert!(!set(&mut reports, "a", "b", Some(policy(4))));
        let saved = reports.clone();
        let codec = vellum_digest::ShareCodec::new("REPORT-TEST-");
        assert_eq!(
            codec
                .decode::<Reports>(&codec.encode(&saved).unwrap())
                .unwrap(),
            saved
        );
        assert!(set(&mut reports, "a", "b", Some(policy(8))));
        assert!(reports["a"]["b"].pending.is_none());
        assert!(projection(&reports, "other", 10).is_empty());
        assert!(set(&mut reports, "a", "b", None));
        assert!(reports.is_empty());
    }
    fn world() -> (World, Entity, Entity) {
        let mut world = World::new();
        world.insert_resource(WorldContentRuntime::default());
        world.insert_resource(crate::sim_tick::SimTick(10));
        let observer = world
            .spawn((
                EntityUuid("a".into()),
                crate::server_app::Ship,
                crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot(1)),
                ShipPhysics::default(),
                SensorsObservationConfig(crate::radar_config::RadarConfig {
                    range: 100.0,
                    shows: vec![crate::entities::tags::EntityTag::Ship],
                    selects: vec![],
                }),
            ))
            .id();
        let target = world
            .spawn((
                EntityUuid("b".into()),
                EntityName("true-name".into()),
                EntityTagsSection(vec!["ship".into()]),
                Transform::from_xyz(5000.0, 0.0, 0.0),
                ShipPhysics {
                    x: 12.25,
                    z: -9.75,
                    ..Default::default()
                },
                RadarAppearanceSection(toml::from_str("icon = 'ship'").unwrap()),
            ))
            .id();
        (world, observer, target)
    }
    #[test]
    fn capture_uses_own_radar_canonical_pose_and_observed_identity_with_conceal_floor() {
        let (mut world, observer, target) = world();
        let policy = ReportPolicy {
            position_step_mm: 5000,
            ..policy(2)
        };
        set(
            &mut world
                .resource_mut::<WorldContentRuntime>()
                .contact_information
                .reports,
            "a",
            "b",
            Some(policy),
        );
        advance(&mut world);
        assert!(world
            .resource::<WorldContentRuntime>()
            .contact_information
            .reports["a"]["b"]
            .presented
            .is_none());
        world.entity_mut(target).get_mut::<EntityName>().unwrap().0 = "new-name".into();
        world.resource_mut::<crate::sim_tick::SimTick>().0 = 12;
        advance(&mut world);
        let public = projection(
            &world
                .resource::<WorldContentRuntime>()
                .contact_information
                .reports,
            "a",
            13,
        );
        let row = public["b"].as_ref().unwrap();
        assert_eq!(row.name, "true-name");
        assert_eq!(row.position_mm, [10000, 0, -10000]);
        assert_eq!(row.age_ticks, 3);
        world
            .entity_mut(observer)
            .insert(crate::server_app::LocalShip); // presentation ownership cannot alter allowed capture
        gm_contact::set(
            &mut world
                .resource_mut::<WorldContentRuntime>()
                .contact_overrides,
            "a",
            "b",
            ContactMode::Conceal,
        );
        advance(&mut world);
        assert!(world
            .resource::<WorldContentRuntime>()
            .contact_information
            .reports["a"]["b"]
            .pending
            .is_none());
        gm_contact::set(
            &mut world
                .resource_mut::<WorldContentRuntime>()
                .contact_overrides,
            "a",
            "b",
            ContactMode::Normal,
        );
        world.entity_mut(target).get_mut::<ShipPhysics>().unwrap().x = 500.0;
        advance(&mut world);
        assert!(world
            .resource::<WorldContentRuntime>()
            .contact_information
            .reports["a"]["b"]
            .pending
            .is_none());
        gm_contact::set(
            &mut world
                .resource_mut::<WorldContentRuntime>()
                .contact_overrides,
            "a",
            "b",
            ContactMode::Reveal,
        );
        world.resource_mut::<crate::sim_tick::SimTick>().0 = 14;
        advance(&mut world);
        assert_eq!(
            world
                .resource::<WorldContentRuntime>()
                .contact_information
                .reports["a"]["b"]
                .pending
                .as_ref()
                .unwrap()
                .name,
            "console.sensors.basic_contact"
        );
        world.despawn(target);
        advance(&mut world);
        assert!(world
            .resource::<WorldContentRuntime>()
            .contact_information
            .reports
            .is_empty());
    }
    #[test]
    fn native_projection_never_enriches_a_sample_with_current_truth_or_concealed_data() {
        let mut reports = Reports::new();
        set(&mut reports, "a", "b", Some(policy(0)));
        reports
            .get_mut("a")
            .unwrap()
            .get_mut("b")
            .unwrap()
            .advance(10, || sample(10));
        let truth = [EntitySnapshot {
            uuid: "b".into(),
            name: Some("secret".into()),
            position: Some([90.0, 0.0, 0.0]),
            tags: vec!["ship".into()],
            ..Default::default()
        }];
        let projected = viewscreen(
            &truth,
            &reports,
            &Default::default(),
            "a",
            12,
            0.0,
            0.0,
            100.0,
        );
        assert_eq!(projected[0].name.as_deref(), Some("observed"));
        assert_eq!(projected[0].position, Some([0.01, 0.0, 0.0]));
        assert!(!projected[0].tags.contains(&"ship".into()));
        let mut modes = Default::default();
        gm_contact::set(&mut modes, "a", "b", ContactMode::Conceal);
        assert!(viewscreen(&truth, &reports, &modes, "a", 12, 0.0, 0.0, 100.0).is_empty());
        assert_eq!(
            viewscreen(&truth, &reports, &modes, "other", 12, 0.0, 0.0, 100.0),
            truth
        );
    }
}
