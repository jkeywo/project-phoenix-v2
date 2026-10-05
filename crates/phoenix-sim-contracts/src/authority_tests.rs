use super::*;
use crate::control::{control_tick_policy, ControlSource};

#[test]
fn summary_eligibility_does_not_grant_control_permission() {
    for source in [
        ControlSource::Human,
        ControlSource::Ai,
        ControlSource::Simplified,
        ControlSource::Offline,
    ] {
        for summary in [false, true] {
            let control = control_tick_policy(source);
            let target = CommandTargetPolicy {
                control,
                summary,
                debug_route: false,
            };
            let allowed = control.accept_human_input || (summary && control.accept_summary_input);
            assert_eq!(
                authorize_command(CommandClaimant::LocalConsole, target),
                if allowed {
                    CommandAuthorization::Allowed
                } else {
                    CommandAuthorization::Denied
                }
            );
            assert_eq!(
                authorize_command(
                    CommandClaimant::Crew {
                        spectator: false,
                        afk: false,
                        registered: true,
                        holds_station: Some(true)
                    },
                    target
                ),
                if allowed {
                    CommandAuthorization::Allowed
                } else {
                    CommandAuthorization::Denied
                }
            );
            assert_eq!(
                authorize_command(CommandClaimant::Ai, target),
                if control.operate_ai {
                    CommandAuthorization::Allowed
                } else {
                    CommandAuthorization::Denied
                }
            );
        }
    }
    let target = CommandTargetPolicy {
        control: control_tick_policy(ControlSource::Simplified),
        summary: true,
        debug_route: true,
    };
    for claimant in [
        CommandClaimant::Crew {
            spectator: true,
            afk: false,
            registered: true,
            holds_station: Some(true),
        },
        CommandClaimant::Crew {
            spectator: false,
            afk: true,
            registered: true,
            holds_station: Some(true),
        },
    ] {
        assert_eq!(
            authorize_command(claimant, target),
            CommandAuthorization::Denied
        );
    }
}
