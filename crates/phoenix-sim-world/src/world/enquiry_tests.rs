use super::*;
use crate::world::config::{scripted_trigger, EnquiryPhase};
use phoenix_model::identity::SystemId;

#[test]
fn enquiry_filters_subject_observer_instrument_intent_and_phase_without_aliasing_attacks() {
    let names = HashMap::from([
        ("depot".into(), "target".into()),
        ("scout".into(), "observer".into()),
    ]);
    let condition = TriggerCondition::OnEnquiry {
        entity_name: "depot".into(),
        observer_name: Some("scout".into()),
        system: Some(SystemId("science-array".into())),
        aim: Some("resources".into()),
        intrusive: Some(true),
        phase: EnquiryPhase::Started,
    };
    let event = WorldEvent::Enquiry {
        target_uuid: "target".into(),
        observer_uuid: "observer".into(),
        system: SystemId("science-array".into()),
        aim: "resources".into(),
        intrusive: true,
        phase: EnquiryPhase::Started,
    };
    assert!(condition_matches(&condition, &event, &names, &[]));
    let edits: [fn(&mut WorldEvent); 6] = [
        |event| {
            if let WorldEvent::Enquiry { target_uuid, .. } = event {
                *target_uuid = "other".into();
            }
        },
        |event| {
            if let WorldEvent::Enquiry { observer_uuid, .. } = event {
                *observer_uuid = "other".into();
            }
        },
        |event| {
            if let WorldEvent::Enquiry { system, .. } = event {
                *system = SystemId("other-array".into());
            }
        },
        |event| {
            if let WorldEvent::Enquiry { aim, .. } = event {
                *aim = "general".into();
            }
        },
        |event| {
            if let WorldEvent::Enquiry { intrusive, .. } = event {
                *intrusive = false;
            }
        },
        |event| {
            if let WorldEvent::Enquiry { phase, .. } = event {
                *phase = EnquiryPhase::Completed;
            }
        },
    ];
    for edit in edits {
        let mut changed = event.clone();
        edit(&mut changed);
        assert!(!condition_matches(&condition, &changed, &names, &[]));
    }
    assert!(!condition_matches(
        &condition,
        &WorldEvent::Attacked {
            uuid: "target".into(),
            attacker_uuid: "observer".into()
        },
        &names,
        &[]
    ));
    let mut state = vec![TriggerState {
        trigger: scripted_trigger(condition),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        seen_disabled: HashSet::new(),
        last_fired_elapsed: None,
    }];
    assert_eq!(
        evaluate_triggers(&mut state, &[event.clone()], &names).len(),
        1
    );
    assert!(evaluate_triggers(&mut state, &[event], &names).is_empty());
}
