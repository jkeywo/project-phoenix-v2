use super::*;
use crate::ai::server::AiEntityDisabled;

#[test]
fn real_message_bridge_distinguishes_recovery_disablement_and_destruction() {
    let mut app = ai_trigger_test_app();
    {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("actor".into(), "actor-uuid".into());
        runtime
            .entity_groups
            .insert("wave".into(), ["actor".into()].into());
        let state = |condition| TriggerState {
            trigger: crate::world::config::scripted_trigger(condition),
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            seen_disabled: Default::default(),
            last_fired_elapsed: None,
        };
        runtime.triggers.replace_declarative(vec![
            state(TriggerCondition::OnDisabled {
                entity_name: "actor".into(),
            }),
            state(TriggerCondition::OnAllNeutralized {
                group: "wave".into(),
                after_secs: 0.0,
            }),
            state(TriggerCondition::OnDestroyed {
                entity_name: "actor".into(),
            }),
        ]);
    }
    app.world_mut()
        .resource_mut::<Messages<AiEntityDisabled>>()
        .write(AiEntityDisabled {
            entity_uuid: "actor-uuid".into(),
            disabled: false,
        });
    app.update();
    assert!(!trigger_fired(&app, 0));
    assert!(!trigger_fired(&app, 1));
    app.world_mut()
        .resource_mut::<Messages<AiEntityDisabled>>()
        .write(AiEntityDisabled {
            entity_uuid: "actor-uuid".into(),
            disabled: true,
        });
    app.update();
    assert!(trigger_fired(&app, 0));
    assert!(trigger_fired(&app, 1));
    assert!(
        !trigger_fired(&app, 2),
        "defeat is never a fabricated destruction"
    );
    app.world_mut()
        .resource_mut::<Messages<AiEntityDisabled>>()
        .write(AiEntityDisabled {
            entity_uuid: "actor-uuid".into(),
            disabled: false,
        });
    app.update();
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert!(runtime.triggers[1].seen_disabled.is_empty());
    assert!(runtime.triggers[1].seen_destroyed.is_empty());
}
