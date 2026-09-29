use crate::core::messages::{StationId, SystemControlPayload, SystemId};
use crate::ship::components::ShipSystemControlSources;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};

#[test]
fn simplified_summary_admission_keeps_tenure_availability_and_actuator_exclusivity() {
    let config = crate::ship::config::parse_and_validate(
        r#"
[[station]]
id = "engineer"
name = "Engineer"
description = "Repair"
rank = "Lt"
[[station.rating]]
name = "Summary"
automated_systems = []
detailed_systems = []
[[system]]
id = "repair"
kind = "repair"
station = "engineer"
"#,
        &["repair"],
    )
    .unwrap();
    let mut manager = crate::lobby::session::SessionManager::new();
    manager.register("owner".into(), "Engineer".into()).unwrap();
    manager.set_station("owner", Some(StationId("engineer".into())));
    manager.register("stranger".into(), "Other".into()).unwrap();
    let mut sessions = crate::lobby::Sessions(manager);
    let id = SystemId("repair".into());
    let mut sources = ShipSystemControlSources(ControlSourceResolver::new());
    crate::ship::rating::apply_rating(
        &config,
        &StationId("engineer".into()),
        "Summary",
        &mut sources.0,
    );
    assert_eq!(sources.0.source_for(&id), ControlSource::Simplified);
    let policy = sources.0.policy_for(&id);
    assert!(!policy.accept_human_input && policy.accept_summary_input && policy.operate_ai);
    assert_eq!(
        crate::ship::coordination::seat_control_source(&[policy]),
        ControlSource::Human
    );
    let summary = SystemControlPayload::SetRepairPriority {
        team_idx: 0,
        priority: 1,
    };
    let direct = SystemControlPayload::RecallRepairTeam { team_idx: 0 };
    let admits =
        |token, payload, sources: &ShipSystemControlSources, sessions: &crate::lobby::Sessions| {
            crate::command_admission::policy::is_command_authorized(
                token, &id, payload, sources, sessions, &config, None,
            )
        };
    assert!(admits("owner", &summary, &sources, &sessions));
    assert!(!admits("owner", &direct, &sources, &sessions));
    assert!(admits("ai:repair", &direct, &sources, &sessions));
    assert!(!admits("stranger", &summary, &sources, &sessions));
    sessions.0.set_afk("owner", true);
    assert!(!admits("owner", &summary, &sources, &sessions));
    sessions.0.set_afk("owner", false);
    sources.0.set_gm_disabled(id.clone(), true);
    assert!(!admits("owner", &summary, &sources, &sessions));
    sources.0.set_gm_disabled(id.clone(), false);
    sources.0.set_offline(id.clone(), true);
    assert!(!admits("owner", &summary, &sources, &sessions));
    sources.0.set_offline(id.clone(), false);
    sources.0.set(id.clone(), ControlSource::Ai);
    assert!(!admits("owner", &summary, &sources, &sessions));
}
