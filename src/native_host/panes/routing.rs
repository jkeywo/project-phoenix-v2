//! Which pane a dispatched message is for (issue #1122).
//!
//! This is the audience-projection boundary, and it is four lines long, because
//! by the time a message reaches a transport the projection has already been
//! decided somewhere far more careful.
//!
//! `core::broadcast::audience::Audience` — `All`, `Holding(StationId)`,
//! `HoldingSystem(SystemId)`, `HoldingWeapons`, `Token`, `AllExcept` — resolves
//! **every** variant through `SessionManager::holder_for_station` into a
//! `lobby::handler::Target` before an `OutboundMessage` exists at all. So a
//! console projection addressed to whoever is sitting at Helm arrives at this
//! module already flattened to `Target::Token(<the helm holder's token>)`.
//!
//! What is left for a transport to do is therefore exactly what a PeerJS host
//! does with the same `Target`: hand it to the connection with that token, and
//! to no other. A pane holding no station is named by no `Holding*` audience, so
//! it receives nothing addressed to one — not because this module filters it
//! out, but because the resolver never wrote its token down.
//!
//! **The failure this prevents is a real one.** An in-process transport that
//! "helpfully" broadcast every dispatch to every pane, on the reasoning that
//! they are all in one process anyway, would hand every station's private
//! projection to every pane — weapons solutions to the comms officer, the
//! captain's dossier to helm — and would do it silently, because the pages
//! would simply render what they were given. That is the concrete meaning of
//! "cannot bypass audience projection", and
//! [`the_projection_boundary_is_the_target`](fn@pane_receives) is where it is
//! decided.

use crate::lobby::handler::Target;

/// Whether the pane holding `pane_token` is an addressee of `target`.
///
/// The one place a pane's inbox is decided. Every caller in this module tree
/// goes through it, so there is a single site to read, test, and be sure of.
pub fn pane_receives(target: &Target, pane_token: &str) -> bool {
    match target {
        Target::All => true,
        Target::Token(token) => token == pane_token,
        Target::AllExcept(token) => token != pane_token,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000001";
    const GRACE: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000002";

    #[test]
    fn a_broadcast_reaches_every_pane() {
        assert!(pane_receives(&Target::All, ADA));
        assert!(pane_receives(&Target::All, GRACE));
    }

    #[test]
    fn a_targeted_projection_reaches_only_the_token_it_names() {
        // The whole projection boundary, stated once. A `Holding(Helm)`
        // audience has already become `Token(<helm's holder>)` by the time it
        // gets here, so this line is what stops the comms pane reading the
        // weapons pane's console state.
        let helm = Target::Token(ADA.to_string());
        assert!(pane_receives(&helm, ADA));
        assert!(!pane_receives(&helm, GRACE));
    }

    #[test]
    fn an_all_except_projection_skips_exactly_one_pane() {
        let except = Target::AllExcept(ADA.to_string());
        assert!(!pane_receives(&except, ADA));
        assert!(pane_receives(&except, GRACE));
    }

    #[test]
    fn a_token_that_merely_shares_a_prefix_is_not_the_same_participant() {
        // Session tokens are compared whole, never by prefix — a
        // `starts_with` here would leak between two participants whose tokens
        // happened to share a leading run of hex.
        let helm = Target::Token(ADA.to_string());
        assert!(!pane_receives(&helm, &format!("{ADA}x")));
        assert!(!pane_receives(&helm, &ADA[..8]));
    }
}
