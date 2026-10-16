//! The Weapons console (issue #1186): a pure module whose Bevy plugin, system
//! registration, and server-side adapter code live in the `server` sibling,
//! matching the pure-module + `server.rs` shape the other consoles follow. The
//! weapon-family submodules (`beam`, `blaster`, `torpedo`), the shared utilities
//! (`shared`), and the blackboard publishers (`blackboard`) stay declared here;
//! everything the `server` adapter exposes is re-exported so
//! `crate::console::weapons::X` paths keep resolving.

pub mod assignments;
pub mod beam;
pub mod blackboard;
pub mod blaster;
pub mod flight_carrier;
pub mod guidance_link;
pub(crate) mod guidance_ui;
pub(crate) mod guided_advance;
pub mod illumination;
pub(crate) mod pdc;
mod round_work;
mod shared_loaders;
pub(crate) use guided_advance::advance_torpedo_flights;
pub mod server;
pub mod shared;
pub mod strike;
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
    /// Explicit Tactical hold-fire intent, independent of local intake.
    Restrained,
    /// A complete local energy cost was unavailable; intent and ammo survive.
    EnergyUnavailable,
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
    crate::command_admission::finish_action_feedback(
        cmd,
        outbound,
        match result {
            WeaponActionResult::Applied => crate::core::messages::ActionFeedbackOutcome::Applied,
            WeaponActionResult::Refused(_) => crate::core::messages::ActionFeedbackOutcome::Refused,
        },
    );
}

#[cfg(test)]
#[path = "seeker_tests.rs"]
mod seeker_tests;
