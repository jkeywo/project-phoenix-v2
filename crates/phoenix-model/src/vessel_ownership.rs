//! Whole-vessel acquisition evidence. Physical custody and System control stay
//! with their existing owners; these fields grant no machinery command route.
use crate::activity::ActivityJob;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VesselAcquisitionCause {
    SecurityBoard {
        scope: ActivityJob,
        team: u32,
        target: String,
        accepted_tick: u64,
        work_completed_tick: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VesselOperationalOwnerData {
    pub controller: String,
    pub faction: uuid::Uuid,
    pub acquired_tick: u64,
    pub cause: VesselAcquisitionCause,
    pub doctrine: String,
}
impl VesselOperationalOwnerData {
    pub fn validate(&self, target: &str, now: u64) -> Result<(), String> {
        let identity = crate::comms_channel::identity;
        let VesselAcquisitionCause::SecurityBoard {
            scope,
            team,
            target: acquired,
            accepted_tick,
            work_completed_tick,
        } = &self.cause;
        if !identity(target)
            || !identity(&self.controller)
            || self.controller == target
            || scope.actor != self.controller
            || scope.owner.0 != "security"
            || scope.job.0 == 0
            || *team >= u8::MAX.into()
            || acquired != target
            || self.faction.is_nil()
            || self.acquired_tick != *work_completed_tick
            || accepted_tick > work_completed_tick
            || self.acquired_tick > now
            || !identity(&self.doctrine)
        {
            return Err("invalid whole-vessel acquisition identity/cause/clock".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityAcquisitionOrder {
    pub target: String,
    pub doctrine: String,
    pub accepted_tick: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> VesselOperationalOwnerData {
        VesselOperationalOwnerData {
            controller: "acquirer".into(),
            faction: uuid::Uuid::from_u128(1),
            acquired_tick: 10,
            doctrine: "hold".into(),
            cause: VesselAcquisitionCause::SecurityBoard {
                scope: ActivityJob {
                    actor: "acquirer".into(),
                    owner: crate::identity::SystemId("security".into()),
                    job: crate::job::JobId(4),
                },
                team: 0,
                target: "prize".into(),
                accepted_tick: 2,
                work_completed_tick: 10,
            },
        }
    }
    #[test]
    fn exact_cause_and_original_clocks_are_required() {
        let original = owner();
        original.validate("prize", 10).unwrap();
        for fault in 0..7 {
            let mut bad = original.clone();
            let VesselAcquisitionCause::SecurityBoard {
                scope,
                accepted_tick,
                target,
                ..
            } = &mut bad.cause;
            match fault {
                0 => scope.actor = "other".into(),
                1 => scope.job.0 = 0,
                2 => *accepted_tick = 11,
                3 => *target = "other".into(),
                4 => bad.faction = uuid::Uuid::nil(),
                5 => bad.acquired_tick = 11,
                _ => bad.controller = "prize".into(),
            }
            assert!(bad.validate("prize", 10).is_err());
        }
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(
            serde_json::from_str::<VesselOperationalOwnerData>(&json).unwrap(),
            original
        );
    }
}
