//! Dossiers (issue #1030, parent #851 "Falling Skyway").
//!
//! What this crew knows about a ship, a station or a structure — folded fresh
//! every tick from state other subsystems already own, and structurally unable
//! to carry what the scenario is keeping back. [`projection`] is the pure fold
//! and the place the known/hidden rule is written down; [`server`] is the thin
//! adapter that gathers the live inputs and publishes the result on the local
//! ship's `dossiers` blackboard channel.

/// The pure fold: the input port, the fact vocabulary, and the rule that keeps
/// hidden truth out by construction.
pub mod projection;
/// The Bevy adapter: the derived subject roster and the blackboard publisher.
pub mod server;

pub use projection::{
    project, DossierSubject, SubjectCondition, FACT_COMMITMENT_BROKEN, FACT_COMMITMENT_KEPT,
    FACT_COMMITMENT_OPEN, FACT_COMMS, FACT_CONDITION, FACT_FACTION, SHARED_FACT_LABELS,
};
pub use server::{
    dossier_blackboard_key, publish_dossier_blackboard, DossierPlugin, DOSSIER_BLACKBOARD_KEY,
};
