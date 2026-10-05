/// Session token used for actions originating from the local HTML consoles
/// (browser server viewscreen / native wry server), where the operator drives
/// a console directly rather than through a remote network session. The host
/// page routes its console actions through `gui/action-map.js` into full
/// `ClientMessage` JSON and submits them via `wasm_receive_message` under this
/// token (issue #822); the gameplay console handlers treat it as an authorized
/// local operator (see the weapons fire guards). Defined here — ungated — so
/// both the wasm bridge and the (non-wasm) gameplay handlers can reference it.
pub const LOCAL_CONSOLE_TOKEN: &str = "__local_console__";

/// Rating name that automates every system owned by the station.
pub const BACKFILL_RATING: &str = "Backfill";

/// Authentication/tenure facts supplied by the host at the command boundary.
/// AI decisions carry no crew Session dependency.
#[derive(Clone, Copy, Debug)]
pub enum CommandClaimant {
    Ai,
    LocalConsole,
    Crew {
        spectator: bool,
        afk: bool,
        registered: bool,
        /// None means the resolved System has no owning Station.
        holds_station: Option<bool>,
    },
}

/// Gameplay resolves the effective target and summary-intent eligibility;
/// the parent supplies the build-gated debug-route fact.
#[derive(Clone, Copy, Debug)]
pub struct CommandTargetPolicy {
    pub control: crate::control::ControlTickPolicy,
    /// Payload eligibility only; control permission is enforced here.
    pub summary: bool,
    pub debug_route: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAuthorization {
    Allowed,
    Denied,
    UnknownSystem,
}

/// One policy for network input and immediate in-process AI commands.
/// Branch order is part of the Admission contract.
pub fn authorize_command(
    claimant: CommandClaimant,
    target: CommandTargetPolicy,
) -> CommandAuthorization {
    use CommandAuthorization::{Allowed, Denied, UnknownSystem};
    let summary = target.summary && target.control.accept_summary_input;
    let accept = |allowed| if allowed { Allowed } else { Denied };
    let (spectator, afk, registered, holds_station) = match claimant {
        CommandClaimant::Ai => return accept(target.control.operate_ai),
        CommandClaimant::LocalConsole => {
            return accept(target.control.accept_human_input || summary)
        }
        CommandClaimant::Crew {
            spectator,
            afk,
            registered,
            holds_station,
        } => (spectator, afk, registered, holds_station),
    };
    if spectator || (summary && afk) {
        return Denied;
    }
    if !target.control.accept_human_input && !summary {
        return Denied;
    }
    if target.debug_route && registered {
        return Allowed;
    }
    match holds_station {
        Some(held) => accept(held),
        None => UnknownSystem,
    }
}

#[cfg(test)]
#[path = "authority_tests.rs"]
mod tests;
