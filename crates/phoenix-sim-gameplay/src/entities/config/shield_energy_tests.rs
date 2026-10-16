use super::*;
use crate::entities::config::{EntityConfig, ShieldArcEnergyConfig};
use crate::modifiers::power_system::{PowerConfig, PowerSystem};

fn arc(id: &str, cost: f32, targets: &[&str]) -> ShieldArcConfig {
    let mut value: toml::Value = toml::from_str(&format!(
        "id='{id}'\nlabel='Arc'\ncenter_deg=0.0\nwidth_deg=180.0\n[energy]\nupkeep_per_sec=0.05\nrecharge_energy_per_hp={cost}\nrecovery_energy_per_hp=0.25\ndischarge_hp_per_sec=5.0\nrecovery_targets={:?}\n",
        targets,
    ))
    .unwrap();
    value.as_table_mut().unwrap().insert(
        "ai".into(),
        toml::from_str(include_str!(
            "../../../../../tests/fixtures/shield-arc-ai.toml"
        ))
        .unwrap(),
    );
    value.try_into().unwrap()
}

fn hardware(arcs: &[ShieldArcConfig]) -> (ShipConfig, EnergyConfigSection) {
    let template = PowerSystem::new(&PowerConfig::default());
    let mut ship = template.ship_config().clone();
    let mut energy = template.config().clone();
    ship.systems.truncate(arcs.len());
    energy.consumers.truncate(arcs.len());
    for ((system, consumer), arc) in ship.systems.iter_mut().zip(&mut energy.consumers).zip(arcs) {
        let id = shield_arc_system_id(&arc.id).unwrap();
        system.id = id.clone();
        system.kind = SHIELD_ARC_KIND.into();
        consumer.system = id;
    }
    (ship, energy)
}

#[test]
fn authored_arc_requires_explicit_work_data_and_rejects_unknown_coefficients() {
    let error = EntityConfig::from_toml(
        "[[shield_arc]]\nid='fore'\nlabel='Fore'\ncenter_deg=0.0\nwidth_deg=90.0\n",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("increment 9") && error.contains("shield_arc.energy"));
    let source = toml::to_string(&arc("fore", 0.5, &[])).unwrap();
    assert!(toml::from_str::<ShieldArcConfig>(&source.replace(
        "upkeep_per_sec = 0.05",
        "upkeep_per_sec = 0.05\nenergy_capacity = 100.0",
    ))
    .is_err());
}

#[test]
fn work_coefficients_are_finite_and_recovery_cannot_repay_its_source_cost() {
    let config = ShieldArcEnergyConfig::default();
    for invalid in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(ShieldArcEnergyConfig {
            upkeep_per_sec: invalid,
            ..config.clone()
        }
        .validate()
        .is_err());
        assert!(ShieldArcEnergyConfig {
            discharge_hp_per_sec: invalid,
            ..config.clone()
        }
        .validate()
        .is_err());
    }
    assert!(ShieldArcEnergyConfig {
        recharge_energy_per_hp: 0.0,
        ..config.clone()
    }
    .validate()
    .is_err());
    assert!(ShieldArcEnergyConfig {
        recovery_energy_per_hp: 0.5,
        ..config.clone()
    }
    .validate()
    .is_err());
    assert!(ShieldArcEnergyConfig {
        upkeep_per_sec: 0.0,
        recovery_energy_per_hp: 0.0,
        ..config.clone()
    }
    .validate()
    .is_ok());
    assert!(!ShieldArcEnergyConfig {
        discharge_hp_per_sec: 0.0,
        ..config
    }
    .recovery_enabled());
}

#[test]
fn actual_canonical_destinations_are_required_and_duplicate_targets_refused() {
    let mut arcs = vec![arc("fore", 1.0, &["shield-arc-aft"]), arc("aft", 0.5, &[])];
    let (mut ship, mut energy) = hardware(&arcs);
    validate_shield_arc_energy(&arcs, &ship, &energy).unwrap();
    energy.consumers.pop();
    assert!(validate_shield_arc_energy(&arcs, &ship, &energy)
        .unwrap_err()
        .contains("canonical energy reserve"));
    let (_, energy) = hardware(&arcs);
    ship.systems[1].kind = "dock".into();
    assert!(validate_shield_arc_energy(&arcs, &ship, &energy).is_err());
    let (ship, energy) = hardware(&arcs);
    arcs[0]
        .energy
        .recovery_targets
        .push(SystemId("shield-arc-aft".into()));
    assert!(validate_shield_arc_energy(&arcs, &ship, &energy)
        .unwrap_err()
        .contains("repeats"));
    arcs[0].energy.recovery_targets = vec![SystemId("foreign-ship::shield-arc-aft".into())];
    assert!(validate_shield_arc_energy(&arcs, &ship, &energy)
        .unwrap_err()
        .contains("unknown shield arc"));
}

#[test]
fn heterogeneous_recipient_cost_prevents_cross_arc_energy_cycles() {
    let mut arcs = vec![arc("fore", 1.0, &["shield-arc-aft"]), arc("aft", 0.2, &[])];
    arcs[1].energy.recovery_energy_per_hp = 0.0;
    let (ship, energy) = hardware(&arcs);
    assert!(validate_shield_arc_energy(&arcs, &ship, &energy)
        .unwrap_err()
        .contains("recipient shield-arc-aft"));
    arcs[0].energy.recovery_energy_per_hp = 0.1;
    validate_shield_arc_energy(&arcs, &ship, &energy).unwrap();
}

#[test]
fn work_configuration_survives_authored_and_runtime_conversion() {
    let arc = arc("fore", 0.75, &["shield-arc-aft"]);
    let source = toml::to_string(&arc).unwrap();
    assert_eq!(toml::from_str::<ShieldArcConfig>(&source).unwrap(), arc);
    assert_eq!(arc.to_runtime().energy, arc.energy);
}

#[test]
fn focus_availability_defaults_on_and_round_trips_explicit_opt_out() {
    let enabled = arc("fore", 0.5, &[]);
    assert!(enabled.focus_available);
    let mut value = toml::Value::try_from(&enabled).unwrap();
    value
        .as_table_mut()
        .unwrap()
        .insert("focus_available".into(), false.into());
    let disabled: ShieldArcConfig = value.try_into().unwrap();
    assert!(!disabled.to_runtime().focus_available);
    let roundtrip: ShieldArcConfig = toml::from_str(&toml::to_string(&disabled).unwrap()).unwrap();
    assert_eq!(roundtrip, disabled);
}
