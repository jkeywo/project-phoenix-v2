use super::*;
use crate::ship::control_source::ControlSource;

#[test]
fn simplified_priority_drives_the_sweep_but_cannot_recall_the_ai_team() {
    let mut app = test_app();
    start_game(&mut app);
    team_on_site_at_helm_with_two_jobs_left(&mut app);
    let mut query = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut sources in query.iter_mut(app.world_mut()) {
        sources
            .0
            .set(SystemId("repair".into()), ControlSource::Simplified);
    }
    for payload in [
        SystemControlPayload::RecallRepairTeam { team_idx: 0 },
        SystemControlPayload::SetRepairTargetPriority {
            system_id: SystemId("helm-engine-starboard".into()),
        },
    ] {
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload,
            },
        );
    }
    for _ in 0..600 {
        tick(&mut app);
        let teams = local_teams(&mut app);
        assert!(
            !team_is_returning(&teams, 0),
            "summary input must not recall an AI-operated team"
        );
        if let TeamSlot::Repairing {
            system_id: Some(id),
            ..
        } = &teams.0.slots()[0]
        {
            if id.0 != "helm-thrust" {
                assert_eq!(
                    id.0, "helm-engine-starboard",
                    "the AI sweep must consume the holder's priority intent"
                );
                return;
            }
        }
    }
    panic!("the simplified repair team never consumed its summary intent");
}
