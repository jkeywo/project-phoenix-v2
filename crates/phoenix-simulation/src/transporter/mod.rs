//! The rescue transporter (issue #1348, PRD #1337) — Engineering recovers the
//! civilians a scan revealed aboard a derelict contact.
//!
//! The transporter is a first-class engineering-owned `[[system]]`, the shape
//! the tractor (#1156) established and the dock/umbilical/security slices copy:
//! it declares a power group, carries a damage entry, is admission-gated, and
//! publishes its own blackboard. Unlike the tractor it names its OWN discovered
//! contact rather than coupling to the combat lock, and it runs over many ticks
//! at an authored rate rather than a per-tick geometry.
//!
//! Split the same way `tractor` is (rule 10):
//!
//! * [`coupling`] — the pure, Bevy-free half: the authored `[transporter]`
//!   config and its validation, the refusal vocabulary, and the transport
//!   verdict. Unit-tested in isolation.
//! * [`server`] — the Bevy adapter: the per-ship [`server::Transporter`]
//!   component, the per-contact [`server::CivilianRescue`] component, and the
//!   fixed-tick systems that take the commands, reveal a scanned contact, decide
//!   the transport, advance the recovery, run the backfill, record a casualty,
//!   and publish the blackboard.

/// The pure, Bevy-free config, refusal vocabulary and transport verdict.
pub mod coupling;
/// The Bevy adapter: the components, the fixed-tick systems and the blackboard
/// publisher.
pub mod server;

pub use coupling::{
    transport_status, CivilianRescueConfig, TransportInputs, TransportRefusal, TransporterConfig,
};
pub use server::{
    handle_transporter_commands, operate_transporter_ai, publish_transporter_blackboard,
    record_civilian_casualties, rescue_lost_flag, rescue_recovered_flag, reveal_civilian_contacts,
    tick_transport, transporter_blackboard_key, CivilianRescue, CivilianRescueLedger, Transporter,
    TransporterAiEngaged, TransporterPlugin,
};
