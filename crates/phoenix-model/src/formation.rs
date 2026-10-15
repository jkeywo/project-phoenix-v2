//! A received coordination reference and one dated Navigation course cache.
//! Neither sender identity nor another ship's physical state is inferred here.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

fn bounded(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationRequest {
    pub message_id: String,
    pub origin_id: String,
    pub source_name: String,
    pub sequence: u64,
    pub slot: [i32; 2],
    pub accepted_tick: u64,
}
impl FormationRequest {
    pub fn valid_at(&self, now: u64) -> bool {
        bounded(&self.message_id)
            && bounded(&self.origin_id)
            && bounded(&self.source_name)
            && self.sequence > 0
            && self.accepted_tick <= now
            && self
                .slot
                .iter()
                .all(|axis| axis.unsigned_abs() <= 1_000_000)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationMeasurement {
    /// Measured leader position in the existing World x/z plane.
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub observed_tick: u64,
}
impl FormationMeasurement {
    pub fn valid_at(&self, now: u64) -> bool {
        self.observed_tick <= now
            && self
                .position
                .iter()
                .chain(&self.velocity)
                .all(|axis| axis.is_finite())
    }
    pub fn slot_point(&self, request: &FormationRequest) -> Option<[f32; 2]> {
        let point = std::array::from_fn(|axis| self.position[axis] + request.slot[axis] as f32);
        point.iter().all(|axis| axis.is_finite()).then_some(point)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationCourse {
    pub owner: SystemId,
    pub request: FormationRequest,
    /// Earned observer-local public contact handle, never the sender route UUID.
    pub leader: String,
    pub selected_tick: u64,
    pub measurement: FormationMeasurement,
    /// The dated cache survives loss; a paused course is not an executable goal.
    pub paused: bool,
}
impl FormationCourse {
    pub fn valid_at(&self, now: u64) -> bool {
        bounded(&self.owner.0)
            && bounded(&self.leader)
            && self.request.valid_at(now)
            && self.request.accepted_tick <= self.selected_tick
            && self.selected_tick <= now
            && self.measurement.valid_at(now)
            && self.measurement.slot_point(&self.request).is_some()
    }
    pub fn same_intent(&self, other: &Self) -> bool {
        self.owner == other.owner && self.request == other.request && self.leader == other.leader
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationLeaderChoice {
    pub contact: String,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationNavigationView {
    pub suggestions: Vec<FormationRequest>,
    pub leaders: Vec<FormationLeaderChoice>,
    pub course: Option<FormationCourse>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_dated_course_and_bounded_world_slot_round_trip_without_private_identity() {
        let course = FormationCourse {
            owner: SystemId("navigation".into()),
            leader: "own/contact".into(),
            selected_tick: 12,
            request: FormationRequest {
                message_id: "received".into(),
                origin_id: "issued".into(),
                source_name: "public callsign".into(),
                sequence: 1,
                slot: [-40, 80],
                accepted_tick: 10,
            },
            measurement: FormationMeasurement {
                position: [100.0, 200.0],
                velocity: [3.0, 4.0],
                observed_tick: 11,
            },
            paused: false,
        };
        assert!(course.valid_at(12));
        assert_eq!(
            course.measurement.slot_point(&course.request),
            Some([60.0, 280.0])
        );
        assert_eq!(
            serde_json::from_str::<FormationCourse>(&serde_json::to_string(&course).unwrap())
                .unwrap(),
            course
        );
        let mut invalid = course.clone();
        invalid.request.slot[0] = i32::MIN;
        assert!(!invalid.valid_at(12));
        let mut invalid = course.clone();
        invalid.measurement.observed_tick = 13;
        assert!(!invalid.valid_at(12));
        let mut invalid = course;
        invalid.measurement.velocity[0] = f32::NAN;
        assert!(!invalid.valid_at(12));
    }
}
