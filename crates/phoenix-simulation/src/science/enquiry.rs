//! Explicit subject permissions at the physical sampling boundary. Controllers
//! receive only the recorded answer, never these policies or physical inputs.
use bevy::{ecs::system::SystemParam, prelude::*};
use phoenix_model::focused_enquiry::*;
use std::collections::BTreeMap;

#[derive(Component, Clone, Debug)]
pub struct EnquirySubject {
    policy: EnquiryObservationConfig,
    capacities: BTreeMap<String, String>,
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(policy) = &config.enquiry_observation {
        commands.insert(EnquirySubject {
            policy: policy.clone(),
            capacities: config
                .infrastructure
                .as_ref()
                .into_iter()
                .flat_map(|config| &config.capacities)
                .filter_map(|row| {
                    row.label
                        .as_ref()
                        .map(|label| (row.id.clone(), label.clone()))
                })
                .collect(),
        });
    }
}

#[derive(SystemParam)]
pub(crate) struct EnquiryScene<'w, 's> {
    subjects: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static Transform,
            &'static EnquirySubject,
            Option<&'static crate::infrastructure::InfrastructureCondition>,
            Option<&'static crate::entities::spawner::RegionShapeSection>,
            Option<&'static crate::regions::field::RegionFieldState>,
            Option<&'static crate::entities::spawner::RegionEffectsSection>,
        ),
    >,
}
impl EnquiryScene<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sample(
        &self,
        subject: &str,
        aim: FocusedAim,
        intrusive: bool,
        receiver: Vec3,
        observed: Option<Vec3>,
        tick: u64,
        dt: f32,
    ) -> Vec<ScanQuantity> {
        let Some((_, transform, permission, condition, shape, field, effects)) =
            self.subjects.iter().find(|(id, ..)| id.0 == subject)
        else {
            return Vec::new();
        };
        let mut quantities = Vec::new();
        for row in &permission.policy.property {
            if row.property.aim() != aim || !(if intrusive { row.intrusive } else { row.normal }) {
                continue;
            }
            let point = match row.point {
                SamplingPoint::Receiver => Some(receiver),
                SamplingPoint::ObservedSubject => observed,
            };
            let exposure = || {
                use crate::regions::field::{local_exposure, FieldFalloff, FieldPose};
                let shape = shape?;
                let pose = match field {
                    Some(field) => field.sample_pose(tick, dt).ok()?,
                    None => FieldPose::stationary(transform.translation),
                };
                Some(local_exposure(
                    &shape.0,
                    pose,
                    field.map_or(FieldFalloff::Step, |field| field.profile.falloff),
                    point?,
                ))
            };
            let quantity = match &row.property {
                EnquiryProperty::FieldExposure => exposure().and_then(|value| {
                    let value = quantise_exposure(value, row.step);
                    value.is_finite().then(|| ScanQuantity {
                        kind: ScanQuantityKind::FieldExposure,
                        label: "component.scan.quantity.field_exposure".into(),
                        value: ScanQuantityValue::Scalar {
                            value,
                            step: row.step,
                        },
                    })
                }),
                EnquiryProperty::Capacity { capacity } => condition.and_then(|condition| {
                    Some(ScanQuantity {
                        kind: ScanQuantityKind::Capacity,
                        label: permission.capacities.get(capacity)?.clone(),
                        value: ScanQuantityValue::Count(condition.0.capacity(capacity)?),
                    })
                }),
                EnquiryProperty::EffectSignature { effect } => {
                    // A field's configured effect is only measurable where it
                    // actually exposes the permitted sampling point.
                    exposure().filter(|value| *value > 0.0).map(|_| {
                        let held = effects.is_some_and(|effects| {
                            effects.0.iter().any(|kind| {
                                crate::regions::effects::region_effect_name(kind).as_str()
                                    == effect.as_str()
                            })
                        });
                        ScanQuantity {
                            kind: ScanQuantityKind::EffectSignature,
                            label: format!("component.scan.effect.{}", effect.as_str()),
                            value: ScanQuantityValue::Flag(held),
                        }
                    })
                }
            };
            if let Some(quantity) = quantity.filter(|quantity| quantity.validate().is_ok()) {
                quantities.push(quantity);
            }
        }
        // Labels are the public quantity identity. Never choose one private
        // store, invent suffixes, or sum stores that cannot be distinguished.
        let mut counts = BTreeMap::new();
        for quantity in &quantities {
            *counts
                .entry((quantity.kind, quantity.label.clone()))
                .or_insert(0usize) += 1;
        }
        quantities.retain(|quantity| counts[&(quantity.kind, quantity.label.clone())] == 1);
        quantities
    }
}

fn quantise_exposure(value: f32, step: f64) -> f64 {
    let value = f64::from(value);
    let ratio = value / step;
    // Below the represented value's resolution the finite physical sample
    // already lies on the representable grid; division must not discard it.
    if ratio.is_finite() {
        ratio.round() * step
    } else {
        value
    }
}

#[cfg(test)]
#[path = "enquiry_tests.rs"]
mod tests;
