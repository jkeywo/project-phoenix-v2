//! Admitted local postings; existing Repair jobs remain the movement/work owner.
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload, SystemControlPayloadDiscriminants},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::personnel::{CompiledPersonnel, PersonnelState};

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
    app.declare_state::<CompiledPersonnel>(StateClass::Derived, "accepted-personnel-roster")
        .declare_state::<PersonnelState>(StateClass::Folded, "officer-physical-custody");
    app.register_admitted_consumer(
        ConsumerMatcher::kind_payload(
            crate::ship::system_registry::REPAIR_KIND,
            SystemControlPayloadDiscriminants::SetDutyOfficerPosting,
        )
        .with_feedback(
            crate::command_admission::FeedbackAddress::DeclaredKindOnly,
            &[SystemControlPayloadDiscriminants::SetDutyOfficerPosting],
        ),
    );
    app.add_systems(
        FixedUpdate,
        apply
            .in_set(crate::sim_sets::SimSet::Physics)
            .in_set(crate::sim_sets::FixedStep::HandleDutyOfficerPosting),
    );
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;

pub(crate) fn apply(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut ships: Query<
        (
            &AdmittedCommands,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            Option<&super::server::ShipRepairTeams>,
            Option<&CompiledPersonnel>,
            Option<&mut PersonnelState>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    for (admitted, config, sources, teams, profile, mut personnel) in &mut ships {
        for command in &admitted.0 {
            let SystemControlPayload::SetDutyOfficerPosting { officer, team_idx } =
                &command.payload
            else {
                continue;
            };
            if config
                .0
                .system(&command.target)
                .is_none_or(|system| system.kind != crate::ship::system_registry::REPAIR_KIND)
            {
                continue;
            }
            let result = (|| {
                let teams = teams.ok_or("missing actual Repair teams")?;
                if &command.target != teams.0.owner()
                    || !sources.0.entries().any(|(id, _)| id == &command.target)
                    || sources.0.is_offline(&command.target)
                    || sources.0.source_for(&command.target)
                        == crate::ship::control_source::ControlSource::Offline
                {
                    return Err("actual Repair posting owner is unavailable");
                }
                let profile = profile.ok_or("no accepted officer roster")?;
                let state = personnel
                    .as_deref_mut()
                    .ok_or("missing actual officer custody")?;
                let layout = teams.0.layout().ok_or("missing actual team navigation")?;
                let tick = tick
                    .as_deref()
                    .ok_or("missing actual logical posting clock")?;
                state
                    .post(
                        profile,
                        teams.0.job_state(),
                        layout,
                        officer,
                        *team_idx,
                        tick.0,
                    )
                    .map_err(|_| "posting is not physically available")
            })();
            crate::command_admission::finish_action_feedback(
                command,
                &mut outbound,
                if result.is_ok() {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}
