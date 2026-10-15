use super::*;
use bevy::ecs::system::RunSystemOnce;

fn sample(
    world: &mut World,
    aim: FocusedAim,
    intrusive: bool,
    receiver: Vec3,
    observed: Option<Vec3>,
) -> Vec<ScanQuantity> {
    world
        .run_system_once(move |scene: EnquiryScene| {
            scene.sample("subject", aim, intrusive, receiver, observed, 0, 0.1)
        })
        .unwrap()
}
fn subject(world: &mut World, config: &str) -> Entity {
    let config = crate::entities::config::EntityConfig::from_toml(config).unwrap();
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::new(100.0, 0.0, 0.0),
        "subject".into(),
        None,
    );
    world.flush();
    entity
}
#[test]
fn physical_field_reading_samples_permitted_point_without_intrinsic_center_shortcut() {
    let mut world = World::new();
    subject(
        &mut world,
        r#"
[shape]
type="sphere"
radius=10.0
[[enquiry_observation.property]]
property={kind="field_exposure"}
normal=true
point="receiver"
step=0.01
"#,
    );
    assert_eq!(
        sample(
            &mut world,
            FocusedAim::Environment,
            false,
            Vec3::ZERO,
            Some(Vec3::new(100.0, 0.0, 0.0))
        )[0]
        .value,
        ScanQuantityValue::Scalar {
            value: 0.0,
            step: 0.01
        }
    );
    assert_eq!(
        sample(
            &mut world,
            FocusedAim::Environment,
            false,
            Vec3::new(100.0, 0.0, 0.0),
            None
        )[0]
        .value,
        ScanQuantityValue::Scalar {
            value: 1.0,
            step: 0.01
        }
    );
    assert!(sample(
        &mut world,
        FocusedAim::Environment,
        true,
        Vec3::new(100.0, 0.0, 0.0),
        None
    )
    .is_empty());
}
#[test]
fn private_capacity_requires_its_explicit_mode_permission_and_reads_the_actual_store() {
    let mut world = World::new();
    let entity = subject(
        &mut world,
        r#"
[infrastructure]
publish=false
[[infrastructure.capacity]]
id="supplies"
amount=42
label="world.skyhook.berths.label"
[[enquiry_observation.property]]
property={kind="capacity",capacity="supplies"}
intrusive=true
step=1.0
"#,
    );
    assert!(sample(&mut world, FocusedAim::Resources, false, Vec3::ZERO, None).is_empty());
    let answer = sample(&mut world, FocusedAim::Resources, true, Vec3::ZERO, None);
    assert_eq!(answer[0].value, ScanQuantityValue::Count(42));
    assert!(!serde_json::to_string(&answer).unwrap().contains("supplies"));
    world
        .get_mut::<crate::infrastructure::InfrastructureCondition>(entity)
        .unwrap()
        .0
        .adjust_capacity("supplies", -7)
        .unwrap();
    assert_eq!(
        sample(&mut world, FocusedAim::Resources, true, Vec3::ZERO, None)[0].value,
        ScanQuantityValue::Count(35)
    );
    assert_eq!(
        answer[0].value,
        ScanQuantityValue::Count(42),
        "held evidence does not follow the store"
    );
}
#[test]
fn observed_point_is_required_and_protected_properties_do_not_leak_through_anomaly_sampling() {
    let mut world = World::new();
    subject(
        &mut world,
        r#"
[shape]
type="sphere"
radius=10.0
[effects.sensor_blind]
[[enquiry_observation.property]]
property={kind="effect_signature",effect="sensor_blind"}
normal=true
point="observed_subject"
step=1.0
"#,
    );
    assert!(sample(
        &mut world,
        FocusedAim::Anomaly,
        false,
        Vec3::new(100.0, 0.0, 0.0),
        None
    )
    .is_empty());
    assert!(sample(
        &mut world,
        FocusedAim::Anomaly,
        true,
        Vec3::ZERO,
        Some(Vec3::new(100.0, 0.0, 0.0))
    )
    .is_empty());
    assert!(sample(
        &mut world,
        FocusedAim::Anomaly,
        false,
        Vec3::ZERO,
        Some(Vec3::ZERO)
    )
    .is_empty());
    assert_eq!(
        sample(
            &mut world,
            FocusedAim::Anomaly,
            false,
            Vec3::ZERO,
            Some(Vec3::new(100.0, 0.0, 0.0))
        )[0]
        .value,
        ScanQuantityValue::Flag(true)
    );
}
#[test]
fn minimum_positive_precision_preserves_a_finite_physical_reading() {
    assert_eq!(quantise_exposure(1.0, f64::from_bits(1)), 1.0);
    assert_eq!(
        quantise_exposure(f32::MAX, f64::from_bits(1)),
        f64::from(f32::MAX)
    );
    assert_eq!(quantise_exposure(0.0, f64::from_bits(1)), 0.0);
}
