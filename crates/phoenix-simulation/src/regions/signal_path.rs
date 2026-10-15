use crate::{
    entities::spawner::{EntityUuid, RegionShapeSection},
    regions::field::{FieldPose, RegionFieldState},
    sim_tick::SimTick,
};
use bevy::{ecs::system::SystemParam, prelude::*};
pub use phoenix_sim_gameplay::regions::signal_path::*;

/// Read-only physical scene, shared by actual instrument/Comms producers.
/// Neither controllers nor transport gain world knowledge through this query.
#[derive(SystemParam)]
pub struct RegionSignalQuery<'w, 's> {
    volumes: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static Transform,
            &'static RegionShapeSection,
            Option<&'static RegionFieldState>,
            Option<&'static SignalMaterialConfig>,
        ),
    >,
    jamming: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static crate::comms::jamming::JammingEmissions,
        ),
    >,
    tick: Option<Res<'w, SimTick>>,
    fixed: Option<Res<'w, Time<Fixed>>>,
}

impl RegionSignalQuery<'_, '_> {
    /// Endpoint exposure only. Natural segment transmission below is unchanged.
    pub(crate) fn comms_receiver_factor(&self, point: Vec3) -> f32 {
        crate::comms::jamming::receiver_factor(
            self.jamming.iter().collect(),
            self.tick.as_ref().map(|tick| tick.0),
            self,
            point,
        )
        .unwrap_or(0.0)
    }

    pub(crate) fn comms_receiver_interference(&self, point: Vec3) -> Option<bool> {
        crate::comms::jamming::receiver_factor(
            self.jamming.iter().collect(),
            self.tick.as_ref().map(|tick| tick.0),
            self,
            point,
        )
        .map(|factor| factor < 1.0)
    }
    pub fn transmission(&self, start: Vec3, end: Vec3, class: SignalClass) -> f32 {
        let mut volumes = Vec::new();
        for (id, transform, shape, field, material) in &self.volumes {
            if field.is_none() && material.is_none() {
                continue;
            }
            let pose = if let Some(field) = field {
                let (Some(tick), Some(fixed)) = (&self.tick, &self.fixed) else {
                    return 0.0;
                };
                let Ok(pose) = field.sample_pose(tick.0, fixed.timestep().as_secs_f32()) else {
                    return 0.0;
                };
                pose
            } else {
                FieldPose::stationary(transform.translation)
            };
            volumes.push(SignalVolume {
                id: &id.0,
                shape: &shape.0,
                pose,
                field: field.map(|state| &state.profile),
                material,
            });
        }
        signal_transmission(volumes, start, end, class)
    }
}
