//! Recovering a failed pane as an ordinary participant disconnect (issue #1125).
//!
//! A local pane is "just another logical client" (the #1122 doctrine), so its
//! failure must ride the *same* disconnect → Backfill → reconnect machinery a
//! dropped phone does, not a native-only special case. This module is the small
//! amount of glue that routes a pane's fault into that path and, where the fault
//! warrants it, brings the pane back on the **same identity** — the in-process
//! analogue of the browser client's automatic redial (AGENTS.md "Getting on the
//! wire": "the same session token re-sent as `Identify`").
//!
//! Everything here is pure and Bevy-free: it operates on a [`PaneBus`], which is
//! the whole of how a pane talks to the simulation, so the fault-injection
//! acceptance criteria are checked by the ordinary `cargo test` CI runs rather
//! than only under the `ultralight` feature. The Ultralight half
//! ([`super::ultralight`]) does two things this module cannot: it *notices* a
//! real view crash, and it *builds the view* for a recreated pane. What a fault
//! then means — a disconnect, a Backfill, a reconnect on the same token — is all
//! here.
//!
//! # Why a crash recreates and an overflow does not
//!
//! Both faults close the pane, which is the whole of "the failed local
//! participant disconnects through the ordinary Session path" (issue #1125,
//! AC2): [`PaneBus::close`] owes the lobby one `PlayerDisconnected`, the station
//! keeps its holder and flips to `Backfill`, and — because
//! `SessionManager::holder_for_station` gates on `connected` — every audience
//! projection stops resolving to the failed token the instant it disconnects, so
//! no surviving or recreated pane inherits it (AC3).
//!
//! They differ in what comes *after* the close. A [`PaneFault::ViewCrashed`] is a
//! transient — the display is fine, the browser engine hiccuped — so the faithful
//! analogue of a network drop is to rebuild the view and let the page rejoin on
//! the same token, restoring its held station and current projection (AC4). A
//! [`PaneFault::ReliableOverflow`] is a page that stopped draining; recreating it
//! blindly would risk a rebuild loop against a page that wedges again, so that
//! one closes and stays closed — the operator decides. ([ai] decision:
//! auto-recreate is scoped to the crash case for exactly this reason.)

use super::registry::PaneId;
use super::transport::PaneBus;

/// Why a pane failed and its participant must disconnect.
///
/// A pane's fault is not a native-host error — it is a participant dropping, and
/// this names *which kind* so the recovery policy can tell a transient view
/// crash (rebuild it) from a wedged page (leave it closed). Both flip the
/// station to `Backfill`; that half is identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFault {
    /// The page stopped draining and every message queued for it is a reliable
    /// state transition, so nothing may be dropped to make room (issue #1122,
    /// the pre-existing trigger — see [`super::registry`]'s cap note).
    ReliableOverflow,
    /// The pane's Ultralight view stopped answering — a crash, a lost surface, a
    /// frame copy that will not complete. The display itself is fine, so this is
    /// a transient the view can be rebuilt from.
    ViewCrashed,
}

impl PaneFault {
    /// A one-line reason for the operator log.
    ///
    /// Operator diagnostics on the same footing as the rest of the pane host's
    /// log lines (inline English, not `strings.csv`): this is a file, a
    /// scrollback and a screenshot for whoever is running the bridge, never
    /// player-visible console text.
    pub fn reason(&self) -> &'static str {
        match self {
            PaneFault::ReliableOverflow => {
                "stopped draining its console and overflowed its reliable backlog"
            }
            PaneFault::ViewCrashed => "its console view stopped answering (crash or lost surface)",
        }
    }

    /// Whether a fault of this kind should recreate the pane, so the human
    /// reconnects on the same identity.
    ///
    /// True for a view crash (a transient — rebuild the view, the pane analogue
    /// of a network redial); false for a reliable overflow (a wedged page — leave
    /// it closed rather than risk a rebuild loop).
    pub fn recreates(&self) -> bool {
        matches!(self, PaneFault::ViewCrashed)
    }
}

/// What servicing one faulted pane did — for the operator log.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultOutcome {
    /// The pane that failed and was closed.
    pub failed: PaneId,
    /// Why it failed.
    pub fault: PaneFault,
    /// The recreated pane and the URL its view should navigate to, when the
    /// fault warranted a fresh view on the same identity. `None` for a fault that
    /// only closes, and `None` too when the bus was never armed for recreation
    /// (a test with no HTTP server — the new pane still opens, but with no URL).
    pub recreated: Option<(PaneId, String)>,
}

/// Close every pane the bus reported faulted, and recreate the ones whose fault
/// warrants it.
///
/// This is the seam-level whole of issue #1125's fault handling, drivable from a
/// test with no SDK, no GPU and no window — which is where its acceptance
/// criteria live. [`super::ultralight::drive_panes`] calls it once per frame
/// after it has *detected* faults (a page over its reliable budget, a view that
/// stopped answering); a CI test calls it after injecting one through
/// [`PaneBus::fault`].
///
/// Closing hands the lobby the ordinary `PlayerDisconnected`; recreating opens a
/// fresh pane on the same identity and enqueues its view for the Ultralight host
/// to build (see [`PaneBus::recreate`] / [`PaneBus::take_pending_views`]). The
/// close comes first so the disconnect is queued ahead of any reconnect
/// `Identify` the recreated pane will later send.
pub fn service_faults(bus: &PaneBus) -> Vec<FaultOutcome> {
    let mut outcomes = Vec::new();
    for (failed, fault) in bus.take_faulted() {
        // Close before recreate: `close` owes the lobby its disconnect now, and
        // the recreated pane's `Identify` cannot be polled until its view has
        // loaded — frames later — so the ordinary disconnect-then-reconnect
        // order is preserved. `recreate` reads the failed pane's identity from
        // its (now `Closed`) record, which the registry keeps.
        bus.close(failed);
        let recreated = if fault.recreates() {
            bus.recreate(failed)
        } else {
            None
        };
        outcomes.push(FaultOutcome {
            failed,
            fault,
            recreated,
        });
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
    use crate::lobby::handler::Target;
    use crate::native_host::panes::identity::PaneIdentity;
    use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

    fn identity(n: u8) -> PaneIdentity {
        PaneIdentity::adopt(
            format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
            format!("crew-{n}"),
        )
        .unwrap()
    }

    #[test]
    fn a_view_crash_closes_the_pane_and_recreates_it_on_the_same_identity() {
        // The crash case, at the seam. The failed pane is closed (its token owes
        // the lobby a disconnect) and a fresh pane opens carrying the SAME token,
        // which is what lets the page rejoin its held station on reconnect.
        let bus = PaneBus::default();
        let original = bus.open(identity(1));
        let token = bus.token_of(original).unwrap();
        bus.mark_live(original);

        bus.fault(original, PaneFault::ViewCrashed);
        let outcomes = service_faults(&bus);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].failed, original);
        assert_eq!(outcomes[0].fault, PaneFault::ViewCrashed);
        let (recreated, _url) = outcomes[0]
            .recreated
            .clone()
            .expect("a view crash recreates the pane");
        assert_ne!(recreated, original, "a recreated pane is a new handle");
        assert_eq!(
            bus.token_of(recreated).as_deref(),
            Some(token.as_str()),
            "and carries the same session token, so it reconnects as the same participant"
        );

        // The lobby is owed exactly one disconnect, on the shared token.
        assert_eq!(
            bus.transport().poll(),
            vec![TransportEvent::Disconnected { token }]
        );
    }

    #[test]
    fn an_overflow_closes_the_pane_and_does_not_recreate_it() {
        // The wedged-page case: closed, and left closed. Recreating a page that
        // stopped draining risks a rebuild loop; the operator decides.
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        bus.mark_live(id);

        bus.fault(id, PaneFault::ReliableOverflow);
        let outcomes = service_faults(&bus);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].fault, PaneFault::ReliableOverflow);
        assert!(
            outcomes[0].recreated.is_none(),
            "a reliable overflow closes but does not rebuild"
        );
        assert_eq!(bus.open_count(), 0, "nothing is open after the close");
    }

    #[test]
    fn a_recreated_pane_receives_its_own_projection_and_the_failed_one_never_does() {
        // AC3/AC4 at the seam: after a crash, a projection addressed to the
        // shared token reaches the recreated pane and no other, and the failed
        // (closed) pane receives nothing at all.
        let bus = PaneBus::default();
        let failed = bus.open(identity(1));
        let bystander = bus.open(identity(2));
        let token = bus.token_of(failed).unwrap();
        bus.mark_live(failed);
        bus.mark_live(bystander);

        bus.fault(failed, PaneFault::ViewCrashed);
        let (recreated, _) = service_faults(&bus)[0]
            .recreated
            .clone()
            .expect("a crash recreates");
        bus.mark_live(recreated);

        bus.transport().dispatch(TransportDispatch {
            target: &Target::Token(token),
            msg: &ServerMessage::GameStarted,
            delivery: DeliveryClass::Reliable,
        });

        assert_eq!(
            bus.take_outbound(recreated).len(),
            1,
            "the recreated pane receives its own token's projection"
        );
        assert!(
            bus.take_outbound(failed).is_empty(),
            "the failed pane is closed and inherits nothing"
        );
        assert!(
            bus.take_outbound(bystander).is_empty(),
            "and a neighbouring pane never sees another participant's projection"
        );
    }

    #[test]
    fn servicing_with_nothing_faulted_does_nothing() {
        let bus = PaneBus::default();
        let id = bus.open(identity(1));
        bus.mark_live(id);
        assert!(service_faults(&bus).is_empty());
        assert_eq!(bus.open_count(), 1);
    }

    #[test]
    fn a_recreated_pane_can_take_the_seat_back_by_re_identifying() {
        // The transport-visible half: the recreated pane presents the shared
        // token through the ordinary `Identify` path, exactly as a reconnecting
        // phone does. (The lobby's reconnect-yield that restores the held station
        // is proved end to end in `tests/native_host_panes.rs`.)
        let bus = PaneBus::default();
        let original = bus.open(identity(1));
        let token = bus.token_of(original).unwrap();
        bus.mark_live(original);
        bus.fault(original, PaneFault::ViewCrashed);
        let (recreated, _) = service_faults(&bus)[0]
            .recreated
            .clone()
            .expect("a crash recreates");

        // The disconnect is polled in its own frame, before the recreated view
        // has loaded — exactly as it is in production, where a fresh view takes
        // frames to load and identify. Draining it here is what puts the station
        // on Backfill before the reconnect that restores it.
        assert_eq!(
            bus.transport().poll(),
            vec![TransportEvent::Disconnected {
                token: token.clone()
            }],
            "the failed pane's disconnect comes first, on its own"
        );

        // Frames later: the recreated view has loaded and its page rejoins on the
        // shared token, through the ordinary `Identify` path.
        bus.mark_live(recreated);
        bus.submit(
            recreated,
            ClientMessage::Identify {
                token: token.clone(),
                name: "crew-1".to_string(),
            },
        )
        .expect("a recreated pane may identify as its own shared token");
        assert_eq!(
            bus.transport().poll(),
            vec![TransportEvent::Received {
                token: token.clone(),
                msg: ClientMessage::Identify {
                    token,
                    name: "crew-1".to_string(),
                },
            }],
            "then the reconnect Identify arrives on that same token"
        );
    }
}
