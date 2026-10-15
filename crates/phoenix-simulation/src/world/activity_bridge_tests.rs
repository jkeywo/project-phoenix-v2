use super::*;
use phoenix_sim_world::world::activities::*;

#[test]
fn declared_future_actor_arms_only_at_its_actual_consumer_and_dynamic_binding_refuses() {
    let base = declared_actor_fixture("future", "assets/entities/alliance_cruiser.toml");
    let definition = SituationDefinition {
        id: "supply".into(),
        initial: "received".into(),
        state: vec![SituationState {
            id: "received".into(),
            when: SituationCondition::Received,
            call: None,
            next: None,
        }],
    };
    let config = SituationConfig {
        id: "future-supply".into(),
        definition: definition.id.clone(),
        request: ActivityRequest::Delivery {
            actor: "future".into(),
            owner: SystemId("logistics".into()),
            source: "main".into(),
            destination: "local".into(),
            commodity: "parts".into(),
            amount: 1,
        },
    };
    let mut runtime = WorldContentRuntime::default();
    runtime.situation_definitions = vec![definition];
    runtime.situation_configs = vec![config];
    arm_for_actor(&mut runtime, &base, "other", None, 2, |_| true);
    assert!(runtime.situations.instances.is_empty());
    arm_for_actor(&mut runtime, &base, "future", None, 7, |_| true);
    assert_eq!(runtime.situations.instances["future-supply"].armed_tick, 7);
    let mut invalid = runtime.situation_configs[0].clone();
    invalid.id = "dynamic-supply".into();
    if let ActivityRequest::Delivery { actor, .. } = &mut invalid.request {
        *actor = "dynamic".into();
    }
    runtime.situation_configs.push(invalid);
    runtime
        .name_to_uuid
        .insert("dynamic".into(), "dynamic-uuid".into());
    let before = runtime.situations.clone();
    arm_for_actor(&mut runtime, &base, "dynamic-uuid", None, 8, |_| true);
    assert_eq!(
        runtime.situations, before,
        "a transient runtime name/recipe does not authorize a first-consumer binding"
    );
}
