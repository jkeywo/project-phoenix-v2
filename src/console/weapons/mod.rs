//! The Weapons console (issue #1186): a pure module whose Bevy plugin, system
//! registration, and server-side adapter code live in the `server` sibling,
//! matching the pure-module + `server.rs` shape the other consoles follow. The
//! weapon-family submodules (`beam`, `blaster`, `torpedo`), the shared utilities
//! (`shared`), and the blackboard publishers (`blackboard`) stay declared here;
//! everything the `server` adapter exposes is re-exported so
//! `crate::console::weapons::X` paths keep resolving.

pub mod beam;
pub mod blackboard;
pub mod blaster;
pub mod server;
pub mod shared;
pub mod torpedo;

pub use server::*;

/// The owning consumer's factual terminal result.  This stays structured even
/// though the wire deliberately carries only Applied/Refused: it makes every
/// readiness branch account for a correlated action without inventing a new
/// gameplay or client-authority path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WeaponActionResult {
    Applied,
    Refused(WeaponActionRefusal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WeaponActionRefusal {
    UnknownMount,
    Offline,
    /// The power group this mount's `[[system]]` entry authors is at level 0 —
    /// switched off, not turned down (issue #1396).
    ///
    /// Kept apart from [`WeaponActionRefusal::Offline`] because they are
    /// different facts about the ship: Offline is a system the crew have LOST
    /// (damaged, destroyed, or nobody able to operate it), while cold is one
    /// they deliberately switched off at the reactor and can switch back on
    /// with one order. The wire carries only Applied/Refused, so the
    /// distinction costs no message vocabulary.
    PowerCold,
    ActiveOrCooling,
    MissingCombatLock,
    MissingTarget,
    OutOfArc,
    EmptyVolley,
    NotCharging,
    MissingTorpedoSystem,
    MagazineOffline,
    ConservationHold,
    TubeNotLoaded,
    NoTorpedoes,
}

/// Complete a correlated Tactical action only at its owning consumer. The
/// correlation is presentation metadata; command admission and simulation
/// semantics remain unchanged.
pub(crate) fn finish_action_feedback(
    cmd: &crate::core::messages::AdmittedCommand,
    outbound: &mut Option<
        bevy::prelude::ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
    result: WeaponActionResult,
) {
    let (Some(correlation), Some(token), Some(messages)) = (
        cmd.feedback_correlation.as_ref(),
        cmd.response_token.as_ref(),
        outbound.as_deref_mut(),
    ) else {
        return;
    };
    messages.write(crate::lobby::server::OutboundMessage {
        target: crate::lobby::Target::Token(token.clone()),
        msg: crate::core::messages::ServerMessage::ActionFeedback {
            correlation: correlation.clone(),
            outcome: match result {
                WeaponActionResult::Applied => {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                }
                WeaponActionResult::Refused(_) => {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                }
            },
        },
        delivery: crate::core::messages::DeliveryClass::Reliable,
    });
}
