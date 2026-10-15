use super::*;
use phoenix_model::damage::ComponentCondition;

fn fixture() -> (EntityConfig, CompiledFitting, FittingState) {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let profile = CompiledFitting::compile(&config).unwrap().unwrap();
    let state = FittingState::from_profile(&profile).unwrap();
    (config, profile, state)
}
fn healthy() -> ComponentState {
    ComponentState {
        emp: None,
        condition: ComponentCondition::Healthy,
        dose: 0.0,
        last_source: None,
    }
}

#[test]
fn emp_follows_the_actual_removed_assembly_and_expires_in_storage_without_cleansing() {
    let (config, profile, mut state) = fixture();
    let policy = config.damage.as_ref().unwrap().emp.as_ref().unwrap();
    let mut affected = healthy();
    affected.emp = Some(
        phoenix_model::emp::ComponentEmpState::apply(
            None,
            policy,
            phoenix_model::emp::EmpPayload { duration_secs: 6.0 },
            phoenix_model::emp::EmpClock {
                tick: 10,
                period_nanos: 1_000_000_000,
            },
        )
        .unwrap(),
    );
    state
        .stored_to_work(&profile, "fore-beam-spare", 8, "fore-beam")
        .unwrap();
    let old = state
        .remove_installed(&profile, "fore-beam", 8, affected.clone())
        .unwrap();
    let replacement = state
        .install_from_work(&profile, "fore-beam", "fore-beam-spare", 8)
        .unwrap();
    assert_eq!(
        replacement,
        healthy(),
        "healthy replacement cannot inherit another assembly's EMP"
    );
    state
        .return_to_locker(&profile, &old, 8, "weapons-equipment")
        .unwrap();
    let saved: FittingState = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(saved.assemblies[&old].condition, Some(affected.clone()));
    assert_eq!(affected.emp.as_ref().unwrap().factor_at(15), 0.0);
    assert_eq!(
        affected.emp.as_ref().unwrap().factor_at(16),
        1.0,
        "stowed effect expires without inventory ticking or repair"
    );
    state
        .stored_to_work(&profile, &old, 9, "fore-beam")
        .unwrap();
    state
        .remove_installed(&profile, "fore-beam", 9, replacement)
        .unwrap();
    assert_eq!(
        state
            .install_from_work(&profile, "fore-beam", &old, 9)
            .unwrap(),
        affected
    );
    state.validate(&profile).unwrap();
}

#[test]
fn actual_team_fetches_support_spare_to_weapons_without_duplicate_or_early_install() {
    use crate::jobs::repair::RepairTeams;
    use crate::ship::interior::{InteriorGraph, TeamLocation};
    let (config, profile, mut state) = fixture();
    let item = "fore-beam-spare";
    let condition = ComponentState {
        emp: None,
        condition: ComponentCondition::Damaged,
        dose: 0.5,
        last_source: Some("retained-salvage-damage".into()),
    };
    let spare = state.assemblies.get_mut(item).unwrap();
    spare.location = EquipmentLocation::Stored {
        locker: "support-equipment".into(),
    };
    spare.condition = Some(condition.clone());
    state.validate(&profile).unwrap();
    let mut teams = RepairTeams::new(config.interior.as_ref().unwrap().team_homes.len());
    teams.install_layout(
        InteriorGraph::compile(
            config.interior.as_ref().unwrap(),
            config.ship_config.as_ref().unwrap(),
            config.interior.as_ref().unwrap().team_homes.len(),
        )
        .unwrap(),
    );
    let id = teams
        .claim_refit(
            0,
            17,
            profile.slot("fore-beam").unwrap().system.clone(),
            "Fore beam".into(),
        )
        .unwrap();
    assert!(teams.visit_refit_compartment(id, "support"));
    let mut witnessed = false;
    for _ in 0..400 {
        teams.tick_refits(0.125, &BTreeSet::new());
        witnessed |= matches!(
            teams.refit_location(id),
            Some(TeamLocation::Crossing { .. })
        );
        if teams.refit_effect_enabled(id) {
            break;
        }
    }
    assert!(witnessed);
    assert!(teams.refit_effect_enabled(id));
    assert!(
        matches!(teams.refit_location(id), Some(TeamLocation::Compartment { compartment }) if compartment == "support")
    );
    let before = state.clone();
    assert!(state
        .stored_to_work(&profile, item, id.0, "fore-beam")
        .is_err());
    assert!(state
        .pick_up_from_locker(&profile, item, id.0, "weapons")
        .is_err());
    assert_eq!(state, before);
    state
        .pick_up_from_locker(&profile, item, id.0, "support")
        .unwrap();
    assert!(state
        .install_from_work(&profile, "fore-beam", item, id.0)
        .is_err());
    assert!(teams.visit_refit_compartment(id, "weapons"));
    teams.tick_refits(0.125, &BTreeSet::new());
    assert!(matches!(
        teams.refit_location(id),
        Some(TeamLocation::Crossing { .. })
    ));
    let route = teams.save_state();
    let items: FittingState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    let mut resumed = teams.clone();
    resumed
        .restore_state(&serde_json::from_str(&serde_json::to_string(&route).unwrap()).unwrap())
        .unwrap();
    state = items;
    for _ in 0..400 {
        teams.tick_refits(0.125, &BTreeSet::new());
        resumed.tick_refits(0.125, &BTreeSet::new());
        assert_eq!(teams.save_state(), resumed.save_state());
        if teams.refit_effect_enabled(id) {
            break;
        }
    }
    assert!(teams.refit_effect_enabled(id));
    assert!(
        matches!(teams.refit_location(id), Some(TeamLocation::Compartment { compartment }) if compartment == "weapons")
    );
    state.move_in_work(&profile, item, id.0, "weapons").unwrap();
    let count = state.assemblies.len();
    let old = state
        .remove_installed(&profile, "fore-beam", id.0, healthy())
        .unwrap();
    assert_eq!(
        state
            .install_from_work(&profile, "fore-beam", item, id.0)
            .unwrap(),
        condition
    );
    state
        .return_to_locker(&profile, &old, id.0, "weapons-equipment")
        .unwrap();
    assert_eq!(state.assemblies.len(), count);
    assert_eq!(state.installed("fore-beam").unwrap().id, item);
    assert!(state.installed("fore-beam").unwrap().condition.is_none());
    assert!(teams.finish_refit(id));
}

#[test]
fn portable_fetch_rejects_wrong_job_room_and_owner_before_writes() {
    let (_, profile, mut state) = fixture();
    let item = "fore-beam-spare";
    for (job, room) in [(0, "weapons"), (1, "support"), (1, "absent")] {
        let before = state.clone();
        assert!(state
            .pick_up_from_locker(&profile, item, job, room)
            .is_err());
        assert_eq!(state, before);
    }
    state
        .pick_up_from_locker(&profile, item, 3, "weapons")
        .unwrap();
    let before = state.clone();
    assert!(state
        .pick_up_from_locker(&profile, item, 3, "weapons")
        .is_err());
    assert!(state.move_in_work(&profile, item, 4, "support").is_err());
    assert!(state.move_in_work(&profile, item, 3, "absent").is_err());
    assert!(state
        .move_in_work(&profile, "reactor-installed", 3, "support")
        .is_err());
    assert_eq!(state, before);
    state.move_in_work(&profile, item, 3, "support").unwrap();
    assert!(
        matches!(&state.assemblies[item].location, EquipmentLocation::InWork { job: 3, compartment } if compartment == "support")
    );
    assert_eq!(
        state.assemblies[item].condition,
        before.assemblies[item].condition
    );
}
fn swap(
    profile: &CompiledFitting,
    state: &mut FittingState,
    slot: &str,
    spare: &str,
    job: u64,
) -> ComponentState {
    state.stored_to_work(profile, spare, job, slot).unwrap();
    let old = state
        .remove_installed(profile, slot, job, healthy())
        .unwrap();
    let condition = state.install_from_work(profile, slot, spare, job).unwrap();
    let locker = if slot == "fore-beam" {
        "weapons-equipment"
    } else {
        "engineering-equipment"
    };
    state.return_to_locker(profile, &old, job, locker).unwrap();
    condition
}

#[test]
fn actual_cruiser_defaults_are_equivalent_and_inheritor_has_no_mismatched_slots() {
    let (config, profile, state) = fixture();
    assert_eq!(profile.effective_config(&state).unwrap(), config);
    assert!(profile.unavailable_systems(&state).unwrap().is_empty());
    let child =
        crate::test_fixtures::load_entity_config("assets/entities/dynasty_player_cruiser.toml")
            .unwrap();
    assert!(CompiledFitting::compile(&child).unwrap().is_none());
    assert!(!child
        .weapons_console
        .as_ref()
        .unwrap()
        .phaser_banks
        .is_empty());
    let source = toml::to_string(&config.to_toml_value().unwrap()).unwrap();
    let reparsed = EntityConfig::from_toml(&source).unwrap();
    assert_eq!(reparsed.fitting, config.fitting);
}

#[test]
fn real_sidegrade_changes_only_supported_strategy_and_work_cost_not_the_fixed_graph() {
    let (config, profile, mut state) = fixture();
    state
        .request(&profile, "fore-beam", "cruiser-beam-pulse")
        .unwrap();
    assert_eq!(
        profile.effective_config(&state).unwrap(),
        config,
        "intent alone grants no hardware"
    );
    swap(&profile, &mut state, "fore-beam", "fore-beam-spare", 7);
    let effective = profile.effective_config(&state).unwrap();
    let old = &config.weapons_console.as_ref().unwrap().phaser_banks[0];
    let new = &effective.weapons_console.as_ref().unwrap().phaser_banks[0];
    assert_eq!(
        (
            new.beam_damage_per_sec,
            new.beam_duration_secs,
            new.cooldown_secs
        ),
        (40.0, 4.0, 8.0)
    );
    assert_eq!(
        (
            &new.id,
            new.facing_deg,
            new.fire_arc_deg,
            &new.marker,
            &new.ai
        ),
        (
            &old.id,
            old.facing_deg,
            old.fire_arc_deg,
            &old.marker,
            &old.ai
        )
    );
    assert_eq!(effective.ship_config, config.ship_config);
    assert_eq!(effective.interior, config.interior);
    assert_eq!(effective.damage, config.damage);
    let old_consumer = config
        .energy
        .as_ref()
        .unwrap()
        .consumers
        .iter()
        .find(|c| c.system.0 == "phaser-fore")
        .unwrap();
    let new_consumer = effective
        .energy
        .as_ref()
        .unwrap()
        .consumers
        .iter()
        .find(|c| c.system.0 == "phaser-fore")
        .unwrap();
    assert_eq!(new_consumer.continuous_cost_per_sec, 1.5);
    assert_eq!(
        (
            &new_consumer.reserve,
            new_consumer.initial_energy,
            new_consumer.initial_heat,
            &new_consumer.coolant
        ),
        (
            &old_consumer.reserve,
            old_consumer.initial_energy,
            old_consumer.initial_heat,
            &old_consumer.coolant
        )
    );
}

#[test]
fn condition_moves_once_and_healthy_replacement_does_not_inherit_removed_faults() {
    let (_, profile, mut state) = fixture();
    let damaged = ComponentState {
        emp: None,
        condition: ComponentCondition::Damaged,
        dose: 3.25,
        last_source: Some("actual-hit".into()),
    };
    state
        .stored_to_work(&profile, "fore-beam-spare", 8, "fore-beam")
        .unwrap();
    let old = state
        .remove_installed(&profile, "fore-beam", 8, damaged.clone())
        .unwrap();
    assert!(state.installed("fore-beam").is_none());
    assert_eq!(state.assemblies[&old].condition, Some(damaged.clone()));
    let saved = serde_json::to_string(&state).unwrap();
    let mut resumed: FittingState = serde_json::from_str(&saved).unwrap();
    resumed.validate(&profile).unwrap();
    assert_eq!(
        state
            .install_from_work(&profile, "fore-beam", "fore-beam-spare", 8)
            .unwrap(),
        healthy()
    );
    assert_eq!(
        resumed
            .install_from_work(&profile, "fore-beam", "fore-beam-spare", 8)
            .unwrap(),
        healthy()
    );
    assert_eq!(state, resumed);
    assert_eq!(state.assemblies["fore-beam-spare"].condition, None);
    state
        .return_to_locker(&profile, &old, 8, "weapons-equipment")
        .unwrap();
    state
        .stored_to_work(&profile, &old, 9, "fore-beam")
        .unwrap();
    state
        .remove_installed(&profile, "fore-beam", 9, healthy())
        .unwrap();
    assert_eq!(
        state
            .install_from_work(&profile, "fore-beam", &old, 9)
            .unwrap(),
        damaged,
        "reinstall cannot cleanse damage/dose/source"
    );
}

#[test]
fn authored_initial_equipment_cannot_invent_a_runtime_emp_clock() {
    let (mut config, _, _) = fixture();
    let policy = config.damage.as_ref().unwrap().emp.as_ref().unwrap();
    let effect = phoenix_model::emp::ComponentEmpState::apply(
        None,
        policy,
        phoenix_model::emp::EmpPayload { duration_secs: 6.0 },
        phoenix_model::emp::EmpClock {
            tick: 20,
            period_nanos: 1_000_000_000,
        },
    )
    .unwrap();
    config
        .fitting
        .as_mut()
        .unwrap()
        .assemblies
        .iter_mut()
        .find_map(|item| item.condition.as_mut())
        .unwrap()
        .emp = Some(effect);
    assert!(CompiledFitting::compile(&config)
        .unwrap_err()
        .contains("causal EMP"));
}

#[test]
fn vacancies_are_explicit_unavailable_and_reactor_refit_preserves_all_buffers_and_initials() {
    let (config, profile, mut state) = fixture();
    let old = state
        .remove_installed(&profile, "fore-beam", 10, healthy())
        .unwrap();
    let vacant = profile.effective_config(&state).unwrap();
    assert!(vacant
        .weapons_console
        .as_ref()
        .unwrap()
        .phaser_banks
        .iter()
        .all(|b| b.id != "fore"));
    assert_eq!(vacant.ship_config, config.ship_config);
    assert!(profile
        .unavailable_systems(&state)
        .unwrap()
        .contains(&SystemId("phaser-fore".into())));
    state
        .install_from_work(&profile, "fore-beam", &old, 10)
        .unwrap();
    swap(&profile, &mut state, "reactor", "reactor-spare", 11);
    let changed = profile.effective_config(&state).unwrap();
    let before = &config.energy.as_ref().unwrap().reactor;
    let after = &changed.energy.as_ref().unwrap().reactor;
    assert_eq!(after.sustainable_output_per_sec, 28.53);
    assert_eq!(after.ramp_per_sec, 11.8875);
    assert_eq!(
        (
            after.reserve.capacity,
            after.initial_energy,
            after.initial_heat,
            &after.coolant
        ),
        (
            before.reserve.capacity,
            before.initial_energy,
            before.initial_heat,
            &before.coolant
        )
    );
    state
        .remove_installed(&profile, "reactor", 12, healthy())
        .unwrap();
    assert!(profile
        .unavailable_systems(&state)
        .unwrap()
        .contains(&SystemId("power-reactor".into())));
    assert_eq!(
        profile
            .effective_config(&state)
            .unwrap()
            .energy
            .as_ref()
            .unwrap()
            .reactor,
        *before,
        "vacancy is an explicit runtime availability gate, not zeroed operator intent/config"
    );
}

#[test]
fn work_identity_space_and_local_access_refuse_transactionally_without_losing_items() {
    let (_, profile, mut state) = fixture();
    let before = state.clone();
    assert!(state
        .stored_to_work(&profile, "reactor-spare", 1, "fore-beam")
        .is_err());
    assert_eq!(state, before);
    state
        .stored_to_work(&profile, "fore-beam-spare", 4, "fore-beam")
        .unwrap();
    let committed = state.clone();
    assert!(state
        .stored_to_work(&profile, "fore-beam-spare", 5, "fore-beam")
        .is_err());
    assert!(state
        .return_to_locker(&profile, "fore-beam-spare", 5, "weapons-equipment")
        .is_err());
    assert!(state
        .return_to_locker(&profile, "fore-beam-spare", 4, "engineering-equipment")
        .is_err());
    assert_eq!(state, committed);
    state
        .return_to_locker(&profile, "fore-beam-spare", 4, "weapons-equipment")
        .unwrap();
    assert_eq!(
        state, before,
        "cancel before removal returns the same actual assembly without refund/mint"
    );
    let mut full = profile.clone();
    full.config
        .lockers
        .iter_mut()
        .find(|l| l.id == "weapons-equipment")
        .unwrap()
        .space = 1;
    let old = state
        .remove_installed(&full, "fore-beam", 6, healthy())
        .unwrap();
    let removed = state.clone();
    assert!(state
        .return_to_locker(&full, &old, 6, "weapons-equipment")
        .is_err());
    assert_eq!(
        state, removed,
        "full return leaves old hardware real and in work"
    );
    assert_eq!(state.assemblies.len(), 4);
}

#[test]
fn continuation_rejects_duplicate_installs_competing_condition_and_orphan_keys() {
    let (_, profile, state) = fixture();
    for mode in 0..10 {
        let mut bad = state.clone();
        match mode {
            0 => {
                bad.assemblies
                    .get_mut("fore-beam-installed")
                    .unwrap()
                    .condition = Some(healthy());
            }
            1 => {
                bad.assemblies.get_mut("fore-beam-spare").unwrap().condition = None;
            }
            2 => {
                bad.assemblies.get_mut("fore-beam-spare").unwrap().location =
                    EquipmentLocation::Installed {
                        slot: "fore-beam".into(),
                    };
                bad.assemblies.get_mut("fore-beam-spare").unwrap().condition = None;
            }
            3 => {
                bad.assemblies
                    .get_mut("fore-beam-spare")
                    .unwrap()
                    .condition
                    .as_mut()
                    .unwrap()
                    .dose = f64::NAN;
            }
            4 => {
                bad.desired
                    .insert("unknown".into(), "cruiser-beam-standard".into());
            }
            5 => {
                bad.assemblies.get_mut("fore-beam-spare").unwrap().id = "other-key".into();
            }
            6 => {
                bad.assemblies.get_mut("fore-beam-spare").unwrap().location =
                    EquipmentLocation::InWork {
                        job: 0,
                        compartment: "weapons".into(),
                    };
            }
            7 => {
                bad.assemblies
                    .get_mut("fore-beam-spare")
                    .unwrap()
                    .condition
                    .as_mut()
                    .unwrap()
                    .dose = 20.0;
            }
            8 => {
                bad.assemblies
                    .get_mut("fore-beam-spare")
                    .unwrap()
                    .condition
                    .as_mut()
                    .unwrap()
                    .dose = 1.0;
            }
            _ => {
                let condition = bad
                    .assemblies
                    .get_mut("fore-beam-spare")
                    .unwrap()
                    .condition
                    .as_mut()
                    .unwrap();
                condition.condition = ComponentCondition::Patched { factor: 0.123456 };
                condition.last_source = Some("real-source".into());
            }
        }
        assert!(bad.validate(&profile).is_err(), "tamper mode {mode}");
    }
    let mut globally_named = state;
    let item = globally_named.assemblies.remove("fore-beam-spare").unwrap();
    let id = "eq-11111111-1111-4111-8111-111111111111-fore-beam-spare".to_string();
    globally_named
        .assemblies
        .insert(id.clone(), EquipmentAssembly { id, ..item });
    globally_named.validate(&profile).unwrap();
}

#[test]
fn authored_closed_types_and_fixed_graph_refusals_are_not_runtime_migration_fallbacks() {
    let (config, _, _) = fixture();
    for mode in 0..11 {
        let mut bad = config.clone();
        let profile = bad.fitting.as_mut().unwrap();
        match mode {
            0 => profile.slots.push(profile.slots[0].clone()),
            1 => profile.slots[0].system = SystemId("tractor".into()),
            2 => profile.slots[0].operation_component = "intake".into(),
            3 => profile.equipment[0].work.manual_work_fraction = 0.0,
            4 => profile.equipment[0].work.requirements[0].service = "absent-service".into(),
            5 => profile.equipment[0].storage_space = 0,
            6 => {
                if let EquipmentHardware::Beam(h) = &mut profile.equipment[1].hardware {
                    h.auto_arc_deg = 300.0;
                }
            }
            7 => profile.equipment[2].work.class = RefitClass::Field,
            8 => profile.assemblies.retain(|a| a.id != "fore-beam-installed"),
            9 => profile.slots[0].default_equipment = "cruiser-beam-pulse".into(),
            _ => profile.assemblies.push(profile.assemblies[0].clone()),
        }
        assert!(
            CompiledFitting::compile(&bad).is_err(),
            "authored mode {mode}"
        );
    }
    let mut json = serde_json::to_value(config.fitting.unwrap()).unwrap();
    json["equipment"][0]["hardware"]["unrestricted_extension"] = true.into();
    assert!(serde_json::from_value::<FittingConfig>(json).is_err());
}

#[test]
fn facility_has_real_owned_powered_dock_and_explicit_port_permission() {
    let good =
        crate::test_fixtures::load_entity_config("assets/entities/station_axiom.toml").unwrap();
    validate_refit_facility(&good).unwrap();
    assert!(
        good.dock.is_none(),
        "active provider has one Dock tuning owner"
    );
    let system = good
        .ship_config
        .as_ref()
        .unwrap()
        .system(&SystemId("dock".into()))
        .unwrap();
    assert!(system.ai_only);
    assert!(
        system.config.as_ref().unwrap().get("ai").is_some(),
        "ordinary authored Dock tree, not a second service controller"
    );
    assert_eq!(
        good.ship_config.as_ref().unwrap().stations.len(),
        0,
        "no claimable seats added"
    );
    let energy = good.energy.as_ref().unwrap();
    assert_eq!(
        energy.reactor.sustainable_output_per_sec, 2.55,
        "explicit new .25 facility hardware budget"
    );
    assert_eq!(
        energy
            .groups
            .iter()
            .find(|g| g.id.0 == "weapons")
            .unwrap()
            .default_flow_per_sec,
        2.3
    );
    assert_eq!(
        energy
            .consumers
            .iter()
            .find(|c| c.system.0 == "phaser-defence")
            .unwrap()
            .continuous_cost_per_sec,
        1.0
    );
    assert!(energy
        .consumers
        .iter()
        .any(|c| c.system.0 == "dock" && c.continuous_cost_per_sec > 0.0));
    for mode in 0..5 {
        let mut bad = good.clone();
        match mode {
            0 => bad.refit_facility.as_mut().unwrap().system = SystemId("absent".into()),
            1 => bad.refit_facility.as_mut().unwrap().system = SystemId("phaser-defence".into()),
            2 => bad.port_service = None,
            3 => bad
                .energy
                .as_mut()
                .unwrap()
                .consumers
                .retain(|c| c.system.0 != "dock"),
            _ => bad
                .ship_config
                .as_mut()
                .unwrap()
                .consoles
                .retain(|c| c.id.0 != "ai-dock"),
        }
        assert!(
            validate_refit_facility(&bad).is_err(),
            "facility mode {mode}"
        );
    }
    let inherited =
        crate::test_fixtures::load_entity_config("assets/entities/manual_trade_port.toml").unwrap();
    validate_refit_facility(&inherited).unwrap();
}
