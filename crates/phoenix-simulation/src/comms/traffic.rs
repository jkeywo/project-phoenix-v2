//! The actual Radio's finite coverage and contextual-send continuation.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use bevy::prelude::*;
use phoenix_model::{
    comms_channel::CommsRadioConfig,
    observation::{
        ObservationMetadata, ObservationProvenance, ObservationSource, ObservationSubject,
        ObservationUncertainty,
    },
};
use phoenix_sim_gameplay::electronic::traffic::AlertSample;
pub use phoenix_sim_gameplay::electronic::traffic::CommsTrafficState;

#[derive(Component, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShipCommsTraffic {
    pub state: CommsTrafficState,
    /// Derived current payment, cleared on recovery; not another resource owner.
    #[serde(skip)]
    pub(crate) coverage: Option<(u64, Vec<String>)>,
}

pub(super) fn refuse_unsupported(
    owners: &Query<(
        &crate::core::messages::AdmittedCommands,
        Option<&super::radio::RadioProfile>,
        Has<super::capture::ShipCommsCapture>,
        Has<ShipCommsTraffic>,
    )>,
    mut outbox: Option<&mut crate::server_app::SimOutbox>,
) {
    let Some(outbox) = outbox.as_deref_mut() else {
        return;
    };
    for (commands, radio, _, has_traffic) in owners {
        for command in &commands.0 {
            use crate::core::messages::SystemControlPayload as Control;
            if matches!(
                command.payload,
                Control::SetCommsListeningOrder { .. } | Control::ReportCommsAlert
            ) && !(has_traffic
                && radio.is_some_and(|radio| {
                    radio.0.system == command.target && radio.0.traffic.is_some()
                }))
            {
                crate::command_admission::feedback::finish_outbox_action_feedback(
                    command,
                    outbox,
                    crate::core::messages::ActionFeedbackOutcome::Refused,
                );
            }
        }
    }
}

pub(super) fn apply_order(
    commands: &crate::core::messages::AdmittedCommands,
    radio: &CommsRadioConfig,
    mut state: Option<&mut ShipCommsTraffic>,
    mut outbox: Option<&mut crate::server_app::SimOutbox>,
) {
    let (Some(profile), Some(state)) = (&radio.traffic, state.as_deref_mut()) else {
        return;
    };
    for command in commands.for_target(&radio.system.0) {
        let crate::core::messages::SystemControlPayload::SetCommsListeningOrder { priority } =
            &command.payload
        else {
            continue;
        };
        let applied = state.state.owner == radio.system
            && state
                .state
                .set_order(profile, &channels(radio), priority.clone())
                .is_ok();
        if let Some(outbox) = outbox.as_deref_mut() {
            crate::command_admission::feedback::finish_outbox_action_feedback(
                command,
                outbox,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

pub(crate) fn unique_system(
    config: &crate::ship::config::ShipConfig,
    kind: &str,
) -> Option<SystemId> {
    let mut matching = config.systems.iter().filter(|system| system.kind == kind);
    let owner = matching.next()?.id.clone();
    matching.next().is_none().then_some(owner)
}

pub(crate) fn alert_observation(now: u64, owner: &SystemId) -> ObservationMetadata {
    ObservationMetadata::at(
        now,
        ObservationSubject::System { id: owner.clone() },
        ObservationProvenance {
            kind: ObservationSource::SystemReport,
            system: Some(owner.clone()),
            label: String::new(),
        },
        ObservationUncertainty::Unspecified,
    )
}

pub(crate) fn channels(radio: &CommsRadioConfig) -> Vec<String> {
    radio
        .channels
        .iter()
        .map(|channel| channel.id.clone())
        .collect()
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    let Some(radio) = config.comms.as_ref().and_then(|comms| comms.radio.as_ref()) else {
        return;
    };
    let Some(profile) = &radio.traffic else {
        return;
    };
    commands.insert(ShipCommsTraffic {
        state: CommsTrafficState::new(radio.system.clone(), profile, &channels(radio))
            .expect("accepted Radio traffic profile"),
        coverage: None,
    });
}

/// No private ShipRedAlert fallback: only the exact authored typed Console feed.
pub(crate) fn sample(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    radio: &CommsRadioConfig,
    now: u64,
) -> Option<AlertSample> {
    let source = &radio.traffic.as_ref()?.alert.as_ref()?.source;
    let console = config.0.console_for_system(&radio.system)?;
    if !console.feeds.iter().any(|feed| {
        feed.source == *source
            && feed.kind == phoenix_model::console::ConsoleObservationKind::Captain
    }) {
        return None;
    }
    if config.0.system(source)?.kind != crate::ship::system_registry::CAPTAIN_KIND {
        return None;
    }
    if unique_system(&config.0, crate::ship::system_registry::CAPTAIN_KIND).as_ref() != Some(source)
    {
        return None;
    }
    let owner = unique_system(&config.0, crate::ship::system_registry::RED_ALERT_KIND)?;
    let permitted = config.0.observations_for_system(&radio.system, &boards.0);
    let SystemBlackboard::Captain(board) = permitted.get(source)? else {
        return None;
    };
    (board.red_alert_system_id == owner).then_some(())?;
    AlertSample::from_publication(
        board.red_alert,
        &owner,
        board.alert_observation.as_ref()?,
        now,
    )
}

pub(crate) fn view(
    state: &ShipCommsTraffic,
    radio: &CommsRadioConfig,
    now: u64,
) -> Option<phoenix_model::comms_traffic::CommsTrafficView> {
    let profile = radio.traffic.as_ref()?;
    let channels = channels(radio);
    if state.state.owner != radio.system || state.state.validate(profile, &channels, now).is_err() {
        return None;
    }
    let current = state
        .coverage
        .as_ref()
        .filter(|(tick, _)| *tick <= now && now - *tick <= 1);
    Some(phoenix_model::comms_traffic::CommsTrafficView {
        simultaneous_channels: profile
            .listening
            .as_ref()
            .map_or(channels.len() as u16, |profile| {
                profile.simultaneous_channels
            }),
        channels,
        priority: state.state.priority.clone(),
        monitored: current.map_or_else(Vec::new, |(_, channels)| channels.clone()),
        observed_tick: current.map(|(tick, _)| *tick),
        report_supported: profile.alert.is_some(),
        last_report: state.state.last_report,
        controllable: false,
    })
}

/// Called by the existing response host, after actual reading and released work
/// have priority. Default Backfill has no branch that resets operator order.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decision(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    radio: &super::radio::RadioProfile,
    policy: Option<&crate::ai::policy::AiPolicy>,
    flags: &[&crate::world::flags::FlagStore],
    available: bool,
    state: Option<&ShipCommsTraffic>,
    now: u64,
) -> bool {
    let Some(policy) = policy.filter(|policy| !policy.idle) else {
        return false;
    };
    let owner = &radio.0.system;
    if !available
        || radio
            .0
            .traffic
            .as_ref()
            .and_then(|traffic| traffic.alert.as_ref())
            .is_none()
        || !crate::ai::host::ai_operates(&sources.0, owner)
    {
        return false;
    }
    let permitted = config.0.observations_for_system(owner, &boards.0);
    let Some(SystemBlackboard::Comms(board)) = permitted.get(owner) else {
        return false;
    };
    if !board.planned_requests.is_empty()
        || board.messages.iter().any(|message| {
            message.selected_response.is_none()
                && message.responses.iter().any(|response| response.available)
        })
        || board
            .captured
            .as_ref()
            .is_some_and(|captured| captured.rows.iter().any(|row| row.selected))
    {
        return false;
    }
    let Some(state) = state.filter(|state| state.state.owner == *owner) else {
        return false;
    };
    let Some(profile) = &radio.0.traffic else {
        return false;
    };
    let mut trial = state.state.clone();
    if trial
        .prepare_report(
            profile,
            &channels(&radio.0),
            sample(config, boards, &radio.0, now),
            now,
        )
        .ok()
        .flatten()
        .is_none()
    {
        return false;
    }
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(
        phoenix_sim_gameplay::entities::ai_flag_hosts::COMMS_AVAILABLE,
        1.0,
    );
    matches!(
        crate::ai::host::decide(
            &sources.0,
            Some(policy),
            &crate::ai::host::HostTick {
                system: owner.clone(),
                channel: crate::entities::config::COMMS_REPORT_CHANNEL,
                facts: &facts,
                flags,
                state: None,
            }
        ),
        crate::ai::host::HostOutcome::Act(crate::ai::policy::AiPolicyVerb::ReportCommsAlert)
    )
}

#[cfg(test)]
#[path = "traffic_tests.rs"]
mod tests;
