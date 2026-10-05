//! One reliable feedback envelope, completed only by the gameplay owner.
use crate::core::messages::{
    ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommand, DeliveryClass, ServerMessage,
};
use crate::lobby::{OutboundMessage, Target};
use bevy::prelude::*;

pub(crate) fn action_feedback(
    token: &str,
    correlation: &ActionCorrelationId,
    outcome: ActionFeedbackOutcome,
) -> OutboundMessage {
    OutboundMessage {
        target: Target::Token(token.to_string()),
        msg: ServerMessage::ActionFeedback {
            correlation: correlation.clone(),
            outcome,
        },
        delivery: DeliveryClass::Reliable,
    }
}

pub(crate) fn admitted_action_feedback(
    command: &AdmittedCommand,
    outcome: ActionFeedbackOutcome,
) -> Option<OutboundMessage> {
    Some(action_feedback(
        command.response_token.as_deref()?,
        command.feedback_correlation.as_ref()?,
        outcome,
    ))
}

pub(crate) fn finish_action_feedback(
    command: &AdmittedCommand,
    outbound: &mut Option<ResMut<Messages<OutboundMessage>>>,
    outcome: ActionFeedbackOutcome,
) {
    if let (Some(message), Some(messages)) = (
        admitted_action_feedback(command, outcome),
        outbound.as_deref_mut(),
    ) {
        messages.write(message);
    }
}

pub(crate) fn finish_outbox_action_feedback(
    command: &AdmittedCommand,
    outbox: &mut crate::server_app::SimOutbox,
    outcome: ActionFeedbackOutcome,
) {
    if let Some(message) = admitted_action_feedback(command, outcome) {
        outbox.push_reliable((message.target, message.msg));
    }
}

pub(crate) fn write_action_feedback(
    outbound: &mut Option<ResMut<Messages<OutboundMessage>>>,
    token: &str,
    correlation: &ActionCorrelationId,
    outcome: ActionFeedbackOutcome,
) {
    if let Some(messages) = outbound.as_deref_mut() {
        messages.write(action_feedback(token, correlation, outcome));
    }
}

pub(crate) fn finish_admitted_action_feedback(
    outbound: &mut Option<ResMut<Messages<OutboundMessage>>>,
    command: &AdmittedCommand,
    outcome: ActionFeedbackOutcome,
) {
    finish_action_feedback(command, outbound, outcome);
}
