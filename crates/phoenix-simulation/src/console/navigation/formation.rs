//! Received coordination becomes an own course only through Navigation admission.
use super::{knowledge::Environment, server::WaypointMode};
use crate::{
    core::messages::SystemId, server_app::ShipSystemBlackboards, ship_plugin::ShipConfigComponent,
};
use phoenix_model::formation::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LeaderReading {
    pub name: Option<String>,
    pub measurement: FormationMeasurement,
}
pub(crate) fn requests(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    actor: &str,
) -> Vec<FormationRequest> {
    crate::comms::formation::accepted(config, boards, owner, actor)
        .into_iter()
        .map(|received| FormationRequest {
            message_id: received.message_id,
            origin_id: received.origin_id,
            source_name: received.source_name,
            sequence: received.sequence,
            slot: received.slot,
            accepted_tick: received.accepted_tick,
        })
        .collect()
}
pub(crate) fn select(
    owner: &SystemId,
    request: &FormationRequest,
    leader: &str,
    readings: &BTreeMap<String, LeaderReading>,
    now: u64,
) -> Option<FormationCourse> {
    let measurement = readings.get(leader)?.measurement.clone();
    let course = FormationCourse {
        owner: owner.clone(),
        request: request.clone(),
        leader: leader.into(),
        selected_tick: now,
        measurement,
        paused: false,
    };
    course.valid_at(now).then_some(course)
}
pub(crate) fn correlated<'a>(
    request: &FormationRequest,
    readings: &'a BTreeMap<String, LeaderReading>,
) -> Option<&'a str> {
    // Independently matured public identity only. Private source-route IDs are
    // not present in this projection and therefore cannot settle ambiguity.
    let mut matches = readings
        .iter()
        .filter(|(_, reading)| reading.name.as_deref() == Some(request.source_name.as_str()));
    let (handle, _) = matches.next()?;
    matches.next().is_none().then_some(handle.as_str())
}
pub(crate) fn key(course: &FormationCourse) -> String {
    format!(
        "formation:{}:{}:{}",
        course.request.message_id.len(),
        course.request.message_id,
        course.leader
    )
}
pub(crate) fn refresh(
    course: &mut FormationCourse,
    requests: &[FormationRequest],
    readings: &BTreeMap<String, LeaderReading>,
    paid: bool,
) {
    course.paused = true;
    if !paid || !requests.contains(&course.request) {
        return;
    }
    if let Some(reading) = readings.get(&course.leader) {
        course.measurement = reading.measurement.clone();
        course.paused = false;
    }
}
pub(crate) fn view(
    requests: Vec<FormationRequest>,
    readings: &BTreeMap<String, LeaderReading>,
    mode: Option<&WaypointMode>,
) -> Option<FormationNavigationView> {
    let course = match mode {
        Some(WaypointMode::Formation { course }) => Some(course.clone()),
        _ => None,
    };
    if requests.is_empty() && course.is_none() {
        return None;
    }
    Some(FormationNavigationView {
        suggestions: requests,
        leaders: readings
            .iter()
            .map(|(contact, reading)| FormationLeaderChoice {
                contact: contact.clone(),
                name: reading.name.clone(),
            })
            .collect(),
        course,
    })
}
pub(crate) fn request_for_command(
    knowledge: &Environment,
    entity: bevy::prelude::Entity,
    config: Option<&ShipConfigComponent>,
    boards: Option<&ShipSystemBlackboards>,
    owner: &SystemId,
    request_id: &str,
    leader: &str,
) -> Option<FormationCourse> {
    let config = config?;
    let boards = boards?;
    let actor = knowledge.actor(entity)?;
    let requests = requests(config, boards, owner, actor);
    let request = requests
        .iter()
        .find(|request| request.message_id == request_id)?;
    select(
        owner,
        request,
        leader,
        &knowledge.formation_readings(entity, config, boards, owner),
        knowledge.now()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> FormationRequest {
        FormationRequest {
            message_id: "received/1".into(),
            origin_id: "sent/1".into(),
            source_name: "Observed leader".into(),
            sequence: 1,
            slot: [-40, 80],
            accepted_tick: 10,
        }
    }
    fn readings() -> BTreeMap<String, LeaderReading> {
        [(
            "earned/contact".into(),
            LeaderReading {
                name: Some("Observed leader".into()),
                measurement: FormationMeasurement {
                    position: [100.0, 200.0],
                    velocity: [3.0, 4.0],
                    observed_tick: 12,
                },
            },
        )]
        .into()
    }
    #[test]
    fn correlation_requires_unique_independently_held_name_and_never_a_route_uuid() {
        let request = request();
        let mut readings = readings();
        assert_eq!(correlated(&request, &readings), Some("earned/contact"));
        readings.values_mut().next().unwrap().name = None;
        assert!(correlated(&request, &readings).is_none());
        readings = self::readings();
        readings.insert("another/contact".into(), readings["earned/contact"].clone());
        assert!(correlated(&request, &readings).is_none());
    }
    #[test]
    fn measured_refresh_and_pause_preserve_semantic_course_generation_and_dated_cache() {
        let owner = SystemId("navigation".into());
        let request = request();
        let mut readings = readings();
        let course = select(&owner, &request, "earned/contact", &readings, 12).unwrap();
        let mut waypoint =
            super::super::server::NavigationWaypoint::new(WaypointMode::Formation { course });
        let generation = waypoint.generation();
        readings.get_mut("earned/contact").unwrap().measurement = FormationMeasurement {
            position: [110.0, 220.0],
            velocity: [3.0, 4.0],
            observed_tick: 13,
        };
        let mut updated = select(&owner, &request, "earned/contact", &readings, 13).unwrap();
        waypoint.set(WaypointMode::Formation {
            course: updated.clone(),
        });
        assert_eq!(waypoint.generation(), generation);
        assert_eq!(waypoint.canonical_snapshot().unwrap().x, 70.0);
        refresh(&mut updated, &[request.clone()], &BTreeMap::new(), true);
        waypoint.set(WaypointMode::Formation {
            course: updated.clone(),
        });
        assert!(waypoint.snapshot().is_none());
        assert_eq!(waypoint.generation(), generation);
        assert_eq!(updated.measurement.observed_tick, 13);
        assert_eq!(waypoint.canonical_snapshot().unwrap().z, 300.0);
        refresh(&mut updated, &[request], &readings, true);
        assert!(!updated.paused);
        waypoint.set(WaypointMode::Formation { course: updated });
        assert!(waypoint.snapshot().is_some());
        assert_eq!(waypoint.generation(), generation);
        waypoint.clear();
        assert_ne!(waypoint.generation(), generation);
    }
}
