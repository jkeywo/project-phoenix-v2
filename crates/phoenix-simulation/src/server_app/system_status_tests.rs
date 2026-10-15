use super::*;

fn cruiser() -> (
    crate::ship::config::ShipConfig,
    crate::ship::power::ShipPowerSystem,
) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let config = authored.ship_config.unwrap();
    let power = crate::modifiers::power_system::PowerSystem::from_energy(
        authored.energy.unwrap(),
        config.clone(),
    )
    .unwrap();
    (config, crate::ship::power::ShipPowerSystem(power))
}

#[test]
fn parent_intake_cut_keeps_charged_equipment_degraded_until_thermal_trip() {
    let (config, mut power) = cruiser();
    let id = SystemId("dock".into());
    let group = config.system(&id).unwrap().power_group.as_ref().unwrap();
    let parent = power
        .0
        .config()
        .groups
        .iter()
        .find(|entry| &entry.id == group)
        .unwrap()
        .parent
        .clone()
        .unwrap();
    power.0.network_mut().groups.get_mut(&parent).unwrap().cut = true;
    assert!(!power.0.network().groups[group].cut);
    let sources = ControlSourceResolver::new();
    let status = &project(&config, None, None, &sources, None, Some(&power), None)[&id];
    assert_eq!(status.availability, Availability::Degraded);
    assert_eq!(
        status
            .reasons
            .iter()
            .map(|reason| reason.code)
            .collect::<Vec<_>>(),
        vec![StatusReasonCode::IntakeCut]
    );
    let trip_heat = power
        .0
        .config()
        .consumers
        .iter()
        .find(|profile| profile.system == id)
        .unwrap()
        .reserve
        .trip_heat;
    let reserve = power.0.network_mut().consumers.get_mut(&id).unwrap();
    reserve.thermal_stopped = true;
    reserve.heat = trip_heat;
    let status = &project(&config, None, None, &sources, None, Some(&power), None)[&id];
    assert_eq!(status.availability, Availability::Unavailable);
    assert!(status
        .reasons
        .iter()
        .any(|reason| reason.code == StatusReasonCode::Overheated));
    assert!(status
        .reasons
        .iter()
        .any(|reason| reason.code == StatusReasonCode::IntakeCut));
    assert!(!status
        .reasons
        .iter()
        .any(|reason| reason.code == StatusReasonCode::Unpowered));
}

#[test]
fn work_costs_distinguish_free_discrete_and_partial_mixed_capabilities() {
    let (config, base) = cruiser();
    let id = SystemId("dock".into());
    let sources = ControlSourceResolver::new();
    for (continuous, discrete, energy, expected) in [
        (0.0, 0.0, 0.0, Availability::Normal),
        (0.0, 2.0, 1.0, Availability::Unavailable),
        (1.0, 2.0, 1.0, Availability::Degraded),
    ] {
        let mut hardware = base.0.config().clone();
        let profile = hardware
            .consumers
            .iter_mut()
            .find(|profile| profile.system == id)
            .unwrap();
        profile.continuous_cost_per_sec = continuous;
        profile.discrete_cost = discrete;
        profile.initial_energy = energy;
        let power = crate::ship::power::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_energy(hardware, config.clone())
                .unwrap(),
        );
        let status = &project(&config, None, None, &sources, None, Some(&power), None)[&id];
        assert_eq!(status.availability, expected);
        assert_eq!(
            status
                .reasons
                .iter()
                .any(|reason| reason.code == StatusReasonCode::Unpowered),
            expected != Availability::Normal
        );
    }
}
#[test]
fn status_visibility_follows_exact_console_host_and_revokes_stale_host() {
    use crate::core::messages::StationId;
    use phoenix_model::{console::ConsoleHostSnapshot, identity::ConsoleId};
    let id = SystemId("passive".into());
    let console = ConsoleId("navigation".into());
    let owners = BTreeMap::from([(id.clone(), console.clone())]);
    let mut hosts = vec![ConsoleHostSnapshot {
        console,
        host: Some(StationId("tactical".into())),
        rating: "Std".into(),
    }];
    assert!(visible_to(
        &id,
        &owners,
        &hosts,
        Some(&StationId("tactical".into()))
    ));
    assert!(!visible_to(
        &id,
        &owners,
        &hosts,
        Some(&StationId("navigation".into()))
    ));
    assert!(!visible_to(&id, &owners, &hosts, None));
    hosts[0].host = Some(StationId("captain".into()));
    assert!(!visible_to(
        &id,
        &owners,
        &hosts,
        Some(&StationId("tactical".into()))
    ));
    assert!(visible_to(
        &id,
        &owners,
        &hosts,
        Some(&StationId("captain".into()))
    ));
}
#[test]
fn dock_current_facts_override_historical_refusal_and_keep_multiple_reasons() {
    let (config, mut power) = cruiser();
    let id = SystemId("dock".into());
    let mut dock = crate::dock::server::DockControl::from_system(config.system(&id).unwrap());
    let mut sources = ControlSourceResolver::new();
    sources.set(id.clone(), ControlSource::Ai);
    let mut hull = crate::ship::damage::SystemHull::from_config(&[(id.clone(), 100.0)]);
    hull.set_hp(&id, 0.0);
    let statuses = project(
        &config,
        Some(&hull),
        None,
        &sources,
        Some(&dock),
        None,
        None,
    );
    let status = &statuses[&id];
    assert_eq!(status.availability, Availability::Unavailable);
    assert!(status.delegated);
    assert_eq!(
        status
            .reasons
            .iter()
            .map(|reason| reason.code)
            .collect::<Vec<_>>(),
        vec![
            StatusReasonCode::Destroyed,
            StatusReasonCode::Unpowered,
            StatusReasonCode::NoDockMarkers
        ]
    );
    assert_eq!(status.reasons[1].component_id.as_deref(), Some("power"));
    assert_eq!(status.reasons[2].component_id.as_deref(), Some("mating"));
    hull.set_hp(&id, 100.0);
    dock.last_refusal = Some(crate::dock::mating::DockRefusal::Unpowered);
    let markers = crate::dock::server::DockMarkers {
        markers: vec![crate::dock::mating::DockMarker {
            position: bevy::prelude::Vec3::ZERO,
            direction: bevy::prelude::Vec3::NEG_Z,
        }],
    };
    let status = &project(
        &config,
        Some(&hull),
        None,
        &sources,
        Some(&dock),
        Some(&power),
        Some(&markers),
    )[&id];
    assert_eq!(status.availability, Availability::Normal);
    assert!(status.reasons.is_empty());
    power.0.network_mut().consumers.get_mut(&id).unwrap().energy = 0.0;
    let status = &project(
        &config,
        Some(&hull),
        None,
        &sources,
        Some(&dock),
        Some(&power),
        Some(&markers),
    )[&id];
    assert_eq!(status.availability, Availability::Unavailable);
    assert_eq!(status.reasons[0].code, StatusReasonCode::Unpowered);
    assert!(status.delegated);
}

#[test]
fn physical_intake_fault_preserves_charged_work_but_operation_loss_does_not() {
    use phoenix_model::damage::{ComponentCondition, DamageBinding};
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut physical = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .expect("physical Cruiser");
    let (config, power) = cruiser();
    let id = SystemId("dock".into());
    let intake = physical.definition.components[&id]
        .iter()
        .find(|(_, row)| row.binding == DamageBinding::Intake)
        .unwrap()
        .0
        .clone();
    let operation = physical.definition.components[&id]
        .iter()
        .find(|(_, row)| row.binding == DamageBinding::Operation)
        .unwrap()
        .0
        .clone();
    {
        let state = physical
            .state
            .components
            .get_mut(&id)
            .unwrap()
            .get_mut(&intake)
            .unwrap();
        state.condition = ComponentCondition::Destroyed;
        state.last_source = Some("test-hit".into());
    }
    let sources = ControlSourceResolver::new();
    assert!(!crate::ship::compartment_damage::operation_disabled(
        Some(&physical),
        None,
        &id
    ));
    let status = &project(
        &config,
        None,
        Some(&physical),
        &sources,
        None,
        Some(&power),
        None,
    )[&id];
    assert_eq!(status.availability, Availability::Degraded);
    {
        let state = physical
            .state
            .components
            .get_mut(&id)
            .unwrap()
            .get_mut(&operation)
            .unwrap();
        state.condition = ComponentCondition::Destroyed;
        state.last_source = Some("test-hit".into());
    }
    assert!(crate::ship::compartment_damage::operation_disabled(
        Some(&physical),
        None,
        &id
    ));
    let status = &project(
        &config,
        None,
        Some(&physical),
        &sources,
        None,
        Some(&power),
        None,
    )[&id];
    assert_eq!(status.availability, Availability::Unavailable);
    assert!(
        power.0.network().consumers[&id].energy > 0.0,
        "tool loss does not erase its battery"
    );
}

#[test]
fn emp_shared_status_expires_without_repair_or_physical_damage_reclassification() {
    use phoenix_model::{
        damage::{ComponentCondition, DamageBinding},
        emp::{ComponentEmpState, EmpClock, EmpPayload},
    };
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut physical = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .unwrap();
    let (config, power) = cruiser();
    let id = SystemId("passive-receiver".into());
    let component = physical.definition.components[&id]
        .iter()
        .find(|(_, row)| row.binding == DamageBinding::Operation)
        .unwrap()
        .0
        .clone();
    let clock = EmpClock {
        tick: 100,
        period_nanos: 1_000_000_000,
    };
    let effect = ComponentEmpState::apply(
        None,
        physical.definition.config.emp.as_ref().unwrap(),
        EmpPayload { duration_secs: 3.0 },
        clock,
    )
    .unwrap();
    physical.emp_clock = Some(clock);
    physical
        .state
        .components
        .get_mut(&id)
        .unwrap()
        .get_mut(&component)
        .unwrap()
        .emp = Some(effect);
    let sources = ControlSourceResolver::new();
    let status = &project(
        &config,
        None,
        Some(&physical),
        &sources,
        None,
        Some(&power),
        None,
    )[&id];
    assert_eq!(status.availability, Availability::Unavailable);
    assert!(status
        .reasons
        .iter()
        .any(|reason| reason.code == StatusReasonCode::Emp
            && reason.component_id.as_ref() == Some(&component)));
    assert!(!status
        .reasons
        .iter()
        .any(|reason| reason.code == StatusReasonCode::Destroyed));
    let before = physical.state.clone();
    physical.emp_clock = Some(EmpClock { tick: 103, ..clock });
    let status = &project(
        &config,
        None,
        Some(&physical),
        &sources,
        None,
        Some(&power),
        None,
    )[&id];
    assert_eq!(status.availability, Availability::Normal);
    assert!(!status
        .reasons
        .iter()
        .any(|reason| reason.code == StatusReasonCode::Emp));
    assert_eq!(physical.state, before);
    assert_eq!(
        physical.state.components[&id][&component].condition,
        ComponentCondition::Healthy
    );
}
