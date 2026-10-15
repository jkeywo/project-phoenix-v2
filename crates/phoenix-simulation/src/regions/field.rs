pub use phoenix_sim_gameplay::regions::field::*;
/// Presentation cache of the current logical field pose, rebuilt by Regions.
#[derive(bevy::prelude::Component, Clone, Copy, Debug)]
pub struct SampledRegionFieldPose(pub FieldPose);
