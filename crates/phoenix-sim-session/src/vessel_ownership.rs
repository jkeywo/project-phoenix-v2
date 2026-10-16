//! Session owns whole-vessel operational authority, separately from immutable
//! launch slots, Station tenure and each System's ordinary control source.
pub use phoenix_model::vessel_ownership::{VesselAcquisitionCause, VesselOperationalOwnerData};

#[derive(bevy::prelude::Component, Clone, Debug, PartialEq, Eq)]
pub struct VesselOperationalOwner(pub VesselOperationalOwnerData);

impl VesselOperationalOwner {
    pub fn acquire(
        current: Option<&Self>,
        candidate: VesselOperationalOwnerData,
        target: &str,
        now: u64,
    ) -> Result<Self, String> {
        candidate.validate(target, now)?;
        if current.is_some() {
            return Err("vessel already has an operational acquisition owner".into());
        }
        Ok(Self(candidate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_existing_owner_cannot_be_replaced_even_by_identical_repeated_work() {
        let data: VesselOperationalOwnerData = serde_json::from_str(r#"{
            "controller":"a","faction":"aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa","acquired_tick":7,"doctrine":"hold",
            "cause":{"kind":"security_board","scope":{"actor":"a","owner":"security","job":1},"team":0,"target":"b","accepted_tick":2,"work_completed_tick":7}}
        "#).unwrap();
        let owner = VesselOperationalOwner::acquire(None, data.clone(), "b", 7).unwrap();
        assert!(VesselOperationalOwner::acquire(Some(&owner), data, "b", 8).is_err());
    }
}
