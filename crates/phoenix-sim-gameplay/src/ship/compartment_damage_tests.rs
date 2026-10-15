use super::*;
use phoenix_model::messages::SystemId;
use serde_json::json;

fn fixture() -> (InteriorConfig, DamageConfig, ShipConfig) {
    let interior: InteriorConfig=serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1.0}],
        "compartment":[
            {"id":"front","deck":"main","label":"Front","x":0.0,"z":0.0,"width":2.0,"depth":2.0,"material":"steel","max_hp":100.0,"recovery_fraction":0.25,"structural_remedy":"structure"},
            {"id":"rear","deck":"main","label":"Rear","x":2.0,"z":0.0,"width":2.0,"depth":2.0,"material":"steel","max_hp":100.0,"recovery_fraction":0.25,"structural_remedy":"structure"}
        ],
        "placements":{"engine":"front","scanner":"rear"}
    })).unwrap();
    let config:DamageConfig=serde_json::from_value(json!({
        "materials":{"steel":{"resistance":{"physical":1.0,"laser":2.0},"residual_fraction":0.5}},
        "remedies":{
            "structure":{"work_secs":1.0,"manual_work_fraction":0.25,"structural_hp":10.0,"outcome":{"kind":"structural"},"requirements":[{"resource":"plate","service":"parts","amount":1}]},
            "full":{"work_secs":2.0,"manual_work_fraction":0.25,"outcome":{"kind":"full"},"requirements":[{"resource":"module","service":"parts","amount":1}]},
            "patch":{"work_secs":1.0,"manual_work_fraction":0.25,"outcome":{"kind":"patch","factor":0.25},"requirements":[]}
        }
    })).unwrap();
    let component = json!({"id":"operation","binding":"operation","susceptibility":{"physical":1.0,"laser":1.0},"fault_dose_threshold":4.0,"degraded_factor":0.5,"full_remedy":"full","patch_remedy":"patch","field_replaceable":true});
    let ship: ShipConfig = serde_json::from_value(json!({"system":[
        {"id":"engine","kind":"helm_thrust","damage_component":[component.clone()]},
        {"id":"scanner","kind":"sensors","damage_component":[component]}
    ]}))
    .unwrap();
    (interior, config, ship)
}
fn definition() -> DamageDefinition {
    let (i, c, s) = fixture();
    DamageDefinition::compile(&i, &c, &s).unwrap()
}
fn hit(payload: f64, penetration: f32) -> DamageHit {
    DamageHit {
        incoming: [1.0, 0.0],
        profile: DamageProfile {
            emp: None,
            damage_type: DamageType::Physical,
            penetration,
        },
        payload,
        aim: None,
        source: "beam:ship:fore".into(),
        sample: Some(HitSample {
            deck: "main".into(),
            profile_fraction: 0.5,
        }),
    }
}
fn rng() -> Pcg32 {
    Pcg32::seeded(417, 0)
}

#[test]
fn emp_uses_actual_deposits_without_faults_and_expires_without_a_remedy() {
    let (interior, mut config, ship) = fixture();
    config.emp = Some(phoenix_model::emp::EmpResistanceConfig {
        resistance_per_stack: 1.0,
        max_stacks: 4,
        decay_secs: 20.0,
        bindings: vec![DamageBinding::Operation],
    });
    let definition = DamageDefinition::compile(&interior, &config, &ship).unwrap();
    let mut state = DamageState::new(&definition);
    let mut incident = hit(1.0, 0.0);
    incident.profile.emp = Some(phoenix_model::emp::EmpPayload { duration_secs: 6.0 });
    let clock = |tick| phoenix_model::emp::EmpClock {
        tick,
        period_nanos: 1_000_000_000,
    };
    let before = state.clone();
    assert!(state.apply_hit(&definition, &incident, &mut rng()).is_err());
    assert_eq!(state, before);
    let result = state
        .apply_hit_at(&definition, &incident, &mut rng(), Some(clock(0)))
        .unwrap();
    assert_eq!(
        result
            .deposits
            .iter()
            .map(|row| row.compartment.as_str())
            .collect::<Vec<_>>(),
        vec!["front"]
    );
    let engine = SystemId("engine".into());
    let scanner = SystemId("scanner".into());
    assert_eq!(
        state.effect_factor_at(
            &definition,
            &engine,
            DamageBinding::Operation,
            Some(clock(0))
        ),
        0.0
    );
    assert_eq!(
        state.effect_factor_at(
            &definition,
            &scanner,
            DamageBinding::Operation,
            Some(clock(0))
        ),
        1.0
    );
    assert!(state.components[&scanner]["operation"].emp.is_none());
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Healthy);
    let emp = state.components[&engine]["operation"].emp.clone();
    assert!(!state
        .remedy_component(&definition, &target("engine"), "full")
        .unwrap());
    assert_eq!(state.components[&engine]["operation"].emp, emp);
    state
        .apply_hit_at(&definition, &incident, &mut rng(), Some(clock(1)))
        .unwrap();
    let effect = state.components[&engine]["operation"].emp.as_ref().unwrap();
    assert_eq!(effect.effect_until_tick, 6);
    assert_eq!(effect.resistance_stacks, 2);
    assert_eq!(
        state.effect_factor_at(
            &definition,
            &engine,
            DamageBinding::Operation,
            Some(clock(6))
        ),
        1.0
    );
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Healthy);
    let before = state.clone();
    assert!(state
        .apply_hit_at(
            &definition,
            &incident,
            &mut rng(),
            Some(clock(u64::MAX - 1))
        )
        .is_err());
    assert_eq!(
        state, before,
        "overflow must refuse before structure/dose/EMP writes"
    );
}

#[test]
fn emp_json_continuation_keeps_absolute_effects_resistance_and_physical_damage_separate() {
    let (interior, mut config, ship) = fixture();
    config.emp = Some(phoenix_model::emp::EmpResistanceConfig {
        resistance_per_stack: 0.5,
        max_stacks: 8,
        decay_secs: 20.0,
        bindings: vec![DamageBinding::Operation],
    });
    let definition = DamageDefinition::compile(&interior, &config, &ship).unwrap();
    let mut state = DamageState::new(&definition);
    let mut incident = hit(0.1, 0.0);
    incident.profile.emp = Some(phoenix_model::emp::EmpPayload { duration_secs: 3.0 });
    let clock = |tick| phoenix_model::emp::EmpClock {
        tick,
        period_nanos: 100_000_000,
    };
    state
        .apply_hit_at(&definition, &incident, &mut rng(), Some(clock(10)))
        .unwrap();
    let mut resumed: DamageState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    for tick in 11..371 {
        state.validate_emp_at(&definition, clock(tick)).unwrap();
        resumed.validate_emp_at(&definition, clock(tick)).unwrap();
        if tick % 31 == 0 {
            for next in [&mut state, &mut resumed] {
                next.apply_hit_at(&definition, &incident, &mut rng(), Some(clock(tick)))
                    .unwrap();
            }
        }
        assert_eq!(state, resumed);
        assert_eq!(
            state.effect_factor_at(
                &definition,
                &SystemId("engine".into()),
                DamageBinding::Operation,
                Some(clock(tick))
            ),
            resumed.effect_factor_at(
                &definition,
                &SystemId("engine".into()),
                DamageBinding::Operation,
                Some(clock(tick))
            )
        );
    }
}
fn target(system: &str) -> ComponentRef {
    ComponentRef {
        system: SystemId(system.into()),
        component: "operation".into(),
    }
}
fn condition<'a>(state: &'a DamageState, system: &str) -> &'a ComponentCondition {
    &state.components[&SystemId(system.into())]["operation"].condition
}
fn approx(left: f64, right: f64) {
    assert!(
        (left - right).abs() <= f64::EPSILON * left.abs().max(right.abs()).max(1.0) * 8.0,
        "{left} != {right}"
    );
}

#[test]
fn strict_model_rejects_unknown_types_fields_and_unbound_remedy_shape() {
    assert!(
        serde_json::from_value::<DamageProfile>(json!({"damage_type":"emp","penetration":0}))
            .is_err()
    );
    assert!(serde_json::from_value::<DamageProfile>(
        json!({"damage_type":"laser","penetration":0,"hp":1})
    )
    .is_err());
    assert!(serde_json::from_value::<RemedyOutcome>(
        json!({"kind":"patch","factor":0.5,"timer":2})
    )
    .is_err());
    assert!(DamageProfile {
        emp: None,
        damage_type: DamageType::Physical,
        penetration: f32::NAN
    }
    .validate()
    .is_err());
    assert_eq!(
        DamageProfile::default(),
        DamageProfile {
            emp: None,
            damage_type: DamageType::Physical,
            penetration: 0.0
        }
    );
}
#[test]
fn compile_rejects_overlap_missing_material_and_invalid_structural_recipes() {
    let (i, c, s) = fixture();
    let mut bad = i.clone();
    bad.compartments[1].x = 1.0;
    assert!(DamageDefinition::compile(&bad, &c, &s)
        .unwrap_err()
        .contains("overlap"));
    let mut bad = i.clone();
    bad.compartments[0].material = "absent".into();
    assert!(DamageDefinition::compile(&bad, &c, &s).is_err());
    let mut bad = i.clone();
    bad.compartments[0].structural_remedy = "full".into();
    assert!(DamageDefinition::compile(&bad, &c, &s).is_err());
    let mut bad = i.clone();
    bad.compartments[0].recovery_fraction = 1.0;
    assert!(DamageDefinition::compile(&bad, &c, &s).is_err());
    let mut bad = c.clone();
    bad.materials
        .get_mut("steel")
        .unwrap()
        .resistance
        .remove(&DamageType::Laser);
    assert!(DamageDefinition::compile(&i, &bad, &s).is_err());
}
#[test]
fn compile_validates_exact_placements_components_and_bounded_recipes() {
    let (i, c, s) = fixture();
    let mut bad = i.clone();
    bad.placements.remove(&SystemId("engine".into()));
    assert!(DamageDefinition::compile(&bad, &c, &s).is_err());
    let mut bad = s.clone();
    let duplicate = bad.systems[0].damage_components[0].clone();
    bad.systems[0].damage_components.push(duplicate);
    assert!(DamageDefinition::compile(&i, &c, &bad).is_err());
    let mut bad = s.clone();
    bad.systems[0].damage_components[0].fault_dose_threshold = 0.0;
    assert!(DamageDefinition::compile(&i, &c, &bad).is_err());
    let mut bad = c.clone();
    bad.remedies.get_mut("patch").unwrap().outcome = RemedyOutcome::Patch { factor: 1.0 };
    assert!(DamageDefinition::compile(&i, &bad, &s).is_err());
    let mut bad = c.clone();
    bad.remedies.get_mut("full").unwrap().requirements[0].amount = -1;
    assert!(DamageDefinition::compile(&i, &bad, &s).is_err());
}
#[test]
fn zero_penetration_hits_the_first_room_from_the_actual_planar_direction() {
    let d = definition();
    let mut state = DamageState::new(&d);
    let result = state.apply_hit(&d, &hit(10.0, 0.0), &mut rng()).unwrap();
    assert_eq!(result.deposits.len(), 1);
    assert_eq!(result.deposits[0].compartment, "front");
    assert_eq!(state.compartments["rear"].hp, 100.0);
    assert_eq!(result.escaped_payload, 0.0);
    let mut reverse = hit(10.0, 0.0);
    reverse.incoming = [-1.0, 0.0];
    let mut state = DamageState::new(&d);
    let result = state.apply_hit(&d, &reverse, &mut rng()).unwrap();
    assert_eq!(result.deposits[0].compartment, "rear");
}
#[test]
fn penetration_spends_length_resistance_and_conserves_one_payload() {
    let d = definition();
    let mut state = DamageState::new(&d);
    let result = state.apply_hit(&d, &hit(40.0, 8.0), &mut rng()).unwrap();
    assert_eq!(
        result
            .deposits
            .iter()
            .map(|r| r.payload)
            .collect::<Vec<_>>(),
        vec![10.0, 10.0]
    );
    assert_eq!(result.structural_loss, 20.0);
    assert_eq!(result.escaped_payload, 20.0);
    assert_eq!(result.remaining_penetration, 4.0);
    assert_eq!(
        result.deposits.iter().map(|r| r.payload).sum::<f64>() + result.escaped_payload,
        40.0
    );
    let mut laser = hit(40.0, 8.0);
    laser.profile.damage_type = DamageType::Laser;
    let result = DamageState::new(&d)
        .apply_hit(&d, &laser, &mut rng())
        .unwrap();
    assert_eq!(result.escaped_payload, 0.0);
    assert_eq!(result.remaining_penetration, 0.0);
}
#[test]
fn distance_and_weakened_material_change_the_actual_traversal() {
    let d = definition();
    let mut straight = DamageState::new(&d);
    let first = straight.apply_hit(&d, &hit(20.0, 2.0), &mut rng()).unwrap();
    assert_eq!(first.deposits.len(), 1);
    let second = straight.apply_hit(&d, &hit(20.0, 2.0), &mut rng()).unwrap();
    assert_eq!(
        second.deposits.len(),
        2,
        "weakened front material allows breakthrough"
    );
    assert!(second.deposits[1].payload > 0.0);
    let room = d.compartment("front").unwrap();
    let a = intersect(room, [-1.0, 1.0], [1.0, 0.0]).unwrap();
    let r = libm::sqrt(0.5);
    let b = intersect(room, [0.0, 0.0], [r, r]).unwrap();
    assert!(b.1 - b.0 > a.1 - a.0);
}
#[test]
fn profile_sampling_uses_interval_union_and_never_samples_the_gaps() {
    let (mut i, c, s) = fixture();
    i.compartments[1].z = 4.0;
    let d = DamageDefinition::compile(&i, &c, &s).unwrap();
    let mut h = hit(1.0, 0.0);
    h.sample.as_mut().unwrap().profile_fraction = 0.75;
    let result = DamageState::new(&d).apply_hit(&d, &h, &mut rng()).unwrap();
    assert_eq!(result.deposits[0].compartment, "rear");
    let rooms: Vec<_> = definition().rooms.into_values().collect();
    assert_eq!(
        profile_offset(&rooms.iter().collect::<Vec<_>>(), [0.0, 1.0], 0.75),
        1.5,
        "overlapping projections are counted once"
    );
}
#[test]
fn shared_compartment_edges_do_not_deposit_twice_for_parallel_rays() {
    let (mut i, c, s) = fixture();
    i.compartments[1].x = 0.0;
    i.compartments[1].z = 2.0;
    let d = DamageDefinition::compile(&i, &c, &s).unwrap();
    let result = DamageState::new(&d)
        .apply_hit(&d, &hit(8.0, 8.0), &mut rng())
        .unwrap();
    assert_eq!(result.deposits.len(), 1);
    assert_eq!(result.deposits[0].compartment, "rear");
}
#[test]
fn weighted_decks_are_stable_by_identity_and_independent_of_aim() {
    let (mut i, c, s) = fixture();
    i.decks.push(phoenix_model::interior::DeckConfig {
        id: "other".into(),
        label: "Other".into(),
        damage_weight: 3.0,
    });
    let mut other = i.compartments[1].clone();
    other.id = "upper".into();
    other.deck = "other".into();
    other.z = 20.0;
    i.compartments.push(other);
    let d = DamageDefinition::compile(&i, &c, &s).unwrap();
    let mut reversed = i.clone();
    reversed.decks.reverse();
    let reversed = DamageDefinition::compile(&reversed, &c, &s).unwrap();
    let mut a = rng();
    let mut b = rng();
    let mut count = 0;
    for _ in 0..2000 {
        let sample = d.sample_hit(&mut a);
        assert_eq!(sample, reversed.sample_hit(&mut b));
        count += usize::from(sample.deck == "other");
    }
    assert!((1400..1600).contains(&count));
    let mut h = hit(5.0, 0.0);
    h.aim = Some("front".into());
    h.sample.as_mut().unwrap().deck = "other".into();
    let result = DamageState::new(&d).apply_hit(&d, &h, &mut rng()).unwrap();
    assert!(result.deposits.is_empty());
    assert_eq!(
        result.escaped_payload, 5.0,
        "known aim never guarantees another deck"
    );
}
#[test]
fn retained_bank_sample_uses_actual_new_direction_without_another_rng_draw() {
    let d = definition();
    let mut a = rng();
    let mut b = a.clone();
    let mut h = hit(1.0, 0.0);
    let mut state = DamageState::new(&d);
    state.apply_hit(&d, &h, &mut a).unwrap();
    h.incoming = [-1.0, 0.0];
    let result = state.apply_hit(&d, &h, &mut a).unwrap();
    assert_eq!(result.deposits[0].compartment, "rear");
    assert_eq!(a.next_u32(), b.next_u32());
}
#[test]
fn amount_based_dose_is_insensitive_to_continuous_tick_subdivision() {
    let d = definition();
    let mut whole = DamageState::new(&d);
    let mut split = whole.clone();
    whole.apply_hit(&d, &hit(8.0, 0.0), &mut rng()).unwrap();
    for _ in 0..32 {
        split.apply_hit(&d, &hit(0.25, 0.0), &mut rng()).unwrap();
    }
    assert_eq!(whole, split);
    assert_eq!(condition(&whole, "engine"), &ComponentCondition::Destroyed);
}
#[test]
fn susceptibility_divides_one_dose_among_components_instead_of_copying_it() {
    let (i, c, mut s) = fixture();
    let mut second = s.systems[0].damage_components[0].clone();
    second.id = "intake".into();
    second.binding = DamageBinding::Intake;
    s.systems[0].damage_components.push(second);
    let d = DamageDefinition::compile(&i, &c, &s).unwrap();
    let mut state = DamageState::new(&d);
    state.apply_hit(&d, &hit(4.0, 0.0), &mut rng()).unwrap();
    let components = &state.components[&SystemId("engine".into())];
    assert!(components
        .values()
        .all(|c| c.condition == ComponentCondition::Healthy && c.dose == 2.0));
    let mut whole = DamageState::new(&d);
    let mut split = whole.clone();
    whole.apply_hit(&d, &hit(16.0, 0.0), &mut rng()).unwrap();
    for _ in 0..64 {
        split.apply_hit(&d, &hit(0.25, 0.0), &mut rng()).unwrap();
    }
    assert_eq!(whole, split);
}
#[test]
fn structural_latch_clears_only_at_recovery_and_does_not_repair_components() {
    let d = definition();
    let mut state = DamageState::new(&d);
    state.apply_hit(&d, &hit(100.0, 0.0), &mut rng()).unwrap();
    assert!(state.compartments["front"].disabled);
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Destroyed);
    assert_eq!(state.restore_structure(&d, "front", 24.0).unwrap(), 24.0);
    assert!(state.compartments["front"].disabled);
    state.restore_structure(&d, "front", 1.0).unwrap();
    assert!(!state.compartments["front"].disabled);
    assert_eq!(
        state.effect_factor(&d, &SystemId("engine".into()), DamageBinding::Operation),
        0.0
    );
    state
        .remedy_component(&d, &target("engine"), "full")
        .unwrap();
    assert_eq!(
        state.effect_factor(&d, &SystemId("engine".into()), DamageBinding::Operation),
        1.0
    );
}
#[test]
fn zero_hp_rooms_still_receive_residual_material_deposition_and_component_faults() {
    let d = definition();
    let mut state = DamageState::new(&d);
    state.compartments.get_mut("front").unwrap().hp = 0.0;
    state.compartments.get_mut("front").unwrap().disabled = true;
    let result = state.apply_hit(&d, &hit(8.0, 2.0), &mut rng()).unwrap();
    assert_eq!(result.deposits[0].payload, 4.0);
    assert_eq!(result.deposits[0].structural_loss, 0.0);
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Damaged);
}
#[test]
fn remedies_clear_only_the_selected_cause_and_patch_is_stable_but_not_immune() {
    let d = definition();
    let mut state = DamageState::new(&d);
    state.apply_hit(&d, &hit(8.0, 0.0), &mut rng()).unwrap();
    assert!(state
        .remedy_component(&d, &target("engine"), "patch")
        .unwrap());
    assert_eq!(
        state.effect_factor(&d, &SystemId("engine".into()), DamageBinding::Operation),
        0.25
    );
    let unchanged = state.clone();
    assert!(!state
        .remedy_component(&d, &target("engine"), "patch")
        .unwrap());
    assert_eq!(state, unchanged);
    state.apply_hit(&d, &hit(4.0, 0.0), &mut rng()).unwrap();
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Destroyed);
    state
        .remedy_component(&d, &target("engine"), "full")
        .unwrap();
    assert_eq!(condition(&state, "engine"), &ComponentCondition::Healthy);
    let before = state.clone();
    assert!(state
        .remedy_component(&d, &target("engine"), "structure")
        .is_err());
    assert_eq!(before, state);
}
#[test]
fn irreparable_catastrophic_equipment_is_not_a_field_replacement_target() {
    let (i, c, mut s) = fixture();
    s.systems[0].damage_components[0].field_replaceable = false;
    let d = DamageDefinition::compile(&i, &c, &s).unwrap();
    let mut state = DamageState::new(&d);
    state.apply_hit(&d, &hit(8.0, 0.0), &mut rng()).unwrap();
    assert!(state
        .remedy_component(&d, &target("engine"), "full")
        .is_err());
    assert!(state
        .remedy_component(&d, &target("engine"), "patch")
        .is_err());
}
#[test]
fn all_compartments_zero_is_recoverable_and_not_automatic_ship_destruction() {
    let d = definition();
    let mut state = DamageState::new(&d);
    state.apply_hit(&d, &hit(1000.0, 0.0), &mut rng()).unwrap();
    let mut reverse = hit(1000.0, 0.0);
    reverse.incoming = [-1.0, 0.0];
    state.apply_hit(&d, &reverse, &mut rng()).unwrap();
    assert_eq!(state.structural_fraction(&d), 0.0);
    assert!(!state.catastrophic);
    state.validate(&d).unwrap();
}
#[test]
fn restore_is_preflighted_and_preserves_json_fractional_continuation() {
    let d = definition();
    let mut live = DamageState::new(&d);
    live.apply_hit(&d, &hit(0.9667950463481249, 0.0), &mut rng())
        .unwrap();
    let saved: DamageState = serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
    assert_eq!(live, saved);
    let mut restored = DamageState::new(&d);
    restored.restore(&d, &saved).unwrap();
    for _ in 0..16 {
        live.apply_hit(&d, &hit(0.25, 0.0), &mut rng()).unwrap();
        restored.apply_hit(&d, &hit(0.25, 0.0), &mut rng()).unwrap();
        assert_eq!(live, restored);
    }
    let mut invalid = saved.clone();
    invalid.compartments.remove("rear");
    let before = restored.clone();
    assert!(restored.restore(&d, &invalid).is_err());
    assert_eq!(restored, before);
    let mut invalid = saved;
    invalid
        .components
        .get_mut(&SystemId("engine".into()))
        .unwrap()
        .get_mut("operation")
        .unwrap()
        .dose = f64::NAN;
    assert!(restored.restore(&d, &invalid).is_err());
    assert_eq!(restored, before);
}
#[test]
fn invalid_hit_or_retained_sample_does_not_mutate_state_or_advance_rng() {
    let d = definition();
    for mut h in [hit(-1.0, 1.0), hit(1.0, f32::NAN), hit(f64::INFINITY, 1.0)] {
        h.sample = None;
        let mut a = rng();
        let mut b = a.clone();
        let mut state = DamageState::new(&d);
        let before = state.clone();
        assert!(state.apply_hit(&d, &h, &mut a).is_err());
        assert_eq!(state, before);
        assert_eq!(a.next_u32(), b.next_u32());
    }
    let mut h = hit(1.0, 1.0);
    h.sample.as_mut().unwrap().deck = "missing".into();
    assert!(DamageState::new(&d).apply_hit(&d, &h, &mut rng()).is_err());
    h.sample.as_mut().unwrap().deck = "main".into();
    h.sample.as_mut().unwrap().profile_fraction = 1.0;
    assert!(DamageState::new(&d).apply_hit(&d, &h, &mut rng()).is_err());
    h.sample = None;
    h.incoming = [0.0, 0.0];
    assert!(DamageState::new(&d).apply_hit(&d, &h, &mut rng()).is_err());
}
#[test]
fn oblique_penetration_and_escape_budget_remain_conservative() {
    let d = definition();
    for direction in [[1.0, 0.0], [1.0, 1.0], [-1.0, 0.0], [0.0, 1.0]] {
        for penetration in [0.0, 0.125, 1.0, 3.0, 100.0] {
            let mut h = hit(37.25, penetration);
            h.incoming = direction;
            let result = DamageState::new(&d).apply_hit(&d, &h, &mut rng()).unwrap();
            approx(
                result.deposits.iter().map(|d| d.payload).sum::<f64>() + result.escaped_payload,
                h.payload,
            );
            assert!(
                result.remaining_penetration >= 0.0
                    && result.remaining_penetration <= penetration as f64
            );
            assert!(result.structural_loss <= h.payload);
        }
    }
}

#[path = "compartment_damage_internal_tests.rs"]
mod internal_tests;
